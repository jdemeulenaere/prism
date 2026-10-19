//! State handles in MIR (D6 and D17 of `state-handles`).
//!
//! Every state operation is a `Call` of a [`StateOperation`] with the value type as its type
//! argument and the handle copied:
//!
//! - a state place is the dereference of the `&T` that `state read` gives, as a top-level
//!   value is the dereference of its accessor's reference, and a `Copy` read copies out of it;
//! - `s.value = v` is `state write(h, v)`, the handle evaluated first and the value moved in,
//!   and `s.value op= e` reads, applies the operator with its checks and writes, the handle
//!   held in a temporary once;
//! - `read`, `update` and `edit` are a begin call, a call of the closure value and an end
//!   call: `begin_read`/`end_read` around a `&T`, `take`/`put` around the value moved out and
//!   back in, `begin_edit`/`end_edit` around a `&mut T`.
//!
//! Each access costs a constant number of calls.

use super::Builder;
use crate::mir::{Callee, Constant, Operand, Place, Rvalue, StateOperation};
use prism_ast::Span;
use prism_typed_tree::{
    Argument, BinaryOperator, BuiltinFunction, Call, CoercionKind, ExpressionId, ExpressionKind,
    IntrinsicStruct, Type,
};

/// The handle kind of `ty`, a `State<T>` or `MutableState<T>`, and its value type.
fn handle_kind(ty: &Type) -> (IntrinsicStruct, Type) {
    match ty {
        Type::Struct { id, arguments } => match (id.intrinsic(), &arguments[..]) {
            (Some(kind), [value]) if kind.is_state_handle() => (kind, value.clone()),
            _ => panic!("internal error: a state operation on a {}", ty),
        },
        _ => panic!("internal error: a state operation on a {}", ty),
    }
}

