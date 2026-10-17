//! Calls and default values (D10 and D11 of `mir-lowering`).

use super::{BodySlots, Builder, Preallocated};
use crate::mir::{
    Body, BodyId, BodyOwner, BorrowOrigin, Callee, Constant, Operand, Place, Projection, Rvalue,
    TerminatorKind,
};
use prism_ast::Span;
use prism_typed_tree::{
    Argument, BorrowOrigin as TreeBorrowOrigin, Call, Callee as TreeCallee, ExpressionId,
    ExpressionKind, FunctionItem, SafeNavigationOrigin, Type, TypeTable, TypedFile,
};

/// A receiver that a call borrows: its place is evaluated before the written arguments and
/// borrowed after them (D10 of `mir-lowering`). A receiver that the call moves or copies is
/// evaluated as an ordinary argument, first (D10 of `borrowck-moves`).
struct PendingReceiver {
    mutable: bool,
    place: Place,
}

/// The type of the slot a generator of a parameter of type `parameter` fills, when the
/// parameter has a reference type (`&R`, `&mut R`, `(&R)?` or `(&mut R)?`): `R?` (D11).
pub(super) fn slot_type(parameter: &Type) -> Option<Type> {
    let reference = match parameter {
        Type::Nullable(inner) => inner.as_ref(),
        other => other,
    };
    match reference {
        Type::Reference { inner, .. } => Some(inner.as_ref().clone().nullable()),
        _ => None,
    }
}

fn shared(ty: Type) -> Type {
    Type::Reference {
        mutable: false,
        inner: Box::new(ty),
    }
}

