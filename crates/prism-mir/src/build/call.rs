//! Calls and default values (D10 and D11 of `mir-lowering`).

use super::{BodySlots, Builder, Preallocated};
use crate::mir::{
    Body, BodyId, BodyOwner, BorrowOrigin, Callee, Constant, Operand, Place, Projection, Rvalue,
    TerminatorKind,
};
use prism_ast::{DefaultInputs, Span};
use prism_typed_tree::ParameterKind;
use prism_typed_tree::{
    Argument, BorrowOrigin as TreeBorrowOrigin, BuiltinFunction, Call, Callee as TreeCallee,
    ExpressionId, ExpressionKind, FunctionItem, SafeNavigationOrigin, Type, TypeTable, TypedFile,
};

/// A receiver that a call borrows: its place is evaluated before the written arguments and
/// borrowed after them (D10 of `mir-lowering`), unless a `?.` test reaches it, which is
/// borrowed right after the test (`borrowck-exclusivity`). A receiver that the call moves or copies is
/// evaluated as an ordinary argument, first (D10 of `borrowck-moves`).
struct PendingReceiver {
    mutable: bool,
    place: Place,
    /// The receiver's own span, where its borrow is taken.
    span: Span,
    /// The guard of the state's value the receiver is a part of, taken where it is borrowed,
    /// after the arguments (D3 of `owned-state`).
    guard: Option<super::state::DeferredGuard>,
}

/// Whether a value of `ty` may hold a borrow its call's operands lent: a reference, a function
/// value or a keeper, or a nullable or box of one.
fn may_hold_borrow(ty: &Type) -> bool {
    match ty {
        Type::Reference { .. } | Type::Fn { .. } => true,
        Type::Nullable(inner) | Type::Heap(inner) => may_hold_borrow(inner),
        other => other.contains_keeper(),
    }
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

/// Whether a value of type `ty` can hold no borrow: no reference, function value or type
/// parameter, at any depth (struct and enum fields never hold borrows).
fn holds_no_borrow(ty: &Type) -> bool {
    match ty {
        Type::Int
        | Type::Float
        | Type::Bool
        | Type::Char
        | Type::String
        | Type::Unit
        | Type::Nothing
        | Type::Opaque(_) => true,
        Type::Nullable(inner) | Type::Heap(inner) | Type::List(inner) => holds_no_borrow(inner),
        Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
            arguments.iter().all(holds_no_borrow)
        }
        Type::Reference { .. } | Type::Fn { .. } | Type::Parameter(_) | Type::Error => false,
    }
}

/// The inputs the generator of a default of declared type `ty` takes, of the context, the
/// receiver and the earlier parameters: those its default reads (D26 of `ui-toolkit`), or all
/// of them for a parameter whose value may hold a borrow, since the elision rules tie such a
/// result to the borrows its generator is given. The caller and the generator both ask this,
/// from the same declared type and the same syntactic answer.
pub(super) fn generator_inputs(ty: &Type, inputs: DefaultInputs) -> DefaultInputs {
    if holds_no_borrow(ty) {
        inputs
    } else {
        DefaultInputs::ALL
    }
}

