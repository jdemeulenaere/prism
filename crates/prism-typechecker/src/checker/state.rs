//! State handles in the type checker (D3, D5 and D7 of `state-handles`).
//!
//! `s.value` of a `State<T>` or `MutableState<T>` is typed by its own rule rather than as a
//! computed property:
//!
//! - for a `Copy` `T` (by its declared marker, a type parameter only through a `Copy` bound),
//!   a read is a call of [`BuiltinFunction::StateValue`] with the handle copied as its
//!   receiver, so borrowing it borrows a temporary;
//! - for any other `T`, it is an [`ExpressionKind::StatePlace`] over the handle, which only a
//!   composable context may read in place, borrow and project; outside one it is `E4033`;
//! - as the target of an assignment it is a state place of either kind, written through a
//!   runtime call, never as a place rooted at the handle.
//!
//! A mutable borrow, a `mut fun` receiver or a part write of any `s.value` is `E4031` in a
//! composable context and `E4033` elsewhere, with help naming `edit`. Writes, `update` and
//! `edit` in a composable context are `E4031`, and `mutableStateOf` there is `E4032`. Each
//! rule costs a member lookup, a marker lookup and a test of the composable-context flag.

use super::TypeChecker;
use crate::builder::PendingStateRead;
use crate::error::TypeError;
use crate::types::Type;
use prism_ast::expr::{CallArg, Expr};
use prism_ast::Span;
use prism_typed_tree::{
    Argument, BuiltinFunction, Call, Callee, ExpressionId, ExpressionKind, IntrinsicStruct,
};

/// The member name of a state's value.
pub(super) const VALUE: &str = "value";

/// The handle kind and value type of `ty` when it is a state handle, `State<T>` or
/// `MutableState<T>`.
pub(super) fn state_handle(ty: &Type) -> Option<(IntrinsicStruct, &Type)> {
    match ty {
        Type::Struct { id, arguments } => match (id.intrinsic(), &arguments[..]) {
            (Some(handle), [value]) if handle.is_state_handle() => Some((handle, value)),
            _ => None,
        },
        _ => None,
    }
}

/// `State<T>` for `MutableState<T>`: the type a lookup that misses on a mutable handle is
/// retried on (D3 of `state-handles`).
pub(super) fn read_only_handle(ty: &Type) -> Option<Type> {
    match state_handle(ty) {
        Some((IntrinsicStruct::MutableState, value)) => Some(Type::Struct {
            id: IntrinsicStruct::State.id(),
            arguments: vec![value.clone()],
        }),
        _ => None,
    }
}

impl TypeChecker {
    /// The type of `value` on the handle `peeled` and whether it may be assigned, when
    /// `member` is `value` and the handle's module is visible: the value type, writable on a
    /// `MutableState<T>`.
    pub(super) fn state_value_member(&self, peeled: &Type, member: &str) -> Option<(Type, bool)> {
        if member != VALUE {
            return None;
        }
        let (_, value) = state_handle(peeled)?;
        let found = self.member(peeled, member).filter(|m| m.property)?;
        Some((value.clone(), found.writable))
    }

    /// The node of `s.value` read on the handle node `base` (D5): a call of the built-in
    /// `value` for a `Copy` value type, else a state place, which outside composable contexts
    /// waits for the end of the body to be reported unless a mutable use is reported first.
    pub(super) fn state_value_node(
        &mut self,
        base: ExpressionId,
        value: Type,
        span: Span,
    ) -> ExpressionId {
        let handle_ty = self.ty(base);
        let handle_text = self.describe_handle(base);
        let handle = self.consume(base);
        if self.is_copy(&value) {
            let call = Call {
                callee: Callee::Builtin(BuiltinFunction::StateValue),
                arguments: vec![Argument::Written(handle)],
                parameters: vec![(Some(super::THIS.to_string()), handle_ty)],
                parameter_kinds: vec![prism_typed_tree::ParameterKind::Plain],
                type_arguments: vec![value.clone()],
                evaluation_order: vec![0],
                has_receiver: true,
                has_context: false,
                is_compose: false,
            };
            return self.add(ExpressionKind::Call(call), value, span);
        }
        let node = self.add(ExpressionKind::StatePlace { handle }, value, span);
        let in_compose = self.is_in_compose();
        self.builder.state_value_spans.insert(span, in_compose);
        if !in_compose {
            self.builder.pending_state_reads.push(PendingStateRead {
                node,
                span,
                handle: handle_text,
            });
        }
        node
    }

