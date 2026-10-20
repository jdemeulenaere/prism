//! Operators over borrowed operands, and optional reborrows (D12 and D14 of `mir-lowering`).
//!
//! An operator over a type that is not `Copy` (`==`, `!=`, string concatenation), and a string
//! template's `String` parts, read shared borrows of their operands (`Operator` origin), nullable ones as optional borrows `(&T)?`, so no
//! operand is moved or copied; a comparison with `null` reads the tag. A `(&mut T)?` place lent as a `(&T)?` or as a
//! `(&mut T)?` is reborrowed through a tag test.

use super::Builder;
use crate::mir::{BorrowOrigin, Constant, Operand, Place, Projection, Rvalue, TemplatePart};
use prism_ast::Span;
use prism_typed_tree::{BinaryOperator, CoercionKind, ExpressionId, ExpressionKind, Type};

/// The subject of a `when` with value patterns: its place and, when its type is not `Copy`, the
/// place holding its (optional) borrow, evaluated once (D14).
#[derive(Debug, Clone)]
pub(super) struct ComparisonSubject {
    place: Place,
    view: Option<Place>,
    /// The subject's own span, where each pattern reads it.
    span: Span,
}

impl ComparisonSubject {
    /// The subject's own place.
    pub(super) fn subject_place(&self) -> &Place {
        &self.place
    }
}

fn shared(ty: Type) -> Type {
    reference(ty, false)
}

fn reference(ty: Type, mutable: bool) -> Type {
    Type::Reference {
        mutable,
        inner: Box::new(ty),
    }
}