impl Builder<'_> {
    /// A call: the callee, then the receiver (moved or copied into a temporary when the call
    /// takes it by value, or only its place evaluated when the call borrows it, and borrowed
    /// then when that place is a payload a `?.` test reaches), the written arguments in source
    /// order (each in a temporary, except constants no default borrows), the borrowed
    /// receiver's borrow, the omitted arguments in parameter order by calling their
    /// generators, then the call (D10 of `mir-lowering` and of `borrowck-moves`).
    pub(super) fn lower_call(
        &mut self,
        destination: Option<&Place>,
        call: &Call,
        ty: &Type,
        span: Span,
    ) {
        if matches!(&call.callee, TreeCallee::Builtin(builtin) if builtin.is_state_operation()) {
            return self.lower_state_call(destination, call, ty, span);
        }
        // The guards of the states' values a call's operands borrow end with the call when its
        // result holds no borrow (D3 of `owned-state`), so `xs.value.add(xs.value.size)` takes
        // the shared guard of `size` and the exclusive one of `add` one after the other.
        if self.guards_end_with_call(call, ty) {
            let destination = self.destination_or_temporary(destination, ty, span);
            self.push_scope(span);
            self.lower_call_operands_and_call(Some(&destination), call, ty, span);
            self.pop_scope();
            return;
        }
        self.lower_call_operands_and_call(destination, call, ty, span)
    }

    /// Whether the guards a call's operands take end with the call: outside composable contexts,
    /// for a call whose result holds no borrow and one of whose operands reads a state's value.
    fn guards_end_with_call(&mut self, call: &Call, ty: &Type) -> bool {
        if self.is_compose || may_hold_borrow(ty) {
            return false;
        }
        call.arguments.iter().any(|argument| match argument {
            Argument::Written(value) => self.reads_state_place(*value),
            Argument::Default(_) => false,
        })
    }

    /// Whether the expression `id` reads a state place outside closures, memoized so that each
    /// node is visited once per body.
    fn reads_state_place(&mut self, id: ExpressionId) -> bool {
        if let Some(&known) = self.reads_state.get(&id) {
            return known;
        }
        let kind = self.expression(id).kind.clone();
        let found = match kind {
            ExpressionKind::StatePlace { .. } => true,
            ExpressionKind::Closure { .. } => false,
            kind => {
                let mut statements = Vec::new();
                kind.children(&mut statements)
                    .into_iter()
                    .any(|child| self.reads_state_place(child))
            }
        };
        self.reads_state.insert(id, found);
        found
    }

    /// [`Self::lower_call`] once the call's scope is decided.
    fn lower_call_operands_and_call(
        &mut self,
        destination: Option<&Place>,
        call: &Call,
        ty: &Type,
        span: Span,
    ) {
        let callee = match &call.callee {
            TreeCallee::Function(path) => Callee::Function(path.clone()),
            TreeCallee::External(path) => Callee::External(path.clone()),
            TreeCallee::Builtin(builtin) => Callee::Builtin(*builtin),
            TreeCallee::Value(value) => {
                Callee::Value(self.place(*value), self.expression(*value).span)
            }
        };
        let count = call.arguments.len();
        // What each omitted argument's generator borrows of the arguments before it.
        let generator_reads: Vec<(usize, DefaultInputs)> = call
            .arguments
            .iter()
            .enumerate()
            .filter_map(|(index, argument)| match argument {
                Argument::Default(inputs) => {
                    Some((index, generator_inputs(&call.parameters[index].1, *inputs)))
                }
                Argument::Written(_) => None,
            })
            .collect();
        let mut operands: Vec<Option<Operand>> = vec![None; count];
        let mut pending = None;
        let mut receiver_borrowed = false;
        // The receiver follows the context argument, if any (D9 of `receiver-lambdas`).
        let receiver_index = usize::from(call.has_context);
        // A receiver written after an argument (`a in b` is `b.contains(a)`, D4 of
        // `operator-conventions`) is evaluated and borrowed in its turn.
        let receiver_first = call
            .evaluation_order
            .iter()
            .find(|&&index| !(call.has_context && index == 0))
            == Some(&receiver_index);
        if call.has_receiver && receiver_first {
            if let Some(Argument::Written(receiver)) = call.arguments.get(receiver_index) {
                pending = self.receiver_place(*receiver);
                // A receiver reached through a `?.` test is borrowed right after its test, so
                // that no argument can change the value tested (`n?.push(clear(&mut n))`).
                if let Some(receiver) = pending
                    .take_if(|receiver| receiver.place.projections.contains(&Projection::Payload))
                {
                    operands[receiver_index] =
                        Some(self.borrow_receiver(call, receiver_index, receiver));
                    receiver_borrowed = true;
                }
            }
        }
        for &index in &call.evaluation_order {
            if index == receiver_index && (pending.is_some() || receiver_borrowed) {
                continue;
            }
            let Argument::Written(value) = call.arguments[index] else {
                panic!("internal error: call evaluates an omitted argument");
            };
            if call.has_receiver && !receiver_first && index == receiver_index {
                if let Some(receiver) = self.receiver_place(value) {
                    operands[index] = Some(self.borrow_receiver(call, index, receiver));
                    continue;
                }
            }
            let borrowed_by_default = generator_reads
                .iter()
                .any(|&(default, inputs)| default > index && inputs.reads(index));
            // A composable closure written as an argument is keyed by its call (D5 of
            // `compose-transform`).
            let callee_name = match &call.callee {
                TreeCallee::Function(path) | TreeCallee::External(path) => path.to_string(),
                TreeCallee::Builtin(builtin) => builtin.name().to_string(),
                TreeCallee::Value(_) => "<value>".to_string(),
            };
            self.record_closure_argument(value, &callee_name, index);
            operands[index] = Some(self.argument(value, borrowed_by_default));
        }
        if let Some(pending) = pending {
            operands[receiver_index] = Some(self.borrow_receiver(call, receiver_index, pending));
        }
        for &(index, inputs) in &generator_reads {
            let operand = self.default_argument(call, index, inputs, &operands, span);
            operands[index] = Some(operand);
        }
        if self.current.is_none() {
            return;
        }
        let arguments: Vec<Operand> = operands
            .into_iter()
            .map(|operand| operand.expect("every argument is evaluated"))
            .collect();
        // `a in r` on a `Range` is a test of the range's bounds, with no call (D8 of
        // `operator-conventions`).
        if let (TreeCallee::Builtin(BuiltinFunction::RangeContains), [range, element]) =
            (&call.callee, arguments.as_slice())
        {
            let destination = self.destination_or_temporary(destination, ty, span);
            let range_ty = call.parameters[0].1.clone();
            self.range_contains(destination, range.clone(), range_ty, element.clone(), span);
            return;
        }
        let parameters = call.parameters.iter().map(|(_, ty)| ty.clone()).collect();
        let destination = self.destination_or_temporary(destination, ty, span);
        self.emit_call(
            callee,
            arguments,
            parameters,
            call.parameter_kinds.clone(),
            call.type_arguments.clone(),
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
        parameter_kinds: Vec<ParameterKind>,
        type_arguments: Vec<Type>,
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
            parameter_kinds,
            type_arguments,
            destination,
            target,
            is_compose,
        };
        self.terminate(kind, span);
        self.current = target;
    }

    /// The borrow of the receiver `pending` of `call`, its argument `index`, in a temporary.
    fn borrow_receiver(&mut self, call: &Call, index: usize, pending: PendingReceiver) -> Operand {
        let receiver_ty = call
            .parameter_type(index)
            .expect("a receiver is the first parameter after the context");
        if let Some(guard) = pending.guard {
            self.take_deferred_guard(guard);
        }
        let local = self.new_temporary(receiver_ty, pending.span);
        let rvalue = Rvalue::Borrow {
            mutable: pending.mutable,
            origin: BorrowOrigin::Receiver,
            place: pending.place,
        };
        self.assign(Place::local(local), rvalue, pending.span);
        Operand::Move(Place::local(local), pending.span)
    }

    /// Evaluates the place of a receiver that is borrowed after the arguments, or returns `None`
    /// for a receiver evaluated as an ordinary argument: one the call moves or copies, which is
    /// moved or copied before the arguments (D10 of `borrowck-moves`).
    fn receiver_place(&mut self, receiver: ExpressionId) -> Option<PendingReceiver> {
        let span = self.expression(receiver).span;
        match self.expression(receiver).kind {
            ExpressionKind::Borrow {
                mutable,
                origin: TreeBorrowOrigin::Receiver,
                place,
            } => {
                // A state's guard is taken with the borrow, after the arguments, which may read
                // the state themselves.
                self.defer_guard = true;
                let place = self.place_with_access(
                    place,
                    if mutable {
                        super::state::StateAccess::Exclusive
                    } else {
                        super::state::StateAccess::Shared
                    },
                );
                self.defer_guard = false;
                Some(PendingReceiver {
                    mutable,
                    place,
                    span,
                    guard: self.deferred_guard.take(),
                })
            }
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

    /// The value of the omitted argument `index` of `call`: a call of its generator with shared
    /// borrows of the context, the receiver and the earlier arguments among `inputs`, and, for
    /// a reference parameter, a mutable borrow of a `null` slot that lives until the end of the
    /// caller's temporary scope (D11, and D26 of `ui-toolkit`).
    fn default_argument(
        &mut self,
        call: &Call,
        index: usize,
        inputs: DefaultInputs,
        operands: &[Option<Operand>],
        span: Span,
    ) -> Operand {
        let function = match &call.callee {
            TreeCallee::Function(function) => function.clone(),
            TreeCallee::Builtin(builtin) => builtin
                .declaration_with_defaults()
                .expect("internal error: an omitted argument of an intrinsic without defaults"),
            _ => panic!("internal error: an omitted argument of a function value"),
        };
        let function = &function;
        // The generator is as generic as its function: it is called with the call's type
        // arguments, its parameters are declared, and the caller's temporaries are substituted.
        let (name, declared_ty) = call.parameters[index].clone();
        let parameter_ty = declared_ty.substitute(&call.type_arguments);
        let mut arguments = Vec::new();
        let mut parameters = Vec::new();
        for (earlier, operand) in operands.iter().enumerate().take(index) {
            if !inputs.reads(earlier) {
                continue;
            }
            let declared_earlier = call.parameters[earlier].1.clone();
            let earlier_ty = declared_earlier.substitute(&call.type_arguments);
            let place = match operand {
                Some(Operand::Move(place, _)) => place.clone(),
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
            arguments.push(Operand::Move(Place::local(borrow), span));
            parameters.push(shared(declared_earlier));
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
            arguments.push(Operand::Move(Place::local(borrow), span));
            let declared_slot = slot_type(&declared_ty).expect("a reference parameter");
            parameters.push(Type::Reference {
                mutable: true,
                inner: Box::new(declared_slot),
            });
        }
        let result = self.new_temporary(parameter_ty, span);
        let callee = Callee::Default {
            function: function.clone(),
            parameter: index - usize::from(call.has_receiver) - usize::from(call.has_context),
            name: name.unwrap_or_default(),
        };
        let parameter_kinds = vec![ParameterKind::Plain; parameters.len()];
        self.emit_call(
            callee,
            arguments,
            parameters,
            parameter_kinds,
            call.type_arguments.clone(),
            Place::local(result),
            false,
            true,
            span,
        );
        Operand::Move(Place::local(result), span)
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
    /// `function`: its arguments are shared borrows of the context, `this` and the earlier
    /// parameters that the default reads ([`generator_inputs`]), whose bindings are their
    /// dereferences, plus the caller's slot for a reference parameter (D11, and D26 of
    /// `ui-toolkit`).
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
        builder.type_parameters = function.type_parameters.clone();
        builder.set_return_type(parameter_ty.clone(), span);
        let declared_inputs = function
            .default_inputs
            .get(index)
            .copied()
            .unwrap_or(DefaultInputs::ALL);
        let inputs = generator_inputs(&parameter_ty, declared_inputs);
        for (position, &parameter) in tree.parameters[..receiver_count + index].iter().enumerate() {
            if !inputs.reads(position) {
                continue;
            }
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
