//! Operators over borrowed operands, and optional reborrows (D12 and D14 of `mir-lowering`).
//!
//! An operator over a type that is not `Copy` (`==`, `!=`, string concatenation) reads shared
//! borrows of its operands (`Operator` origin), nullable ones as optional borrows `(&T)?`, so no
//! operand is moved or copied; a comparison with `null` reads the tag. A `(&mut T)?` place lent as a `(&T)?` is
//! reborrowed through a tag test.

use super::Builder;
use crate::mir::{BorrowOrigin, Constant, Operand, Place, Projection, Rvalue};
use prism_ast::Span;
use prism_typed_tree::{BinaryOperator, ExpressionId, ExpressionKind, Type};

/// The subject of a `when` with value patterns: its place and, when its type is not `Copy`, the
/// place holding its (optional) borrow, evaluated once (D14).
#[derive(Debug, Clone)]
pub(super) struct ComparisonSubject {
    place: Place,
    view: Option<Place>,
}

impl ComparisonSubject {
    /// The subject's own place.
    pub(super) fn subject_place(&self) -> &Place {
        &self.place
    }
}

fn shared(ty: Type) -> Type {
    Type::Reference {
        mutable: false,
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
            let place = self.place(other);
            self.compare_tag(&destination, operator, place, span);
            return;
        }
        let operand_ty = self.expression(left).ty.clone();
        let (left, right) = if self.is_copy(&operand_ty) {
            (self.operand(left), self.operand(right))
        } else {
            (
                self.comparison_operand(left, span),
                self.comparison_operand(right, span),
            )
        };
        self.assign(destination, Rvalue::Binary(operator, left, right), span);
    }

    /// `destination = discriminant(place) <operator> 0`: a comparison with `null`.
    fn compare_tag(
        &mut self,
        destination: &Place,
        operator: BinaryOperator,
        place: Place,
        span: Span,
    ) {
        let tag = self.new_temporary(Type::Int, span);
        self.assign(Place::local(tag), Rvalue::Discriminant(place), span);
        let rvalue = Rvalue::Binary(
            operator,
            Operand::Move(Place::local(tag)),
            Operand::Constant(Constant::Int(0)),
        );
        self.assign(destination.clone(), rvalue, span);
    }

    /// The operand comparing the non-`Copy` value `id`: a shared borrow of it, or for a
    /// nullable an optional borrow of its payload. A place is borrowed where it lives, a `?.`
    /// chain ending in a place inside the chain, and any other value in a temporary.
    fn comparison_operand(&mut self, id: ExpressionId, span: Span) -> Operand {
        let expression = self.expression(id);
        let ty = expression.ty.clone();
        match &ty {
            Type::Nullable(inner) => {
                let view_ty = shared(inner.as_ref().clone()).nullable();
                let view = self.new_temporary(view_ty.clone(), span);
                if let ExpressionKind::SafeNavigation { chain, .. } = expression.kind {
                    if self.expression(chain).kind.is_place() {
                        self.lower_chain(Some(&Place::local(view)), chain, &view_ty, true, span);
                        return Operand::Move(Place::local(view));
                    }
                }
                let place = self.place(id);
                self.optional_borrow(
                    &Place::local(view),
                    place,
                    false,
                    BorrowOrigin::Operator,
                    span,
                );
                Operand::Move(Place::local(view))
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
                Operand::Move(Place::local(borrow))
            }
        }
    }

    /// `destination = place == null ? null : wrap &(payload)`, where the payload is
    /// dereferenced first with `through_reference`: an optional shared borrow of the contents of
    /// the nullable `place`, which is neither moved nor copied.
    fn optional_borrow(
        &mut self,
        destination: &Place,
        place: Place,
        through_reference: bool,
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
        let borrow = self.new_temporary(shared(referent), span);
        let rvalue = Rvalue::Borrow {
            mutable: false,
            origin,
            place: payload,
        };
        self.assign(Place::local(borrow), rvalue, span);
        let wrap = Rvalue::WrapNullable(Operand::Move(Place::local(borrow)));
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
        self.optional_borrow(&destination, place, true, BorrowOrigin::Argument, span);
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
        ComparisonSubject { place, view }
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
            self.compare_tag(result, BinaryOperator::Equal, subject.place.clone(), span);
            return;
        }
        let (left, right) = match &subject.view {
            None => (Operand::Copy(subject.place.clone()), self.operand(value)),
            Some(view) => (
                Operand::Copy(view.clone()),
                self.comparison_operand(value, span),
            ),
        };
        self.assign(
            result.clone(),
            Rvalue::Binary(BinaryOperator::Equal, left, right),
            span,
        );
    }
}