impl Builder<'_> {
    /// A binary operator into `destination`: over `Copy` operands read or consumed, over other
    /// operands through their borrows, and a comparison with `null` as a tag read (D14).
    pub(super) fn lower_binary(
        &mut self,
        destination: Option<&Place>,
        operator: BinaryOperator,
        left: ExpressionId,
        right: ExpressionId,
        ty: &Type,
        span: Span,
    ) {
        let destination = self.destination_or_temporary(destination, ty, span);
        let is_equality = matches!(operator, BinaryOperator::Equal | BinaryOperator::NotEqual);
        if is_equality && (self.is_null(left) || self.is_null(right)) {
            let other = if self.is_null(right) { left } else { right };
            let other_span = self.expression(other).span;
            let place = self.place(other);
            self.compare_tag(&destination, operator, place, other_span, span);
            return;
        }
        let operand_ty = self.expression(left).ty.clone();
        let (left, right) = if self.is_copy(&operand_ty) {
            let mut operands = self.ordered_operands(&[left, right]).into_iter();
            (operands.next().unwrap(), operands.next().unwrap())
        } else {
            (
                self.comparison_operand(left),
                self.comparison_operand(right),
            )
        };
        self.assign(destination, Rvalue::Binary(operator, left, right), span);
    }

    /// A string template into `destination` (D10 and D11 of `string-templates`): its value
    /// parts evaluated left to right, each `String` as a shared `Operator` borrow of its place
    /// (a computed one a temporary of the statement), each scalar as an operand of its type,
    /// held in a temporary when a later part may write the place it reads, then one
    /// [`Rvalue::Template`] over the text and those operands.
    pub(super) fn lower_template(
        &mut self,
        destination: Option<&Place>,
        parts: &[prism_typed_tree::TemplatePart],
        span: Span,
    ) {
        let destination = self.destination_or_temporary(destination, &Type::String, span);
        let values: Vec<ExpressionId> = parts
            .iter()
            .filter_map(|part| match part {
                prism_typed_tree::TemplatePart::Value(value) => Some(*value),
                prism_typed_tree::TemplatePart::Text(_) => None,
            })
            .collect();
        let later_may_write = self.later_may_write(&values);
        let mut position = 0;
        let mut lowered = Vec::with_capacity(parts.len());
        for part in parts {
            let value = match part {
                prism_typed_tree::TemplatePart::Text(text) => {
                    lowered.push(TemplatePart::Text(text.clone()));
                    continue;
                }
                prism_typed_tree::TemplatePart::Value(value) => *value,
            };
            let expression = self.expression(value);
            let (ty, value_span) = (expression.ty.clone(), expression.span);
            let operand = if ty == Type::String {
                self.comparison_operand(value)
            } else {
                let operand = self.operand(value);
                if later_may_write[position] {
                    self.stabilize(operand, ty, value_span)
                } else {
                    operand
                }
            };
            position += 1;
            lowered.push(TemplatePart::Value(operand));
        }
        self.assign(destination, Rvalue::Template(lowered), span);
    }

    /// `target op= value` (D5 of `compound-assignment`): the target's place, evaluated once, is
    /// read (copied for a `Copy` type, held in a temporary when the value may write it, otherwise
    /// borrowed for the operator), then the value is evaluated and the operator applied. A `Copy`
    /// result is written to the place directly; any other result goes through a temporary, so
    /// the old value can be dropped before the result is moved in. The read and the write are at
    /// the target's span, like the write of `=` (D6 of `borrowck-exclusivity`).
    pub(super) fn lower_compound_assign(
        &mut self,
        operator: BinaryOperator,
        target: ExpressionId,
        value: ExpressionId,
        span: Span,
    ) {
        let target_expression = self.expression(target);
        let (ty, target_span) = (target_expression.ty.clone(), target_expression.span);
        if let ExpressionKind::StatePlace { handle } = target_expression.kind {
            return self.lower_state_compound_assign(operator, handle, value, target_span, span);
        }
        // A part of a state's value is changed under an exclusive guard (D3 of `owned-state`).
        let mut place = self.place_with_access(target, super::state::StateAccess::Exclusive);
        if self.may_repoint(target, value) {
            place = self.pin_referent(target, place, target_span);
        }
        let (left, right) = if self.is_copy(&ty) {
            let mut left = Operand::Copy(place.clone(), target_span);
            if self.may_write(value) {
                left = self.stabilize(left, ty.clone(), target_span);
            }
            (left, self.operand(value))
        } else {
            let borrow = self.new_temporary(shared(ty.clone()), target_span);
            let rvalue = Rvalue::Borrow {
                mutable: false,
                origin: BorrowOrigin::Operator,
                place: place.clone(),
            };
            self.assign(Place::local(borrow), rvalue, target_span);
            let left = Operand::Move(Place::local(borrow), target_span);
            (left, self.comparison_operand(value))
        };
        if self.current.is_none() {
            return;
        }
        let rvalue = Rvalue::Binary(operator, left, right);
        if self.needs_drop(&ty) {
            let result = self.new_temporary(ty, span);
            self.assign(Place::local(result), rvalue, span);
            self.emit_drop(place.clone(), span);
            let moved = Operand::Move(Place::local(result), span);
            self.assign(place, Rvalue::Use(moved), target_span);
        } else {
            self.assign(place, rvalue, target_span);
        }
    }

    /// `place`, the place of the compound assignment target `target`, reached through a mutable
    /// borrow of the referent of its last dereference, held in a temporary: `(*_t).n` for
    /// `(*_1).n`. The place is then evaluated once even when the value re-points the reference
    /// it is reached through (`*r += if (c) { r = &mut b; 1 } else 1` writes what `r` referred
    /// to when the statement started, D2 of `compound-assignment`).
    fn pin_referent(&mut self, target: ExpressionId, place: Place, span: Span) -> Place {
        let Some(referent) = self.outermost_dereference_type(target) else {
            return place;
        };
        let Some(position) = place
            .projections
            .iter()
            .rposition(|p| *p == Projection::Deref)
        else {
            return place;
        };
        let reference = Type::Reference {
            mutable: true,
            inner: Box::new(referent),
        };
        let pointer = self.new_temporary(reference, span);
        let rvalue = Rvalue::Borrow {
            mutable: true,
            origin: BorrowOrigin::Operator,
            place: Place {
                local: place.local,
                projections: place.projections[..=position].to_vec(),
            },
        };
        self.assign(Place::local(pointer), rvalue, span);
        let mut pinned = Self::deref(Place::local(pointer));
        pinned
            .projections
            .extend_from_slice(&place.projections[position + 1..]);
        pinned
    }

    /// The type of the outermost dereference along the place expression `id` (the referent
    /// last reached through a reference), if it has one.
    fn outermost_dereference_type(&self, id: ExpressionId) -> Option<Type> {
        let expression = self.expression(id);
        match &expression.kind {
            ExpressionKind::Deref { .. } => Some(expression.ty.clone()),
            ExpressionKind::Field { base, .. } => self.outermost_dereference_type(*base),
            _ => None,
        }
    }

    /// `destination = discriminant(place) <operator> 0`: a comparison with `null`, whose tag is
    /// read at the span of the compared value, `place_span`.
    pub(super) fn compare_tag(
        &mut self,
        destination: &Place,
        operator: BinaryOperator,
        place: Place,
        place_span: Span,
        span: Span,
    ) {
        let tag = self.new_temporary(Type::Int, place_span);
        self.assign(Place::local(tag), Rvalue::Discriminant(place), place_span);
        let rvalue = Rvalue::Binary(
            operator,
            Operand::Move(Place::local(tag), place_span),
            Operand::Constant(Constant::Int(0)),
        );
        self.assign(destination.clone(), rvalue, span);
    }

    /// The operand comparing the non-`Copy` value `id`: a shared borrow of it, or for a
    /// nullable an optional borrow of its payload, taken at the value's own span. A place is
    /// borrowed where it lives, a `?.` chain ending in a place inside the chain, and any other
    /// value in a temporary.
    fn comparison_operand(&mut self, id: ExpressionId) -> Operand {
        let expression = self.expression(id);
        let (ty, span) = (expression.ty.clone(), expression.span);
        match &ty {
            Type::Nullable(inner) => {
                let view_ty = shared(inner.as_ref().clone()).nullable();
                let view = self.new_temporary(view_ty.clone(), span);
                if let ExpressionKind::Coerce {
                    value,
                    kind: CoercionKind::WrapNullable,
                } = expression.kind
                {
                    // A value that is never `null`, compared with a nullable one: its borrow,
                    // wrapped, with no tag to test. A place is borrowed where it lives.
                    let place = self.place(value);
                    let borrow = self.new_temporary(shared(inner.as_ref().clone()), span);
                    let rvalue = Rvalue::Borrow {
                        mutable: false,
                        origin: BorrowOrigin::Operator,
                        place,
                    };
                    self.assign(Place::local(borrow), rvalue, span);
                    let wrap = Rvalue::WrapNullable(Operand::Move(Place::local(borrow), span));
                    self.assign(Place::local(view), wrap, span);
                    return Operand::Move(Place::local(view), span);
                }
                if let ExpressionKind::SafeNavigation { chain, .. } = expression.kind {
                    if self.expression(chain).kind.is_place() {
                        self.lower_chain(Some(&Place::local(view)), chain, &view_ty, true, span);
                        return Operand::Move(Place::local(view), span);
                    }
                }
                let place = self.place(id);
                self.optional_borrow(
                    &Place::local(view),
                    place,
                    false,
                    false,
                    BorrowOrigin::Operator,
                    span,
                );
                Operand::Move(Place::local(view), span)
            }
            _ => {
                let place = self.place(id);
                let borrow = self.new_temporary(shared(ty), span);
                let rvalue = Rvalue::Borrow {
                    mutable: false,
                    origin: BorrowOrigin::Operator,
                    place,
                };
                self.assign(Place::local(borrow), rvalue, span);
                Operand::Move(Place::local(borrow), span)
            }
        }
    }

    /// `destination = place == null ? null : wrap &(payload)` (`&mut` with `mutable`), where
    /// the payload is dereferenced first with `through_reference`: an optional borrow of the
    /// contents of the nullable `place`, which is neither moved nor copied.
    fn optional_borrow(
        &mut self,
        destination: &Place,
        place: Place,
        through_reference: bool,
        mutable: bool,
        origin: BorrowOrigin,
        span: Span,
    ) {
        if self.current.is_none() {
            return;
        }
        let (present, absent) = self.switch_on_tag(place.clone(), span);
        self.current = Some(present);
        self.push_scope(span);
        let mut payload = place.project(Projection::Payload);
        if through_reference {
            payload = Self::deref(payload);
        }
        let referent = self.place_type(&payload);
        let borrow = self.new_temporary(reference(referent, mutable), span);
        let rvalue = Rvalue::Borrow {
            mutable,
            origin,
            place: payload,
        };
        self.assign(Place::local(borrow), rvalue, span);
        let wrap = Rvalue::WrapNullable(Operand::Move(Place::local(borrow), span));
        self.assign(destination.clone(), wrap, span);
        self.pop_scope();
        let present_end = self.current;
        self.current = Some(absent);
        let null_ty = self.place_type(destination);
        let null = Rvalue::Use(Operand::Constant(Constant::Null(null_ty)));
        self.assign(destination.clone(), null, span);
        let absent_end = self.current;
        self.join(&[present_end, absent_end], span);
    }

    /// A `(&mut T)?` place read in place as a `(&T)?`: its tag is read, and `null` or a shared
    /// argument borrow of the dereferenced payload, wrapped, is passed; the place is not moved
    /// (D12).
    pub(super) fn lower_reborrow(
        &mut self,
        destination: Option<&Place>,
        value: ExpressionId,
        ty: &Type,
        span: Span,
    ) {
        let value_ty = self.expression(value).ty.clone();
        let is_optional_reference = matches!(
            &value_ty,
            Type::Nullable(inner) if matches!(inner.as_ref(), Type::Reference { .. })
        );
        if !is_optional_reference {
            panic!(
                "internal error: a {} place read in place is converted to a {}",
                value_ty, ty
            );
        }
        let place = self.place(value);
        let destination = self.destination_or_temporary(destination, ty, span);
        self.optional_borrow(
            &destination,
            place,
            true,
            false,
            BorrowOrigin::Argument,
            span,
        );
    }

    /// The `(&mut T)?` place `place` lent as a `(&mut T)?`: its tag is read, and `null` or a
    /// mutable argument borrow of the dereferenced payload, wrapped, is written to
    /// `destination`; the place is not moved (D13 of `borrowck-elision`).
    pub(super) fn lower_optional_reborrow(
        &mut self,
        destination: &Place,
        place: Place,
        span: Span,
    ) {
        self.optional_borrow(destination, place, true, true, BorrowOrigin::Argument, span);
    }

    /// The subject of a `when` compared by its value patterns: for a type that is not `Copy`,
    /// its (optional) borrow is evaluated once (D14).
    pub(super) fn comparison_subject(
        &mut self,
        place: Place,
        ty: Type,
        compares_values: bool,
        span: Span,
    ) -> ComparisonSubject {
        let view = if compares_values && !self.is_copy(&ty) {
            let view = match &ty {
                Type::Nullable(inner) => {
                    let view_ty = shared(inner.as_ref().clone()).nullable();
                    let view = self.new_temporary(view_ty, span);
                    self.optional_borrow(
                        &Place::local(view),
                        place.clone(),
                        false,
                        false,
                        BorrowOrigin::Operator,
                        span,
                    );
                    view
                }
                _ => {
                    let borrow = self.new_temporary(shared(ty.clone()), span);
                    let rvalue = Rvalue::Borrow {
                        mutable: false,
                        origin: BorrowOrigin::Operator,
                        place: place.clone(),
                    };
                    self.assign(Place::local(borrow), rvalue, span);
                    borrow
                }
            };
            Some(Place::local(view))
        } else {
            None
        };
        ComparisonSubject { place, view, span }
    }

    /// `result = subject == value`, for a value pattern of a `when`.
    pub(super) fn equality_with_subject(
        &mut self,
        result: &Place,
        subject: &ComparisonSubject,
        value: ExpressionId,
        span: Span,
    ) {
        if self.is_null(value) {
            let place = subject.place.clone();
            self.compare_tag(result, BinaryOperator::Equal, place, subject.span, span);
            return;
        }
        let (left, right) = match &subject.view {
            None => (
                Operand::Copy(subject.place.clone(), subject.span),
                self.operand(value),
            ),
            Some(view) => (
                Operand::Copy(view.clone(), subject.span),
                self.comparison_operand(value),
            ),
        };
        self.assign(
            result.clone(),
            Rvalue::Binary(BinaryOperator::Equal, left, right),
            span,
        );
    }
}
