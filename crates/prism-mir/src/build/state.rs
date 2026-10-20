//! State objects in MIR (D6 and D17 of `state-handles`, D3, D10 and D11 of `owned-state`).
//!
//! Every state operation is a `Call` of a [`StateOperation`] with the value type as its type
//! argument and the state object passed by shared borrow, held in a temporary once per access:
//!
//! - a state place is the dereference of the reference a read or a guard gives, as a
//!   top-level value is the dereference of its accessor's reference, and a `Copy` read copies
//!   out of it. In a composable context it is `state read`, which a pass keeps valid until the
//!   body ends. Elsewhere a shared borrow of it takes the state's flag shared and a mutable
//!   borrow, a `mut fun` receiver or a part write exclusively: a guard, `state begin shared` or
//!   `state begin exclusive` with the state object and a borrow of the guard's own local, whose
//!   scope ends it on every path (`state end shared`, `state end exclusive`): the statement's,
//!   or the block of the binding it initializes;
//! - `s.value = v` is `state write(&s, v)`, the state evaluated first and the value moved in,
//!   and `s.value op= e` reads, applies the operator with its checks and writes;
//! - `update` is `take`, a call of the transform and `put`; `weak` copies the state's word into
//!   a weak reference, and `get` gives the reference's own word viewed as the state object, or
//!   `null`.
//!
//! Each access costs a constant number of calls.

use super::Builder;
use crate::mir::{Callee, Constant, Operand, Place, Rvalue, StateOperation, TerminatorKind};
use prism_ast::Span;
use prism_typed_tree::ParameterKind;
use prism_typed_tree::{
    Argument, BinaryOperator, BuiltinFunction, Call, ExpressionId, IntrinsicStruct, Type,
};

/// How a state place is used, which decides the flag its access takes outside composable
/// contexts (D3 of `owned-state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum StateAccess {
    /// Read in place or copied out of: no flag past the read.
    #[default]
    Read,
    /// Borrowed shared: the flag shared until the guard's scope ends.
    Shared,
    /// Borrowed mutably, a `mut fun` receiver or written in part: the flag exclusively.
    Exclusive,
}

/// A guard whose begin call is made later: by a receiver's borrow, after the call's arguments.
#[derive(Debug, Clone)]
pub(super) struct DeferredGuard {
    operation: StateOperation,
    kind: IntrinsicStruct,
    value: Type,
    guard: crate::mir::LocalId,
    guard_borrow: crate::mir::LocalId,
    pointer: crate::mir::LocalId,
    span: Span,
}

/// The kind of the state object `ty` (a `State<T>` or `MutableState<T>`, or a shared borrow of
/// one) and its value type.
fn handle_kind(ty: &Type) -> (IntrinsicStruct, Type) {
    let object = match ty {
        Type::Reference { inner, .. } => inner.as_ref(),
        other => other,
    };
    match object {
        Type::Struct { id, arguments } => match (id.intrinsic(), &arguments[..]) {
            (Some(kind), [value]) if kind.is_state_handle() => (kind, value.clone()),
            _ => panic!("internal error: a state operation on a {}", ty),
        },
        _ => panic!("internal error: a state operation on a {}", ty),
    }
}