impl Builder<'_> {
    /// Ends the current block with a call of the state operation `operation` on a handle of
    /// kind `kind`, with the value type `value`, writing its result to `destination`.
    fn state_call(
        &mut self,
        operation: StateOperation,
        kind: IntrinsicStruct,
        value: &Type,
        arguments: Vec<Operand>,
        destination: Place,
        span: Span,
    ) {
        let parameters = operation.parameters(kind);
        let escaping = vec![false; parameters.len()];
        self.emit_call(
            Callee::State(operation),
            arguments,
            parameters,
            escaping,
            vec![value.clone()],
            destination,
            false,
            true,
            span,
        );
    }

    /// The handle of the state node `handle` copied into a temporary of the innermost scope,
    /// with its kind and value type: what every operation of one access copies again.
    fn held_handle(&mut self, handle: ExpressionId) -> (Operand, IntrinsicStruct, Type) {
        let expression = self.expression(handle);
        let (ty, span) = (expression.ty.clone(), expression.span);
        let (kind, value) = handle_kind(&ty);
        let operand = self.operand(handle);
        let local = self.new_temporary(ty, span);
        self.assign(Place::local(local), Rvalue::Use(operand), span);
        (Operand::Copy(Place::local(local), span), kind, value)
    }

    /// The handle a call of a state member takes as its receiver: the receiver node with its
    /// borrow and conversion stripped, since the operations take the handle by value.
    fn receiver_handle(&mut self, receiver: ExpressionId) -> (Operand, IntrinsicStruct, Type) {
        let mut id = receiver;
        loop {
            match &self.expression(id).kind {
                ExpressionKind::Coerce {
                    value,
                    kind: CoercionKind::Subtype,
                } => id = *value,
                ExpressionKind::Borrow { place, .. } => {
                    let place = *place;
                    let expression = self.expression(place);
                    let (ty, span) = (expression.ty.clone(), expression.span);
                    let (kind, value) = handle_kind(&ty);
                    let place = self.place(place);
                    let local = self.new_temporary(ty, span);
                    let copied = Operand::Copy(place, span);
                    self.assign(Place::local(local), Rvalue::Use(copied), span);
                    return (Operand::Copy(Place::local(local), span), kind, value);
                }
                _ => return self.held_handle(id),
            }
        }
    }

    /// The place of the state place over `handle`: `*_r` after `_r = state read<T>(h)`.
    pub(super) fn state_place(&mut self, handle: ExpressionId, span: Span) -> Place {
        let (handle, kind, value) = self.held_handle(handle);
        let reference = Type::Reference {
            mutable: false,
            inner: Box::new(value.clone()),
        };
        let pointer = self.new_temporary(reference, span);
        self.state_call(
            StateOperation::Read,
            kind,
            &value,
            vec![handle],
            Place::local(pointer),
            span,
        );
        Self::deref(Place::local(pointer))
    }

    /// `s.value = v`: the handle, then the value, then `state write(h, v)`, which replaces the
    /// cell's value (D5).
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

    /// `s.value op= e` of a `Copy` value: the handle once, a read copied out, the value, the
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

    /// A call of one of the state built-ins (D6, D17): `mutableStateOf`, a `Copy` read of
    /// `value`, `read`, `update` or `edit`.
    pub(super) fn lower_state_call(
        &mut self,
        destination: Option<&Place>,
        call: &Call,
        ty: &Type,
        span: Span,
    ) {
        let Callee::Builtin(builtin) = Self::state_callee(call) else {
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
                let (handle, kind, value_ty) = self.receiver_handle(written(0));
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
            BuiltinFunction::StateRead
            | BuiltinFunction::StateUpdate
            | BuiltinFunction::StateEdit => {
                let (handle, kind, value_ty) = self.receiver_handle(written(0));
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
                let (begin, end, lent) = match builtin {
                    BuiltinFunction::StateRead => (
                        StateOperation::BeginRead,
                        StateOperation::EndRead,
                        Type::Reference {
                            mutable: false,
                            inner: Box::new(value_ty.clone()),
                        },
                    ),
                    BuiltinFunction::StateUpdate => {
                        (StateOperation::Take, StateOperation::Put, value_ty.clone())
                    }
                    _ => (
                        StateOperation::BeginEdit,
                        StateOperation::EndEdit,
                        Type::Reference {
                            mutable: true,
                            inner: Box::new(value_ty.clone()),
                        },
                    ),
                };
                let lent_local = self.new_temporary(lent, span);
                self.state_call(
                    begin,
                    kind,
                    &value_ty,
                    vec![handle.clone()],
                    Place::local(lent_local),
                    span,
                );
                let result_ty = match builtin {
                    BuiltinFunction::StateUpdate => value_ty.clone(),
                    _ => ty.clone(),
                };
                let result = match builtin {
                    BuiltinFunction::StateUpdate => {
                        Place::local(self.new_temporary(result_ty, span))
                    }
                    _ => self.destination_or_temporary(destination, &result_ty, span),
                };
                let escaping = vec![false; closure_parameters.len()];
                self.emit_call(
                    Callee::Value(function, closure_span),
                    vec![Operand::Move(Place::local(lent_local), span)],
                    closure_parameters,
                    escaping,
                    Vec::new(),
                    result.clone(),
                    false,
                    true,
                    span,
                );
                let mut arguments = vec![handle];
                if builtin == BuiltinFunction::StateUpdate {
                    arguments.push(Operand::Move(result, span));
                }
                let unit = self.new_temporary(Type::Unit, span);
                self.state_call(end, kind, &value_ty, arguments, Place::local(unit), span);
                if builtin == BuiltinFunction::StateUpdate {
                    if let Some(destination) = destination {
                        let unit = Operand::Constant(Constant::Unit);
                        self.assign(destination.clone(), Rvalue::Use(unit), span);
                    }
                }
            }
            other => panic!("internal error: {} is no state built-in", other.name()),
        }
    }

    /// The MIR callee of a state built-in call.
    fn state_callee(call: &Call) -> Callee {
        match &call.callee {
            prism_typed_tree::Callee::Builtin(builtin) => Callee::Builtin(*builtin),
            _ => unreachable!("a state built-in"),
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
