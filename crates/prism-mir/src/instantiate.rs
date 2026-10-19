//! Instances of generic bodies: substitution over elaborated MIR (D6 of `monomorphization`).
//!
//! [`instantiate`] copies a generic body with its type arguments substituted for its type
//! parameters in every local, aggregate, `null` constant and call type argument. A call of a
//! generic keeps its callee's declared parameter types, which mention the callee's own
//! parameters; the call's now concrete type arguments are substituted into them where an
//! operand's type is needed. The parameter types of any other call (a built-in's, such as
//! `clone()`'s `&T`, or a function value's) mention the body's own parameters, and are
//! substituted. A
//! drop whose type no longer needs one becomes a jump. Where a type parameter `T` has a nullable
//! argument `U?`, `T?` is `U?` itself (D4 of `generics`), so the body is **flattened**: wrapping a
//! `T` into a `T?` becomes a plain use, and the payload of a `T?` place is the place itself. Only
//! a type parameter can become nullable by substitution, and a nullable is formed only by `?`
//! around a type, so no other construct flattens. The instance is never borrow-checked again:
//! the polymorphic check is exact for every instantiation (D7 of `generics`). One pass over the
//! body.

use crate::mir::{
    AggregateKind, BasicBlock, Body, Callee, Capture, Constant, Operand, Place, Projection, Rvalue,
    Statement, StatementKind, TemplatePart, Terminator, TerminatorKind,
};
use crate::types::{needs_drop, projection_type};
use prism_typed_tree::{Type, TypeTable};

/// The instance of the generic `body` for `type_arguments`, one per type parameter of the body,
/// over `types`, a table holding the definitions of the body's types and of its arguments'.
pub fn instantiate(body: &Body, type_arguments: &[Type], types: &TypeTable) -> Body {
    let instance = Instance {
        body,
        arguments: type_arguments,
        types,
    };
    let mut result = body.clone();
    for (_, local) in result.locals.iter_mut() {
        local.ty = local.ty.substitute(type_arguments);
    }
    let blocks: Vec<BasicBlock> = body
        .blocks
        .iter()
        .map(|(_, block)| BasicBlock {
            statements: block
                .statements
                .iter()
                .map(|statement| Statement {
                    kind: instance.statement(&statement.kind),
                    span: statement.span,
                })
                .collect(),
            terminator: Terminator {
                kind: instance.terminator(&block.terminator.kind, &result),
                span: block.terminator.span,
            },
        })
        .collect();
    for ((_, block), instantiated) in result.blocks.iter_mut().zip(blocks) {
        *block = instantiated;
    }
    result.type_parameters = Vec::new();
    result.type_arguments = type_arguments.to_vec();
    result
}

struct Instance<'a> {
    /// The polymorphic body.
    body: &'a Body,
    arguments: &'a [Type],
    types: &'a TypeTable,
}

