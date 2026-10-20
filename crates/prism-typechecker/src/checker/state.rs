//! State objects in the type checker (D3, D5 and D7 of `state-handles`, D1 to D5 of
//! `owned-state`).
//!
//! A state object (`State<T>`, `MutableState<T>`) is reached by value or through references;
//! every operation on it takes a shared borrow of it. `s.value` is typed by its own rule rather
//! than as a computed property:
//!
//! - for a `Copy` `T` (by its declared marker, a type parameter only through a `Copy` bound),
//!   a read is a call of [`BuiltinFunction::StateValue`] with a shared borrow of the state
//!   object as its receiver, so borrowing it borrows a temporary;
//! - for any other `T`, it is an [`ExpressionKind::StatePlace`] over that borrow, read in
//!   place, borrowed and projected in every context, never moved out (`E4017`);
//! - as the target of an assignment it is a state place of either kind, written through a
//!   runtime call; a `MutableState<T>` is written through any access, shared borrows included
//!   (D2 of `owned-state`), a `State<T>` never (`E4003`).
//!
//! A mutable borrow, a `mut fun` receiver or a part write of `s.value` is `E4031` in a
//! composable context and allowed elsewhere on a `MutableState<T>`, where it turns a `Copy`
//! read into a state place. Writes and `update` in a composable context are `E4031`, and
//! `mutableStateOf` there is `E4032`. Each rule costs a member lookup, a marker lookup and a
//! test of the composable-context flag.

use super::TypeChecker;
use crate::error::TypeError;
use crate::types::Type;
use prism_ast::expr::{CallArg, Expr};
use prism_ast::Span;
use prism_typed_tree::{
    Argument, BorrowOrigin, BuiltinFunction, Call, Callee, ExpressionId, ExpressionKind,
    IntrinsicStruct,
};

/// The member name of a state's value.
pub(super) const VALUE: &str = "value";

/// The kind and value type of `ty` when it is a state object, `State<T>` or `MutableState<T>`.
pub(super) fn state_handle(ty: &Type) -> Option<(IntrinsicStruct, &Type)> {
    match ty {
        Type::Struct { id, arguments } => match (id.intrinsic(), &arguments[..]) {
            (Some(handle), [value]) if handle.is_state_handle() => Some((handle, value)),
            _ => None,
        },
        _ => None,
    }
}

