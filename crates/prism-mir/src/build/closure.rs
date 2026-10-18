//! Closures (D13 of `mir-lowering`).
//!
//! Each closure is a body of its own, lowered from the closure's subtree in its parent's
//! arenas: its parameters are its arguments, and each binding it captures is a capture local,
//! which stands for the binding itself and is never marked or dropped. The enclosing body builds
//! the closure value from a move or copy of each binding the typed tree records as owned and the
//! place of each other one, whose borrow's kind is left to the borrow checker (D2 of
//! `escaping-closures`).
//!
//! A function name or variant constructor read as a value is an adapter: a closure without
//! captures that calls the function or builds the variant (D7 of `closures`).

use super::Builder;
use crate::mir::{
    AggregateKind, BodyOwner, Callee, Capture as MirCapture, LocalDeclaration, LocalId, LocalKind,
    Operand, Place, Rvalue,
};
use prism_ast::Span;
use prism_typed_tree::{
    CaptureMode as TreeCaptureMode, DeclarationPath, ExpressionId, ExpressionKind, NominalId, Type,
};

impl Builder<'_> {
    pub(super) fn lower_closure(&mut self, destination: Option<&Place>, id: ExpressionId) {
        let expression = self.expression(id);
        let ExpressionKind::Closure {
            parameters,
            body,
            is_compose,
            captures,
            ownership_reason,
        } = &expression.kind
        else {
            unreachable!("a closure node");
        };
        let (ty, span) = (expression.ty.clone(), expression.span);
        let Type::Fn { ret, .. } = &ty else {
            panic!("internal error: a closure of type {}", ty);
        };
        let index = self.closure_count;
        self.closure_count += 1;
        let closure_id = self.bodies.reserve();
        let owner = BodyOwner::Closure {
            parent: self.id,
            index,
        };
        let mut closure = Builder::new(
            self.file,
            self.types,
            &mut *self.bodies,
            self.tree,
            closure_id,
            owner,
            span,
        );
        closure.is_compose = *is_compose;
        closure.type_parameters = self.type_parameters.clone();
        closure.ownership_reason = ownership_reason.clone();
        closure.set_return_type(ret.as_ref().clone(), span);
        for &parameter in parameters {
            closure.bind_argument(parameter);
        }
        for capture in captures {
            let binding = &self.tree.locals[capture.local];
            let local = closure.add_local(LocalDeclaration {
                ty: binding.ty.clone(),
                kind: LocalKind::Capture,
                mutable: binding.mutable,
                name: Some(binding.name.clone()),
                binding: Some(capture.local),
                escaping: false,
                span: capture.span,
            });
            closure.capture_count += 1;
            closure.bindings.insert(capture.local, Place::local(local));
        }
        closure.lower_value(*body);
        let lowered = closure.finish();
        self.bodies.fill(closure_id, lowered);
        // A borrowed capture is its binding's place; an owned one moves or copies it out (D2 of
        // `escaping-closures`), which the enclosing body then no longer drops.
        let captures = captures
            .iter()
            .map(|capture| {
                let place = match self.bindings.get(&capture.local) {
                    Some(place) => place.clone(),
                    None => panic!(
                        "internal error: captured binding {} has no place",
                        capture.local
                    ),
                };
                match capture.mode {
                    TreeCaptureMode::Borrowed => MirCapture::Borrowed(place),
                    TreeCaptureMode::Copied => {
                        MirCapture::Owned(Operand::Copy(place, capture.span))
                    }
                    TreeCaptureMode::Moved => {
                        let operand = Operand::Move(place, capture.span);
                        self.record_operand_moved(&operand);
                        MirCapture::Owned(operand)
                    }
                }
            })
            .collect();
        let destination = self.destination_or_temporary(destination, &ty, span);
        let rvalue = Rvalue::Closure {
            body: closure_id,
            captures,
        };
        self.assign(destination, rvalue, span);
    }
}

/// What an adapter closure calls or builds (D7 of `closures`).
enum AdapterTarget {
    /// A top-level function.
    Function(DeclarationPath),
    /// An external function, called by symbol.
    External(DeclarationPath),
    /// A variant of an enum, with the enum's type arguments.
    Variant {
        enum_id: NominalId,
        index: usize,
        arguments: Vec<Type>,
    },
}