impl Builder<'_> {
    /// A call: the callee, then the receiver (moved or copied into a temporary when the call
    /// takes it by value, or only its place evaluated when the call borrows it), the written
    /// arguments in source order (each in a temporary, except constants no default borrows),
    /// the borrowed receiver's borrow, the omitted arguments in parameter order by calling their
    /// generators, then the call (D10 of `mir-lowering` and of `borrowck-moves`).
    pub(super) fn lower_call(
        &mut self,
        destination: Option<&Place>,
        call: &Call,
        ty: &Type,
        span: Span,
    ) {
        let callee = match &call.callee {
            TreeCallee::Function(path) => Callee::Function(path.clone()),
            TreeCallee::Builtin(builtin) => Callee::Builtin(*builtin),
            TreeCallee::Value(value) => Callee::Value(self.place(*value)),
        };
        let count = call.arguments.len();
        let last_default = call
            .arguments
            .iter()
            .rposition(|argument| matches!(argument, Argument::Default));
        let mut operands: Vec<Option<Operand>> = vec![None; count];
        let mut pending = None;
        if call.has_receiver {
            if let Some(Argument::Written(receiver)) = call.arguments.first() {
                pending = self.receiver_place(*receiver);
            }
        }
        for &index in &call.evaluation_order {
            if index == 0 && pending.is_some() {
                continue;
            }
            let Argument::Written(value) = call.arguments[index] else {
                panic!("internal error: call evaluates an omitted argument");
            };
            let borrowed_by_default = last_default.is_some_and(|last| last > index);
            operands[index] = Some(self.argument(value, borrowed_by_default));
        }
        if let Some(pending) = pending {
            let receiver_ty = call.parameters[0].1.clone();
            let local = self.new_temporary(receiver_ty, span);
            let rvalue = Rvalue::Borrow {
                mutable: pending.mutable,
                origin: BorrowOrigin::Receiver,
                place: pending.place,
            };
            self.assign(Place::local(local), rvalue, span);
            operands[0] = Some(Operand::Move(Place::local(local)));
        }
        for index in 0..count {
            if matches!(call.arguments[index], Argument::Default) {
                let operand = self.default_argument(call, index, &operands, span);
                operands[index] = Some(operand);
            }
        }
        if self.current.is_none() {
            return;
        }
        let arguments: Vec<Operand> = operands
            .into_iter()
            .map(|operand| operand.expect("every argument is evaluated"))
            .collect();
        let parameters = call.parameters.iter().map(|(_, ty)| ty.clone()).collect();
        let destination = self.destination_or_temporary(destination, ty, span);
        self.emit_call(
            callee,
            arguments,
            parameters,
            destination,
            call.is_compose,
            *ty != Type::Nothing,
            span,
        );
    }

    /// Ends the current block with a call, continuing in a new block unless the callee never
    /// returns.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_call(
        &mut self,
        callee: Callee,
        arguments: Vec<Operand>,
        parameters: Vec<Type>,
        destination: Place,
        is_compose: bool,
        returns: bool,
        span: Span,
    ) {
        if self.current.is_none() {
            return;
        }
        for argument in &arguments {
            self.record_operand_moved(argument);
        }
        let target = returns.then(|| self.new_block());
        let kind = TerminatorKind::Call {
            callee,
            arguments,
            parameters,
            destination,
            target,
            is_compose,
        };
        self.terminate(kind, span);
        self.current = target;
    }

    /// Evaluates the place of a receiver that is borrowed after the arguments, or returns `None`
    /// for a receiver evaluated as an ordinary argument: one the call moves or copies, which is
    /// moved or copied before the arguments (D10 of `borrowck-moves`).
    fn receiver_place(&mut self, receiver: ExpressionId) -> Option<PendingReceiver> {
        match self.expression(receiver).kind {
            ExpressionKind::Borrow {
                mutable,
                origin: TreeBorrowOrigin::Receiver,
                place,
            } => Some(PendingReceiver {
                mutable,
                place: self.place(place),
            }),
            _ => None,
        }
    }

    /// A written argument, in a temporary unless it is a constant that no default generator
    /// borrows.
    fn argument(&mut self, value: ExpressionId, borrowed_by_default: bool) -> Operand {
        let expression = self.expression(value);
        let (ty, span) = (expression.ty.clone(), expression.span);
        let operand = self.operand(value);
        match &operand {
            Operand::Constant(_) if !borrowed_by_default => operand,
            Operand::Move(place)
                if place
                    .as_local()
                    .is_some_and(|local| self.is_temporary_of_innermost_scope(local)) =>
            {
                operand
            }
            _ => {
                let local = self.new_temporary(ty, span);
                self.assign(Place::local(local), Rvalue::Use(operand), span);
                Operand::Move(Place::local(local))
            }
        }
    }

    /// The value of the omitted argument `index` of `call`: a call of its generator with shared
    /// borrows of the receiver and the earlier arguments, and, for a reference parameter, a
    /// mutable borrow of a `null` slot that lives until the end of the caller's temporary scope
    /// (D11).
    fn default_argument(
        &mut self,
        call: &Call,
        index: usize,
        operands: &[Option<Operand>],
        span: Span,
    ) -> Operand {
        let TreeCallee::Function(function) = &call.callee else {
            panic!("internal error: an omitted argument of a function value");
        };
        let (name, parameter_ty) = call.parameters[index].clone();
        let mut arguments = Vec::new();
        let mut parameters = Vec::new();
        for (earlier, operand) in operands.iter().enumerate().take(index) {
            let earlier_ty = call.parameters[earlier].1.clone();
            let place = match operand {
                Some(Operand::Move(place)) => place.clone(),
                _ if self.current.is_none() => return Operand::Constant(Constant::Unit),
                _ => panic!(
                    "internal error: argument {} is not held in a temporary",
                    earlier
                ),
            };
            let borrow = self.new_temporary(shared(earlier_ty.clone()), span);
            let rvalue = Rvalue::Borrow {
                mutable: false,
                origin: BorrowOrigin::Default,
                place,
            };
            self.assign(Place::local(borrow), rvalue, span);
            arguments.push(Operand::Move(Place::local(borrow)));
            parameters.push(shared(earlier_ty));
        }
        if let Some(slot_ty) = slot_type(&parameter_ty) {
            let slot = self.new_temporary(slot_ty.clone(), span);
            let null = Rvalue::Use(Operand::Constant(Constant::Null(slot_ty.clone())));
            self.assign(Place::local(slot), null, span);
            let slot_reference = Type::Reference {
                mutable: true,
                inner: Box::new(slot_ty),
            };
            let borrow = self.new_temporary(slot_reference.clone(), span);
            let rvalue = Rvalue::Borrow {
                mutable: true,
                origin: BorrowOrigin::Default,
                place: Place::local(slot),
            };
            self.assign(Place::local(borrow), rvalue, span);
            arguments.push(Operand::Move(Place::local(borrow)));
            parameters.push(slot_reference);
        }
        let result = self.new_temporary(parameter_ty, span);
        let callee = Callee::Default {
            function: function.clone(),
            parameter: index - usize::from(call.has_receiver),
            name: name.unwrap_or_default(),
        };
        self.emit_call(
            callee,
            arguments,
            parameters,
            Place::local(result),
            false,
            true,
            span,
        );
        Operand::Move(Place::local(result))
    }

    /// Evaluates the root temporary of a generator's borrowed default into the caller's slot
    /// `slot` (of type `R?`), returning the place the default borrows: the slot's payload, or
    /// the slot itself when the temporary is nullable (D11). The slot is `null` on entry, so it
    /// is written without a drop.
    pub(super) fn fill_slot(
        &mut self,
        slot: Place,
        value: ExpressionId,
        ty: &Type,
        span: Span,
    ) -> Place {
        if self.place_type(&slot) == *ty {
            self.expression_into(Some(&slot), value);
            slot
        } else {
            let operand = self.operand(value);
            self.assign(slot.clone(), Rvalue::WrapNullable(operand), span);
            slot.project(Projection::Payload)
        }
    }

    /// The temporary at the root of the place a default value borrows, under its coercions, a
    /// `BorrowContents` chain and its argument or written borrow (D11).
    fn slot_root(&self, default: ExpressionId) -> Option<ExpressionId> {
        let mut id = default;
        loop {
            match self.expression(id).kind {
                ExpressionKind::Coerce { value, .. } => id = value,
                ExpressionKind::SafeNavigation {
                    chain,
                    origin: SafeNavigationOrigin::BorrowContents,
                } => id = chain,
                ExpressionKind::Borrow {
                    origin: TreeBorrowOrigin::Argument | TreeBorrowOrigin::Written,
                    place,
                    ..
                } => {
                    id = place;
                    break;
                }
                _ => return None,
            }
        }
        loop {
            match self.expression(id).kind {
                ExpressionKind::Field { base, .. } | ExpressionKind::Index { base, .. } => {
                    id = base
                }
                ExpressionKind::NonNullPayload { nullable } => id = nullable,
                ExpressionKind::Temporary { .. } => return Some(id),
                _ => return None,
            }
        }
    }
}