    /// The target `s.value` of an assignment, on the handle node `base` (D5): a state place
    /// over the handle, writable when the `value` found is the writable one of a
    /// `MutableState<T>`, else `E4003`. Returns the place, the value type and whether it is
    /// writable, or `None` when `base` is no state handle.
    pub(super) fn state_value_target(
        &mut self,
        base: ExpressionId,
        member: &str,
        span: Span,
        target_span: Span,
    ) -> Option<(ExpressionId, Type, bool)> {
        let handle_ty = super::peel_indirection(&self.ty(base)).clone();
        let (value, writable) = self.state_value_member(&handle_ty, member)?;
        let base = self.peel_indirection_nodes(base);
        if !writable {
            self.errors.push(TypeError::ImmutabilityViolation {
                message: format!(
                    "cannot assign to the value of a '{}': a 'State<T>' is read-only",
                    handle_ty
                ),
                span,
                help: Some(format!(
                    "take a 'MutableState<{}>' to write the state, and pass it where a '{}' is expected",
                    value, handle_ty
                )),
            });
        }
        let handle = self.consume(base);
        let node = self.add(
            ExpressionKind::StatePlace { handle },
            value.clone(),
            target_span,
        );
        Some((node, value, writable))
    }

    /// Reports the write of a state's value that `statement` is (`E4031` in a composable
    /// context), for an assignment or, when `compound`, a compound assignment of the state
    /// place `target`; a compound assignment of a value that is not `Copy` is `E4033` outside
    /// composable contexts, with help naming `update`.
    pub(super) fn check_state_write(&mut self, target: ExpressionId, compound: bool, span: Span) {
        if !matches!(self.builder.kind(target), ExpressionKind::StatePlace { .. }) {
            return;
        }
        if self.is_in_compose() {
            let what = if compound {
                "a compound assignment of a state's value"
            } else {
                "an assignment of a state's value"
            };
            self.errors.push(TypeError::StateWriteInComposable {
                what: what.to_string(),
                span,
            });
            return;
        }
        let value = self.ty(target);
        if compound && !self.is_copy(&value) {
            let handle = match self.builder.kind(target) {
                ExpressionKind::StatePlace { handle } => *handle,
                _ => unreachable!("a state place"),
            };
            let handle = self.describe_handle(handle);
            self.errors.push(TypeError::StateValueOutsideComposable {
                message: format!(
                    "a compound assignment reads the value of '{}', a '{}' that is not 'Copy', outside a composable context",
                    handle, value
                ),
                help: format!(
                    "replace the value with '{}.update {{ value -> ... }}'",
                    handle
                ),
                span,
            });
        }
    }

    /// The `s.value` node a place is rooted at, through fields, elements and the temporary
    /// holding a copied value: a state place, or a call of the built-in `value`.
    pub(super) fn state_value_root(&self, mut id: ExpressionId) -> Option<ExpressionId> {
        loop {
            match self.builder.kind(id) {
                ExpressionKind::Field { base, .. } | ExpressionKind::Index { base, .. } => {
                    id = *base
                }
                ExpressionKind::Temporary { value } => id = *value,
                ExpressionKind::StatePlace { .. } => return Some(id),
                ExpressionKind::Call(Call {
                    callee: Callee::Builtin(BuiltinFunction::StateValue),
                    ..
                }) => return Some(id),
                _ => return None,
            }
        }
    }