impl Instance<'_> {
    fn substitute(&self, ty: &Type) -> Type {
        ty.substitute(self.arguments)
    }

    /// Whether the polymorphic type `ty` is a type parameter whose argument is nullable, so that
    /// `ty?` flattens to the argument.
    fn flattens(&self, ty: &Type) -> bool {
        match ty {
            Type::Parameter(parameter) => matches!(
                self.arguments.get(parameter.index as usize),
                Some(Type::Nullable(_))
            ),
            _ => false,
        }
    }

    /// `place` with the payload projections of flattened nullables removed.
    fn place(&self, place: &Place) -> Place {
        if place.projections.is_empty() {
            return place.clone();
        }
        let mut ty = self
            .body
            .locals
            .get(place.local)
            .map(|local| local.ty.clone());
        let mut projections = Vec::with_capacity(place.projections.len());
        for projection in &place.projections {
            let next = ty
                .as_ref()
                .and_then(|base| projection_type(base, *projection, self.types));
            let flattened = *projection == Projection::Payload
                && matches!(&ty, Some(Type::Nullable(inner)) if self.flattens(inner));
            if !flattened {
                projections.push(*projection);
            }
            ty = next;
        }
        Place {
            local: place.local,
            projections,
        }
    }

    fn operand(&self, operand: &Operand) -> Operand {
        match operand {
            Operand::Copy(place, span) => Operand::Copy(self.place(place), *span),
            Operand::Move(place, span) => Operand::Move(self.place(place), *span),
            Operand::Constant(Constant::Null(ty)) => {
                Operand::Constant(Constant::Null(self.substitute(ty)))
            }
            Operand::Constant(constant) => Operand::Constant(constant.clone()),
        }
    }

    /// The polymorphic type of `operand`, if it reads a place.
    fn polymorphic_type(&self, operand: &Operand) -> Option<Type> {
        operand.place()?.try_ty(self.body, self.types)
    }

    fn rvalue(&self, rvalue: &Rvalue) -> Rvalue {
        match rvalue {
            Rvalue::Use(operand) => Rvalue::Use(self.operand(operand)),
            Rvalue::Borrow {
                mutable,
                origin,
                place,
            } => Rvalue::Borrow {
                mutable: *mutable,
                origin: *origin,
                place: self.place(place),
            },
            Rvalue::Binary(operator, left, right) => {
                Rvalue::Binary(*operator, self.operand(left), self.operand(right))
            }
            Rvalue::Bits(operator, left, right) => {
                Rvalue::Bits(*operator, self.operand(left), self.operand(right))
            }
            Rvalue::Unary(operator, operand) => Rvalue::Unary(*operator, self.operand(operand)),
            Rvalue::Discriminant(place) => Rvalue::Discriminant(self.place(place)),
            Rvalue::Aggregate(kind, operands) => Rvalue::Aggregate(
                match kind {
                    AggregateKind::Struct { id, arguments } => AggregateKind::Struct {
                        id: id.clone(),
                        arguments: arguments.iter().map(|ty| self.substitute(ty)).collect(),
                    },
                    AggregateKind::Variant {
                        enum_id,
                        index,
                        arguments,
                    } => AggregateKind::Variant {
                        enum_id: enum_id.clone(),
                        index: *index,
                        arguments: arguments.iter().map(|ty| self.substitute(ty)).collect(),
                    },
                },
                operands
                    .iter()
                    .map(|operand| self.operand(operand))
                    .collect(),
            ),
            Rvalue::WrapNullable(operand) => {
                // Wrapping a `T` whose argument is nullable is the value itself.
                let flattens = self
                    .polymorphic_type(operand)
                    .is_some_and(|ty| self.flattens(&ty));
                if flattens {
                    Rvalue::Use(self.operand(operand))
                } else {
                    Rvalue::WrapNullable(self.operand(operand))
                }
            }
            Rvalue::Subtype(operand) => Rvalue::Subtype(self.operand(operand)),
            Rvalue::Closure { body, captures } => Rvalue::Closure {
                body: *body,
                captures: captures
                    .iter()
                    .map(|capture| match capture {
                        Capture::Borrowed(place) => Capture::Borrowed(self.place(place)),
                        Capture::Owned(operand) => Capture::Owned(self.operand(operand)),
                    })
                    .collect(),
            },
            Rvalue::Template(parts) => Rvalue::Template(
                parts
                    .iter()
                    .map(|part| match part {
                        TemplatePart::Text(text) => TemplatePart::Text(text.clone()),
                        TemplatePart::Value(operand) => TemplatePart::Value(self.operand(operand)),
                    })
                    .collect(),
            ),
        }
    }

    fn statement(&self, statement: &StatementKind) -> StatementKind {
        match statement {
            StatementKind::Assign(place, rvalue) => {
                StatementKind::assign(self.place(place), self.rvalue(rvalue))
            }
            StatementKind::Read(place) => StatementKind::Read(self.place(place)),
            StatementKind::StorageLive(_)
            | StatementKind::StorageDead(_)
            | StatementKind::Destructure { .. }
            | StatementKind::GroupStart(_)
            | StatementKind::GroupEnd(_) => statement.clone(),
        }
    }

    /// The instance of a composer operation: its slot type substituted.
    fn operation(
        &self,
        operation: &crate::mir::ComposerOperation,
    ) -> crate::mir::ComposerOperation {
        use crate::mir::ComposerOperation as Operation;
        match operation {
            Operation::Changed(ty) => Operation::Changed(self.substitute(ty)),
            Operation::SetSlot(ty) => Operation::SetSlot(self.substitute(ty)),
            Operation::Keep(ty) => Operation::Keep(self.substitute(ty)),
            Operation::KeptParameter { index, ty } => Operation::KeptParameter {
                index: *index,
                ty: self.substitute(ty),
            },
            other => other.clone(),
        }
    }

    /// The instance of `terminator`, where `instance` holds the substituted locals.
    fn terminator(&self, terminator: &TerminatorKind, instance: &Body) -> TerminatorKind {
        match terminator {
            TerminatorKind::SwitchInt {
                value,
                targets,
                otherwise,
            } => TerminatorKind::SwitchInt {
                value: self.operand(value),
                targets: targets.clone(),
                otherwise: *otherwise,
            },
            TerminatorKind::Call {
                callee,
                arguments,
                parameters,
                escaping,
                type_arguments,
                destination,
                target,
                is_compose,
            } => TerminatorKind::Call {
                escaping: escaping.clone(),
                callee: match callee {
                    Callee::Value(place, span) => Callee::Value(self.place(place), *span),
                    // A composed generic's slot types mention its type parameters (D21 of
                    // `compose-transform`).
                    Callee::Composer(operation) => Callee::Composer(self.operation(operation)),
                    other => other.clone(),
                },
                arguments: arguments
                    .iter()
                    .map(|operand| self.operand(operand))
                    .collect(),
                // A generic callee's declared types mention its own parameters, which the call's
                // type arguments stand for. Any other callee's parameter types (a built-in's or
                // a function value's) are the caller's own, so they are substituted.
                parameters: if type_arguments.is_empty() {
                    parameters.iter().map(|ty| self.substitute(ty)).collect()
                } else {
                    parameters.clone()
                },
                type_arguments: type_arguments
                    .iter()
                    .map(|ty| self.substitute(ty))
                    .collect(),
                destination: self.place(destination),
                target: *target,
                is_compose: *is_compose,
            },
            TerminatorKind::Drop { place, target } => {
                let place = self.place(place);
                let drops = place
                    .try_ty(instance, self.types)
                    .is_none_or(|ty| needs_drop(&ty, self.types));
                if drops {
                    TerminatorKind::Drop {
                        place,
                        target: *target,
                    }
                } else {
                    TerminatorKind::Goto(*target)
                }
            }
            TerminatorKind::Remembered {
                composer,
                slot_type,
                destination,
                found,
                missing,
            } => TerminatorKind::Remembered {
                composer: self.operand(composer),
                slot_type: self.substitute(slot_type),
                destination: self.place(destination),
                found: *found,
                missing: *missing,
            },
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {
                terminator.clone()
            }
        }
    }
}