/// `State<T>` for `MutableState<T>`: the type a lookup that misses on a mutable state is
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
    /// The type of `value` on the state object `peeled` and whether it may be assigned, when
    /// `member` is `value` and the state's module is visible: the value type, writable on a
    /// `MutableState<T>`.
    pub(super) fn state_value_member(&self, peeled: &Type, member: &str) -> Option<(Type, bool)> {
        if member != VALUE {
            return None;
        }
        let (_, value) = state_handle(peeled)?;
        let found = self.member(peeled, member).filter(|m| m.property)?;
        Some((value.clone(), found.writable))
    }

    /// A shared borrow of the state object at the place `object` (D3 of `owned-state`), which a
    /// closure using it captures as the path of `object` (`*count`, `todo.done`).
    fn state_object_borrow(&mut self, object: ExpressionId) -> ExpressionId {
        self.borrow(false, BorrowOrigin::Receiver, object)
    }

    /// The node of `s.value` read on the state object `base` (D5 of `state-handles`): a call
    /// of the built-in `value` for a `Copy` value type, else a state place.
    pub(super) fn state_value_node(
        &mut self,
        base: ExpressionId,
        value: Type,
        span: Span,
    ) -> ExpressionId {
        let handle = self.state_object_borrow(base);
        let handle_ty = self.ty(handle);
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
                operator: None,
            };
            return self.add(ExpressionKind::Call(call), value, span);
        }
        self.builder.state_value_spans.insert(span);
        self.add(ExpressionKind::StatePlace { handle }, value, span)
    }

    /// The target `s.value` of an assignment, on the node `base` reaching a state object (D5):
    /// a state place over a shared borrow of it, writable when the `value` found is the
    /// writable one of a `MutableState<T>`, else `E4003`. Returns the place, the value type and
    /// whether it is writable, or `None` when `base` reaches no state object.
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
            self.report_read_only_state(&handle_ty, &value, span);
        }
        let handle = self.state_object_borrow(base);
        let node = self.add(
            ExpressionKind::StatePlace { handle },
            value.clone(),
            target_span,
        );
        Some((node, value, writable))
    }

    /// `E4003` for a change of the value of the read-only `State<T>` `handle_ty`, whose value
    /// type is `value`.
    fn report_read_only_state(&mut self, handle_ty: &Type, value: &Type, span: Span) {
        self.errors.push(TypeError::ImmutabilityViolation {
            message: format!(
                "cannot assign to the value of a '{}': a 'State<T>' is read-only",
                handle_ty
            ),
            span,
            help: Some(format!(
                "take a '&MutableState<{}>' to write the state, and pass it where a '&{}' is expected",
                value, handle_ty
            )),
        });
    }

    /// Reports the write of a state's value that `statement` is (`E4031` in a composable
    /// context), for an assignment or, when `compound`, a compound assignment of the state
    /// place `target`.
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
        }
    }

    /// `E4017` for the compound assignment `target op= value` of a state place whose value is
    /// not `Copy`, once the operator is known to apply: it would read the value out by copy.
    /// The help shows the forms that work, assigning the result (whose operands are read in
    /// place) or transforming the value with `update`. A composable context already reported
    /// `E4031`.
    pub(super) fn check_state_compound_value(
        &mut self,
        target: ExpressionId,
        operator: &str,
        value: &Expr,
        span: Span,
    ) {
        let ExpressionKind::StatePlace { handle } = self.builder.kind(target).clone() else {
            return;
        };
        let ty = self.ty(target);
        if self.is_in_compose() || self.is_copy(&ty) {
            return;
        }
        self.errors.push(TypeError::CompoundAssignmentOfStateValue {
            handle: self.describe_handle(handle),
            ty,
            operator: operator.to_string(),
            value: value.to_string(),
            span,
        });
    }

    /// The `s.value` node a place is rooted at, through fields, elements and the temporary
    /// holding a copied value: a state place, or a call of the built-in `value`; and the
    /// temporary over that call, when the place reads a part of a copied value.
    fn state_value_root_and_temporary(
        &self,
        mut id: ExpressionId,
    ) -> Option<(ExpressionId, Option<ExpressionId>)> {
        let mut temporary = None;
        loop {
            match self.builder.kind(id) {
                ExpressionKind::Field { base, .. } | ExpressionKind::Index { base, .. } => {
                    temporary = None;
                    id = *base
                }
                ExpressionKind::Temporary { value } => {
                    temporary = Some(id);
                    id = *value
                }
                ExpressionKind::StatePlace { .. } => return Some((id, None)),
                ExpressionKind::Call(Call {
                    callee: Callee::Builtin(BuiltinFunction::StateValue),
                    ..
                }) => return Some((id, temporary)),
                _ => return None,
            }
        }
    }

    /// The `s.value` node a place is rooted at, through fields, elements and the temporary
    /// holding a copied value: a state place, or a call of the built-in `value`.
    pub(super) fn state_value_root(&self, id: ExpressionId) -> Option<ExpressionId> {
        self.state_value_root_and_temporary(id)
            .map(|(root, _)| root)
    }

    /// Reports a mutable use (`what`: a mutable borrow, a `mut fun` receiver or a part write)
    /// of the place `place` when it is rooted at `s.value` (D3 of `owned-state`): `E4031` in a
    /// composable context, `E4003` on a read-only `State<T>`. Elsewhere it is allowed, and a
    /// `Copy` read at its root becomes a state place, changed in place. Returns whether it
    /// reported.
    pub(super) fn reject_state_mutation(
        &mut self,
        place: ExpressionId,
        what: &str,
        span: Span,
    ) -> bool {
        let Some((root, temporary)) = self.state_value_root_and_temporary(place) else {
            return false;
        };
        if self.builder.reported_state_values.contains(&root) {
            return true;
        }
        if self.is_in_compose() {
            self.builder.reported_state_values.insert(root);
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
                _ => return false,
            },
            _ => return false,
        };
        let handle_ty = super::peel_indirection(&self.ty(handle)).clone();
        if let Some((IntrinsicStruct::State, value)) = state_handle(&handle_ty) {
            let value = value.clone();
            self.builder.reported_state_values.insert(root);
            self.report_read_only_state(&handle_ty, &value, span);
            return true;
        }
        if let ExpressionKind::Call(_) = self.builder.kind(root) {
            // A part of a `Copy` value is changed in place: the read becomes the state place.
            let (value, root_span) = (self.ty(root), self.builder.span_of(root));
            let target = temporary.unwrap_or(root);
            self.builder.replace(
                target,
                ExpressionKind::StatePlace { handle },
                value,
                root_span,
            );
        }
        false
    }

    /// `E4032` for `mutableStateOf(args)` in a composable context, with the call wrapped in
    /// `remember` as its help (D7 of `state-handles`).
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

    /// `E4031` for a call of `update` in a composable context (D7 of `state-handles`).
    pub(super) fn reject_state_member_in_composable(
        &mut self,
        builtin: BuiltinFunction,
        span: Span,
    ) {
        if builtin == BuiltinFunction::StateUpdate && self.is_in_compose() {
            self.errors.push(TypeError::StateWriteInComposable {
                what: format!("a call of '{}'", builtin.name()),
                span,
            });
        }
    }

    /// A short text for the state node `id`, for diagnostics: the name of a local, a
    /// top-level value or a field path, else `state`.
    fn describe_handle(&self, id: ExpressionId) -> String {
        match self.builder.kind(id) {
            ExpressionKind::Local(local) => self.builder.local_name(*local),
            ExpressionKind::TopLevelValue(path) => path.name.clone(),
            ExpressionKind::Field { base, name, .. } => {
                format!("{}.{}", self.describe_handle(*base), name)
            }
            ExpressionKind::Copy { place }
            | ExpressionKind::Move { place }
            | ExpressionKind::Borrow { place, .. } => self.describe_handle(*place),
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