    /// Reports a mutable use (`what`: a mutable borrow, a `mut fun` receiver or a part write)
    /// of the place `place` when it is rooted at `s.value` (D5): `E4031` in a composable
    /// context, `E4033` with help naming `edit` elsewhere. Returns whether it reported.
    pub(super) fn reject_state_mutation(
        &mut self,
        place: ExpressionId,
        what: &str,
        span: Span,
    ) -> bool {
        let Some(root) = self.state_value_root(place) else {
            return false;
        };
        if !self.builder.reported_state_values.insert(root) {
            return true;
        }
        if self.is_in_compose() {
            self.errors.push(TypeError::StateWriteInComposable {
                what: format!("{} of a state's value", what),
                span,
            });
            return true;
        }
        let handle = match self.builder.kind(root) {
            ExpressionKind::StatePlace { handle } => *handle,
            ExpressionKind::Call(call) => match call.arguments.first() {
                Some(Argument::Written(handle)) => *handle,
                _ => root,
            },
            _ => root,
        };
        let handle = self.describe_handle(handle);
        let copied = matches!(self.builder.kind(root), ExpressionKind::Call(_));
        let help = if copied {
            format!(
                "change the value in place with '{}.edit {{ value -> ... }}', or assign the whole value with '{}.value = ...'",
                handle, handle
            )
        } else {
            format!(
                "change the value in place with '{}.edit {{ value -> ... }}'",
                handle
            )
        };
        self.errors.push(TypeError::StateValueOutsideComposable {
            message: format!(
                "{} of the value of '{}' outside a composable context, where state is changed only through 'edit' or a whole assignment",
                what, handle
            ),
            help,
            span,
        });
        true
    }

    /// Reports the reads of `s.value` that only a composable context may make, built in the
    /// body that ends (D7): `E4033` with help naming `read`.
    pub(super) fn report_pending_state_reads(&mut self) {
        let pending = std::mem::take(&mut self.builder.pending_state_reads);
        for read in pending {
            if self.builder.reported_state_values.contains(&read.node) {
                continue;
            }
            let value = self.ty(read.node);
            self.errors.push(TypeError::StateValueOutsideComposable {
                message: format!(
                    "the value of '{}' is a '{}', which is not 'Copy': outside a composable context it is read only through 'read'",
                    read.handle, value
                ),
                help: format!(
                    "borrow the value inside '{}.read {{ value -> ... }}'",
                    read.handle
                ),
                span: read.span,
            });
        }
    }

    /// `E4032` for `mutableStateOf(args)` in a composable context, with the call wrapped in
    /// `remember` as its help (D7).
    pub(super) fn reject_unremembered_state(
        &mut self,
        callee: &Expr,
        args: &[CallArg],
        span: Span,
    ) {
        if !self.is_in_compose() {
            return;
        }
        let arguments: Vec<String> = args
            .iter()
            .map(|argument| match &argument.name {
                Some(name) => format!("{} = {}", name, argument.value),
                None => argument.value.to_string(),
            })
            .collect();
        self.errors.push(TypeError::UnrememberedState {
            remembered: format!("remember {{ {}({}) }}", callee, arguments.join(", ")),
            span: callee.span,
        });
        let _ = span;
    }

    /// `E4031` for a call of `update` or `edit` in a composable context (D7).
    pub(super) fn reject_state_member_in_composable(
        &mut self,
        builtin: BuiltinFunction,
        span: Span,
    ) {
        if matches!(
            builtin,
            BuiltinFunction::StateUpdate | BuiltinFunction::StateEdit
        ) && self.is_in_compose()
        {
            self.errors.push(TypeError::StateWriteInComposable {
                what: format!("a call of '{}'", builtin.name()),
                span,
            });
        }
    }

    /// A short text for the handle node `id`, for diagnostics: the name of a local, a
    /// top-level value or a field path, else `state`.
    fn describe_handle(&self, id: ExpressionId) -> String {
        match self.builder.kind(id) {
            ExpressionKind::Local(local) => self.builder.local_name(*local),
            ExpressionKind::TopLevelValue(path) => path.name.clone(),
            ExpressionKind::Field { base, name, .. } => {
                format!("{}.{}", self.describe_handle(*base), name)
            }
            ExpressionKind::Copy { place } | ExpressionKind::Move { place } => {
                self.describe_handle(*place)
            }
            ExpressionKind::Deref { pointer } => self.describe_handle(*pointer),
            ExpressionKind::Call(Call {
                callee: Callee::Builtin(BuiltinFunction::StateValue),
                arguments,
                ..
            }) => match arguments.first() {
                Some(Argument::Written(handle)) => {
                    format!("{}.{}", self.describe_handle(*handle), VALUE)
                }
                _ => "state".to_string(),
            },
            _ => "state".to_string(),
        }
    }
}