impl<'a> Builder<'a> {
    /// The generator of the default value `default` of declared parameter `index` of
    /// `function`: its arguments are shared borrows of `this` and of each earlier parameter,
    /// whose bindings are their dereferences, plus the caller's slot for a reference parameter
    /// (D11).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lower_default(
        file: &'a TypedFile,
        types: &'a TypeTable,
        bodies: &'a mut BodySlots,
        function: &FunctionItem,
        id: BodyId,
        index: usize,
        receiver_count: usize,
        default: ExpressionId,
    ) -> Body {
        let tree = &file.bodies[function.body];
        let (name, parameter_ty) = function.parameters[index].clone();
        let span = tree.expressions[default].span;
        let owner = BodyOwner::Default {
            function: function.path.clone(),
            parameter: index,
            name,
        };
        let mut builder = Builder::new(file, types, bodies, tree, id, owner, span);
        builder.set_return_type(parameter_ty.clone(), span);
        for &parameter in &tree.parameters[..receiver_count + index] {
            let local = &tree.locals[parameter];
            let argument = builder.add_argument(
                shared(local.ty.clone()),
                Some(local.name.clone()),
                Some(parameter),
                local.span,
            );
            builder
                .bindings
                .insert(parameter, Self::deref(Place::local(argument)));
        }
        if let Some(slot_ty) = slot_type(&parameter_ty) {
            let slot_reference = Type::Reference {
                mutable: true,
                inner: Box::new(slot_ty),
            };
            let slot = builder.add_argument(slot_reference, None, None, span);
            if let Some(root) = builder.slot_root(default) {
                let slot = Self::deref(Place::local(slot));
                builder.preallocated.insert(root, Preallocated::Slot(slot));
            }
        }
        builder.lower_value(default);
        builder.finish()
    }
}