impl Builder<'_> {
    /// Ends the current block with a call of the state operation `operation` on a state object
    /// of kind `kind`, with the value type `value`, writing its result to `destination`.
    fn state_call(
        &mut self,
        operation: StateOperation,
        kind: IntrinsicStruct,
        value: &Type,
        arguments: Vec<Operand>,
        destination: Place,
        span: Span,
    ) {
        // The declared parameters mention the value type as a type parameter, which the type
        // argument gives (D17 of `state-handles`).
        let parameters = operation.parameters(kind);
        let parameter_kinds = vec![ParameterKind::Plain; parameters.len()];
        self.emit_call(
            Callee::State(operation),
            arguments,
            parameters,
            parameter_kinds,
            vec![value.clone()],
            destination,
            false,
            true,
            span,
        );
    }

    /// The borrow of the state object the node `handle` gives, held in a temporary of the
    /// innermost scope, with the state's kind and value type: what every operation of one
    /// access copies again.
    fn held_handle(&mut self, handle: ExpressionId) -> (Operand, IntrinsicStruct, Type) {
        let expression = self.expression(handle);
        let (ty, span) = (expression.ty.clone(), expression.span);
        let (kind, value) = handle_kind(&ty);
        let operand = self.operand(handle);
        // A borrow lowered into a temporary of this scope is already held: it is copied, with
        // no second temporary, since each local of an access costs every borrow-check analysis.
        if let Operand::Move(place, _) | Operand::Copy(place, _) = &operand {
            if let Some(local) = place.as_local() {
                if self.locals[local].kind == crate::mir::LocalKind::Temporary {
                    return (Operand::Copy(Place::local(local), span), kind, value);
                }
            }
        }
        let local = self.new_temporary(ty, span);
        self.assign(Place::local(local), Rvalue::Use(operand), span);
        (Operand::Copy(Place::local(local), span), kind, value)
    }

    /// The place of `id`, used as `access` decides when it is a state's value or a part of one.
    pub(super) fn place_with_access(&mut self, id: ExpressionId, access: StateAccess) -> Place {
        let saved = std::mem::replace(&mut self.state_access, access);
        let place = self.place(id);
        self.state_access = saved;
        place
    }

    /// The place of the state place `node` over `handle` (D3 of `owned-state`): `*_r` after
    /// `_r = state read<T>(&s)` in a composable context or for a read, else after a guard of
    /// the access the place is lowered for, owned by the statement's scope, or by the block of
    /// the binding whose initializer borrows it.
    pub(super) fn state_place(
        &mut self,
        node: ExpressionId,
        handle: ExpressionId,
        span: Span,
    ) -> Place {
        let access = std::mem::take(&mut self.state_access);
        let extended = self.extended_guards.remove(&node);
        let handle_ty = self.expression(handle).ty.clone();
        let (kind, value) = handle_kind(&handle_ty);
        let (operation, pointer_ty) = match access {
            _ if self.is_compose => (StateOperation::Read, false),
            StateAccess::Read => (StateOperation::Read, false),
            StateAccess::Shared => (StateOperation::BeginShared, false),
            StateAccess::Exclusive => (StateOperation::BeginExclusive, true),
        };
        let reference = Type::Reference {
            mutable: pointer_ty,
            inner: Box::new(value.clone()),
        };
        let Some(end) = operation.guard_end() else {
            let (handle, _, _) = self.held_handle(handle);
            let pointer = self.new_temporary(reference, span);
            self.state_call(
                operation,
                kind,
                &value,
                vec![handle],
                Place::local(pointer),
                span,
            );
            return Self::deref(Place::local(pointer));
        };
        // The guard's local holds the state object's borrow until its scope ends; the
        // reference the begin call gives is tied to a borrow of it, so it cannot outlive the
        // guard (`E5006`). The borrow is lowered into the guard's local directly, with no
        // temporary between them, since each local of an access costs every borrow-check
        // analysis.
        let scope = extended.unwrap_or(self.scope_depth() - 1);
        let guard = self.new_guard_local(handle_ty.clone(), end, value.clone(), scope, span);
        self.expression_into(Some(&Place::local(guard)), handle);
        let pointer = self.new_temporary(reference, span);
        let guard_borrow = self.new_temporary(
            Type::Reference {
                mutable: false,
                inner: Box::new(handle_ty),
            },
            span,
        );
        let deferred = DeferredGuard {
            operation,
            kind,
            value,
            guard,
            guard_borrow,
            pointer,
            span,
        };
        if self.defer_guard {
            self.deferred_guard = Some(deferred);
        } else {
            self.take_deferred_guard(deferred);
        }
        Self::deref(Place::local(pointer))
    }

    /// Takes the guard `deferred`: borrows its local and calls its begin operation.
    pub(super) fn take_deferred_guard(&mut self, deferred: DeferredGuard) {
        let DeferredGuard {
            operation,
            kind,
            value,
            guard,
            guard_borrow,
            pointer,
            span,
        } = deferred;
        self.assign(
            Place::local(guard_borrow),
            Rvalue::Borrow {
                mutable: false,
                origin: crate::mir::BorrowOrigin::StateGuard,
                place: Place::local(guard),
            },
            span,
        );
        self.state_call(
            operation,
            kind,
            &value,
            vec![
                Operand::Copy(Place::local(guard), span),
                Operand::Move(Place::local(guard_borrow), span),
            ],
            Place::local(pointer),
            span,
        );
    }

    /// Ends the guard whose local is `guard`, with `end`, on a state of value type `value`.
    pub(super) fn end_guard(
        &mut self,
        guard: crate::mir::LocalId,
        end: StateOperation,
        value: &Type,
        span: Span,
    ) {
        if self.current.is_none() {
            return;
        }
        let (kind, _) = handle_kind(&self.locals[guard].ty.clone());
        let parameters = end.parameters(kind);
        // The call's `Unit` result goes to a local live only around it, since the scope's own
        // locals are ending.
        let unit = self.add_local(crate::mir::LocalDeclaration {
            ty: Type::Unit,
            kind: crate::mir::LocalKind::Temporary,
            mutable: false,
            name: None,
            binding: None,
            parameter_kind: ParameterKind::Plain,
            span,
        });
        self.push_statement(crate::mir::StatementKind::StorageLive(unit), span);
        let target = self.new_block();
        self.terminate(
            TerminatorKind::Call {
                callee: Callee::State(end),
                arguments: vec![Operand::Copy(Place::local(guard), span)],
                parameter_kinds: vec![ParameterKind::Plain; parameters.len()],
                parameters,
                type_arguments: vec![value.clone()],
                destination: Place::local(unit),
                target: Some(target),
                is_compose: false,
            },
            span,
        );
        self.current = Some(target);
        self.push_statement(crate::mir::StatementKind::StorageDead(unit), span);
    }

    /// `s.value = v`: the state, then the value, then `state write(&s, v)`, which replaces the
    /// state's value (D5 of `state-handles`).
    pub(super) fn lower_state_assign(
        &mut self,
        handle: ExpressionId,
        value: ExpressionId,
        span: Span,
    ) {
        let (handle, kind, value_ty) = self.held_handle(handle);
        let operand = self.operand(value);
        let expression = self.expression(value);
        let value_span = expression.span;
        let written = self.new_temporary(value_ty.clone(), value_span);
        self.assign(Place::local(written), Rvalue::Use(operand), value_span);
        if self.current.is_none() {
            return;
        }
        let unit = self.new_temporary(Type::Unit, span);
        let moved = Operand::Move(Place::local(written), value_span);
        self.state_call(
            StateOperation::Write,
            kind,
            &value_ty,
            vec![handle, moved],
            Place::local(unit),
            span,
        );
    }

    /// `s.value op= e` of a `Copy` value: the state once, a read copied out, the value, the
    /// operator with its checks, then the write of the result.
    pub(super) fn lower_state_compound_assign(
        &mut self,
        operator: BinaryOperator,
        handle: ExpressionId,
        value: ExpressionId,
        target_span: Span,
        span: Span,
    ) {
        let (handle, kind, value_ty) = self.held_handle(handle);
        let reference = Type::Reference {
            mutable: false,
            inner: Box::new(value_ty.clone()),
        };
        let pointer = self.new_temporary(reference, target_span);
        self.state_call(
            StateOperation::Read,
            kind,
            &value_ty,
            vec![handle.clone()],
            Place::local(pointer),
            target_span,
        );
        let current = self.new_temporary(value_ty.clone(), target_span);
        let read = Operand::Copy(Self::deref(Place::local(pointer)), target_span);
        self.assign(Place::local(current), Rvalue::Use(read), target_span);
        let right = self.operand(value);
        if self.current.is_none() {
            return;
        }
        let result = self.new_temporary(value_ty.clone(), span);
        let left = Operand::Copy(Place::local(current), target_span);
        self.assign(
            Place::local(result),
            Rvalue::Binary(operator, left, right),
            span,
        );
        let unit = self.new_temporary(Type::Unit, span);
        let moved = Operand::Move(Place::local(result), span);
        self.state_call(
            StateOperation::Write,
            kind,
            &value_ty,
            vec![handle, moved],
            Place::local(unit),
            span,
        );
    }

    /// A call of one of the state built-ins (D6, D17 of `state-handles`, D11 of
    /// `owned-state`): `mutableStateOf`, a `Copy` read of `value`, `update`, `weak` or `get`.
    pub(super) fn lower_state_call(
        &mut self,
        destination: Option<&Place>,
        call: &Call,
        ty: &Type,
        span: Span,
    ) {
        let prism_typed_tree::Callee::Builtin(builtin) = &call.callee else {
            unreachable!("a state built-in");
        };
        let written = |index: usize| match call.arguments.get(index) {
            Some(Argument::Written(value)) => *value,
            _ => panic!(
                "internal error: a state call without its argument {}",
                index
            ),
        };
        match builtin {
            BuiltinFunction::MutableStateOf => {
                let value = written(0);
                let value_ty = self.expression(value).ty.clone();
                let operand = self.argument_operand(value);
                if self.current.is_none() {
                    return;
                }
                let destination = self.destination_or_temporary(destination, ty, span);
                self.state_call(
                    StateOperation::Create,
                    IntrinsicStruct::MutableState,
                    &value_ty,
                    vec![operand],
                    destination,
                    span,
                );
            }
            BuiltinFunction::StateValue => {
                let (handle, kind, value_ty) = self.held_handle(written(0));
                let reference = Type::Reference {
                    mutable: false,
                    inner: Box::new(value_ty.clone()),
                };
                let pointer = self.new_temporary(reference, span);
                self.state_call(
                    StateOperation::Read,
                    kind,
                    &value_ty,
                    vec![handle],
                    Place::local(pointer),
                    span,
                );
                if let Some(destination) = destination {
                    let copied = Operand::Copy(Self::deref(Place::local(pointer)), span);
                    self.assign(destination.clone(), Rvalue::Use(copied), span);
                }
            }
            BuiltinFunction::StateWeak => {
                let (handle, kind, value_ty) = self.held_handle(written(0));
                if self.current.is_none() {
                    return;
                }
                let destination = self.destination_or_temporary(destination, ty, span);
                self.state_call(
                    StateOperation::Weak,
                    kind,
                    &value_ty,
                    vec![handle],
                    destination,
                    span,
                );
            }
            BuiltinFunction::WeakGet => {
                let receiver = written(0);
                let receiver_ty = self.expression(receiver).ty.clone();
                let state = match &receiver_ty {
                    Type::Reference { inner, .. } => match inner.as_ref() {
                        Type::Struct { arguments, .. } if arguments.len() == 1 => {
                            arguments[0].clone()
                        }
                        other => panic!("internal error: get on a {}", other),
                    },
                    other => panic!("internal error: get on a {}", other),
                };
                let operand = self.operand(receiver);
                let local = self.new_temporary(receiver_ty, span);
                self.assign(Place::local(local), Rvalue::Use(operand), span);
                if self.current.is_none() {
                    return;
                }
                let destination = self.destination_or_temporary(destination, ty, span);
                let parameters = StateOperation::Get.parameters(IntrinsicStruct::MutableState);
                self.emit_call(
                    Callee::State(StateOperation::Get),
                    vec![Operand::Copy(Place::local(local), span)],
                    parameters,
                    vec![ParameterKind::Plain],
                    vec![state],
                    destination,
                    false,
                    true,
                    span,
                );
            }
            BuiltinFunction::StateUpdate => {
                let (handle, kind, value_ty) = self.held_handle(written(0));
                let closure = written(1);
                let closure_expression = self.expression(closure);
                let (closure_ty, closure_span) =
                    (closure_expression.ty.clone(), closure_expression.span);
                let function = self.argument_operand(closure);
                if self.current.is_none() {
                    return;
                }
                let function = match function {
                    Operand::Move(place, _) | Operand::Copy(place, _) => place,
                    Operand::Constant(_) => {
                        panic!("internal error: a state member called with a constant closure")
                    }
                };
                let closure_parameters = match &closure_ty {
                    Type::Fn { params, .. } => params.clone(),
                    other => panic!("internal error: a state member closure of type {}", other),
                };
                let taken = self.new_temporary(value_ty.clone(), span);
                self.state_call(
                    StateOperation::Take,
                    kind,
                    &value_ty,
                    vec![handle.clone()],
                    Place::local(taken),
                    span,
                );
                let result = Place::local(self.new_temporary(value_ty.clone(), span));
                let parameter_kinds = vec![ParameterKind::Plain; closure_parameters.len()];
                self.emit_call(
                    Callee::Value(function, closure_span),
                    vec![Operand::Move(Place::local(taken), span)],
                    closure_parameters,
                    parameter_kinds,
                    Vec::new(),
                    result.clone(),
                    false,
                    true,
                    span,
                );
                let unit = self.new_temporary(Type::Unit, span);
                self.state_call(
                    StateOperation::Put,
                    kind,
                    &value_ty,
                    vec![handle, Operand::Move(result, span)],
                    Place::local(unit),
                    span,
                );
                if let Some(destination) = destination {
                    let unit = Operand::Constant(Constant::Unit);
                    self.assign(destination.clone(), Rvalue::Use(unit), span);
                }
            }
            other => panic!("internal error: {} is no state built-in", other.name()),
        }
    }

    /// The operand of a written argument of a state call, moved out of a temporary of the
    /// innermost scope.
    fn argument_operand(&mut self, value: ExpressionId) -> Operand {
        let expression = self.expression(value);
        let (ty, span) = (expression.ty.clone(), expression.span);
        let operand = self.operand(value);
        match &operand {
            Operand::Move(place, _)
                if place
                    .as_local()
                    .is_some_and(|local| self.is_temporary_of_innermost_scope(local)) =>
            {
                operand
            }
            _ => {
                let local = self.new_temporary(ty, span);
                self.assign(Place::local(local), Rvalue::Use(operand), span);
                Operand::Move(Place::local(local), span)
            }
        }
    }
}