impl Builder<'_> {
    /// Lowers the function name or variant constructor `reference`, read as a value of the
    /// function type `value_type`, into `destination` as an adapter (D7 of `closures`): a
    /// closure without captures, the next closure of this body, whose arguments have the
    /// parameter types of `value_type` and whose body calls the function, or builds the
    /// variant, with each argument converted to its declared parameter type, and returns the
    /// result converted to the result type of `value_type`. MIR has no other function value.
    pub(super) fn lower_adapter(
        &mut self,
        destination: Option<&Place>,
        reference: ExpressionId,
        value_type: &Type,
        span: Span,
    ) {
        let Some(destination) = destination else {
            // A discarded function name has no effect.
            return;
        };
        let expression = self.expression(reference);
        let Type::Fn {
            params: declared_parameters,
            ret: declared_result,
            is_compose: declared_compose,
            escaping: declared_escaping,
        } = &expression.ty
        else {
            panic!("internal error: a function value of type {}", expression.ty);
        };
        let target = match &expression.kind {
            // An external has no slot: its adapter calls it by symbol (D5 of
            // `standard-library`).
            ExpressionKind::FunctionReference(path) if self.file.externals.contains(path) => {
                AdapterTarget::External(path.clone())
            }
            ExpressionKind::FunctionReference(path) => AdapterTarget::Function(path.clone()),
            ExpressionKind::VariantConstructor { enum_id, index, .. } => {
                let arguments = match declared_result.as_ref() {
                    Type::Enum { arguments, .. } => arguments.clone(),
                    _ => Vec::new(),
                };
                AdapterTarget::Variant {
                    enum_id: enum_id.clone(),
                    index: *index,
                    arguments,
                }
            }
            _ => unreachable!("an adapter of a function name or variant constructor"),
        };
        let Type::Fn {
            params: parameters,
            ret: result,
            is_compose,
            escaping: value_escaping,
        } = value_type
        else {
            panic!("internal error: an adapter of type {}", value_type);
        };
        let index = self.closure_count;
        self.closure_count += 1;
        let closure_id = self.bodies.reserve();
        let owner = BodyOwner::Closure {
            parent: self.id,
            index,
        };
        let mut closure = Builder::new(
            self.file,
            self.types,
            &mut *self.bodies,
            self.tree,
            closure_id,
            owner,
            span,
        );
        closure.is_compose = *is_compose;
        closure.type_parameters = self.type_parameters.clone();
        closure.set_return_type(result.as_ref().clone(), span);
        let arguments: Vec<LocalId> = parameters
            .iter()
            .zip(value_escaping)
            .map(|(parameter, &escaping)| {
                let argument = closure.add_argument(parameter.clone(), None, None, span);
                closure.locals[argument].escaping = escaping;
                argument
            })
            .collect();
        closure.push_scope(span);
        let operands: Vec<Operand> = arguments
            .iter()
            .zip(parameters.iter().zip(declared_parameters))
            .map(|(&argument, (from, to))| {
                let operand = if closure.is_copy(from) {
                    Operand::Copy(Place::local(argument), span)
                } else {
                    Operand::Move(Place::local(argument), span)
                };
                closure.convert_operand(operand, from, to, span)
            })
            .collect();
        let converts_result = declared_result != result;
        let value = if converts_result {
            Place::local(closure.new_temporary(declared_result.as_ref().clone(), span))
        } else {
            Place::return_place()
        };
        match target {
            AdapterTarget::External(path) => closure.emit_call(
                Callee::External(path),
                operands,
                declared_parameters.clone(),
                declared_escaping.clone(),
                Vec::new(),
                value.clone(),
                *declared_compose,
                **declared_result != Type::Nothing,
                span,
            ),
            AdapterTarget::Function(path) => closure.emit_call(
                Callee::Function(path),
                operands,
                declared_parameters.clone(),
                declared_escaping.clone(),
                Vec::new(),
                value.clone(),
                *declared_compose,
                **declared_result != Type::Nothing,
                span,
            ),
            AdapterTarget::Variant {
                enum_id,
                index,
                arguments,
            } => {
                let kind = AggregateKind::Variant {
                    enum_id,
                    index,
                    arguments,
                };
                closure.assign(value.clone(), Rvalue::Aggregate(kind, operands), span);
            }
        }
        if converts_result && closure.current.is_some() {
            let operand = if closure.is_copy(declared_result) {
                Operand::Copy(value, span)
            } else {
                Operand::Move(value, span)
            };
            let converted = closure.convert_operand(operand, declared_result, result, span);
            closure.assign(Place::return_place(), Rvalue::Use(converted), span);
        }
        closure.pop_scope();
        if closure.current.is_some() {
            closure.emit_return();
        }
        let lowered = closure.finish();
        self.bodies.fill(closure_id, lowered);
        let rvalue = Rvalue::Closure {
            body: closure_id,
            captures: Vec::new(),
        };
        self.assign(destination.clone(), rvalue, span);
    }

    /// `operand`, of type `from`, converted to `to` as a value conversion lowers it: unchanged
    /// when the types are equal, wrapped for `T` to `T?`, and any other step (a reference
    /// weakening, a function value converting in place) as a `Subtype` into a temporary.
    fn convert_operand(&mut self, operand: Operand, from: &Type, to: &Type, span: Span) -> Operand {
        if from == to || *from == Type::Error || *to == Type::Error {
            return operand;
        }
        let rvalue = match to {
            Type::Nullable(inner) if !matches!(from, Type::Nullable(_)) => {
                Rvalue::WrapNullable(self.convert_operand(operand, from, inner, span))
            }
            _ => Rvalue::Subtype(operand),
        };
        let local = self.new_temporary(to.clone(), span);
        self.assign(Place::local(local), rvalue, span);
        Operand::Move(Place::local(local), span)
    }
}
