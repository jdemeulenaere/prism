//! Testing and binding patterns (D15 of `payload-patterns`).
//!
//! A pattern is tested against a place: a constructor by its tag, then its sub-patterns from left
//! to right over [`Projection::VariantField`], [`Projection::Field`] and [`Projection::Payload`]
//! places. Once a pattern matched, its copying and borrowing bindings are made; then its guard, if
//! any, runs, reading each moving binding's part in place; then a value the pattern takes apart is
//! moved whole into its parts by [`StatementKind::Destructure`], each part going to its binding or
//! to a temporary of the branch, which is dropped where the branch ends.
//!
//! The locals of a branch (its bindings and the temporaries of every alternative's take-apart) are
//! declared once, in the branch's scope, and marked live at the start of each path that matched a
//! pattern, so that every path into the branch body has them live.

use super::Builder;
use crate::mir::{
    BlockId, BorrowOrigin, Constant, LocalDeclaration, LocalId, LocalKind, Operand, Place,
    Projection, Rvalue, StatementKind,
};
use prism_ast::Span;
use prism_typed_tree::{
    BinaryOperator, BindingMode, ExpressionId, LocalId as TreeLocalId, Pattern, Type,
};

/// The MIR locals of one branch: its bindings, then per pattern the temporaries its take-apart
/// needs, in the order [`Builder::take_apart`] uses them.
#[derive(Debug, Default)]
pub(super) struct BranchLocals {
    pub(super) all: Vec<LocalId>,
    pub(super) temporaries: Vec<Vec<LocalId>>,
}

impl Builder<'_> {
    /// Declares the locals of a branch whose patterns are `patterns`, matching a value of type
    /// `ty`, in the innermost scope (the branch's), without marking them live.
    pub(super) fn declare_branch_locals(
        &mut self,
        patterns: &[&Pattern],
        ty: &Type,
    ) -> BranchLocals {
        let mut locals = BranchLocals::default();
        // The patterns of a branch share one binding per name: the first one declares them.
        let mut declared: Vec<TreeLocalId> = Vec::new();
        for pattern in patterns {
            for (binding, _) in pattern.bindings() {
                if declared.contains(&binding) {
                    continue;
                }
                declared.push(binding);
                let tree_local = &self.tree.locals[binding];
                let id = self.add_local(LocalDeclaration {
                    ty: tree_local.ty.clone(),
                    kind: LocalKind::User,
                    mutable: tree_local.mutable,
                    name: Some(tree_local.name.clone()),
                    binding: Some(binding),
                    span: tree_local.span,
                });
                self.schedule_unstarted(id);
                self.bindings.insert(binding, Place::local(id));
                locals.all.push(id);
            }
        }
        for pattern in patterns {
            let mut types = Vec::new();
            self.take_apart_types(pattern, ty, &mut types);
            let mut temporaries = Vec::new();
            for ty in types {
                let id = self.add_local(LocalDeclaration {
                    ty,
                    kind: LocalKind::Temporary,
                    mutable: false,
                    name: None,
                    binding: None,
                    span: self.span,
                });
                self.schedule_unstarted(id);
                locals.all.push(id);
                temporaries.push(id);
            }
            locals.temporaries.push(temporaries);
        }
        locals
    }

    /// Marks the branch's locals live, or dead, on the current path.
    pub(super) fn mark_branch_locals(&mut self, locals: &BranchLocals, live: bool, span: Span) {
        for &local in &locals.all {
            let kind = if live {
                StatementKind::StorageLive(local)
            } else {
                StatementKind::StorageDead(local)
            };
            self.push_statement(kind, span);
        }
    }

    /// The type of a part of a value of type `ty`: payload `field` of variant `variant`, or
    /// field `field` of a struct.
    fn part_type(&self, ty: &Type, variant: Option<usize>, field: usize) -> Type {
        let projection = match variant {
            Some(variant) => Projection::VariantField { variant, field },
            None => Projection::Field(field),
        };
        crate::types::projection_type(ty, projection, self.types)
            .unwrap_or_else(|| panic!("internal error: part {} of a {}", field, ty))
    }

    /// The parts of a constructor pattern: its variant (`None` for a struct) and sub-patterns.
    fn constructor_parts(pattern: &Pattern) -> Option<(Option<usize>, &[Pattern])> {
        match pattern {
            Pattern::Variant { index, fields, .. } => Some((Some(*index), fields)),
            Pattern::Struct { fields, .. } => Some((None, fields)),
            _ => None,
        }
    }

    /// Pushes onto `out` the types of the temporaries taking apart a value of type `ty` by
    /// `pattern` needs, in the order [`Self::take_apart`] uses them.
    fn take_apart_types(&self, pattern: &Pattern, ty: &Type, out: &mut Vec<Type>) {
        if !pattern.moves() {
            return;
        }
        if let Pattern::NonNull(inner) = pattern {
            let payload = match ty {
                Type::Nullable(payload) => payload.as_ref().clone(),
                other => panic!("internal error: a non-null pattern of a {}", other),
            };
            out.push(payload.clone());
            return self.take_apart_types(inner, &payload, out);
        }
        let Some((variant, fields)) = Self::constructor_parts(pattern) else {
            return;
        };
        for (field, sub) in fields.iter().enumerate() {
            let part = self.part_type(ty, variant, field);
            if self.is_copy(&part)
                || matches!(
                    sub,
                    Pattern::Binding {
                        mode: BindingMode::Move,
                        ..
                    }
                )
            {
                continue;
            }
            out.push(part.clone());
            self.take_apart_types(sub, &part, out);
        }
    }

    /// Tests whether the value at `place`, of type `ty`, matches `pattern`, from the current
    /// block: afterwards the current block is the path where it matched, and each block where a
    /// test failed is pushed onto `failures`. The tag of a constructor is tested first, then its
    /// sub-patterns from left to right. `subject_span` is where the subject is read.
    pub(super) fn test_pattern(
        &mut self,
        place: &Place,
        ty: &Type,
        pattern: &Pattern,
        failures: &mut Vec<BlockId>,
        subject_span: Span,
        span: Span,
    ) {
        if self.current.is_none() {
            return;
        }
        match pattern {
            Pattern::Wildcard | Pattern::Binding { .. } | Pattern::Else => {}
            Pattern::Variant { index, fields, .. } => {
                let index = *index;
                let tested = self.branch_on(span, |builder, result| {
                    let discriminant = builder.new_temporary(Type::Int, subject_span);
                    builder.assign(
                        Place::local(discriminant),
                        Rvalue::Discriminant(place.clone()),
                        subject_span,
                    );
                    let rvalue = Rvalue::Binary(
                        BinaryOperator::Equal,
                        Operand::Move(Place::local(discriminant), subject_span),
                        Operand::Constant(Constant::Int(index as i64)),
                    );
                    builder.assign(result.clone(), rvalue, span);
                });
                if !self.follow(tested, failures) {
                    return;
                }
                for (field, sub) in fields.iter().enumerate() {
                    let part = place.project(Projection::VariantField {
                        variant: index,
                        field,
                    });
                    let part_ty = self.part_type(ty, Some(index), field);
                    self.test_pattern(&part, &part_ty, sub, failures, subject_span, span);
                }
            }
            Pattern::Struct { fields, .. } => {
                for (field, sub) in fields.iter().enumerate() {
                    let part = place.project(Projection::Field(field));
                    let part_ty = self.part_type(ty, None, field);
                    self.test_pattern(&part, &part_ty, sub, failures, subject_span, span);
                }
            }
            Pattern::NonNull(inner) => {
                let tested = self.branch_on(span, |builder, result| {
                    builder.compare_tag(
                        result,
                        BinaryOperator::NotEqual,
                        place.clone(),
                        subject_span,
                        span,
                    );
                });
                if !self.follow(tested, failures) {
                    return;
                }
                let payload = match ty {
                    Type::Nullable(payload) => payload.as_ref().clone(),
                    other => panic!("internal error: a non-null pattern of a {}", other),
                };
                let part = place.project(Projection::Payload);
                self.test_pattern(&part, &payload, inner, failures, subject_span, span);
            }
            Pattern::Value(value) => {
                let value = *value;
                let value_span = self.expression(value).span;
                let tested = self.branch_on(value_span, |builder, result| {
                    let subject =
                        builder.comparison_subject(place.clone(), ty.clone(), true, subject_span);
                    builder.equality_with_subject(result, &subject, value, value_span);
                });
                self.follow(tested, failures);
            }
            Pattern::Range {
                start,
                end,
                end_inclusive,
            } => self.test_range(place, *start, *end, *end_inclusive, failures, subject_span),
            Pattern::Condition(_) => {
                panic!("internal error: a condition pattern under a subject")
            }
        }
    }

    /// Continues on the `true` edge of a test, recording its `false` edge as a failure. Returns
    /// whether the test was lowered (it does not diverge).
    fn follow(&mut self, tested: Option<(BlockId, BlockId)>, failures: &mut Vec<BlockId>) -> bool {
        match tested {
            Some((on_true, on_false)) => {
                failures.push(on_false);
                self.current = Some(on_true);
                true
            }
            None => false,
        }
    }

    /// Tests whether the value at `place` is in `start..end` (or `start..<end`).
    pub(super) fn test_range(
        &mut self,
        place: &Place,
        start: ExpressionId,
        end: ExpressionId,
        end_inclusive: bool,
        failures: &mut Vec<BlockId>,
        subject_span: Span,
    ) {
        let start_span = self.expression(start).span;
        let lower = self.branch_on(start_span, |builder, result| {
            let start = builder.operand(start);
            let rvalue = Rvalue::Binary(
                BinaryOperator::LessOrEqual,
                start,
                Operand::Copy(place.clone(), subject_span),
            );
            builder.assign(result.clone(), rvalue, start_span);
        });
        if !self.follow(lower, failures) {
            return;
        }
        let end_span = self.expression(end).span;
        let upper = self.branch_on(end_span, |builder, result| {
            let end = builder.operand(end);
            let operator = if end_inclusive {
                BinaryOperator::LessOrEqual
            } else {
                BinaryOperator::Less
            };
            let rvalue = Rvalue::Binary(operator, Operand::Copy(place.clone(), subject_span), end);
            builder.assign(result.clone(), rvalue, end_span);
        });
        self.follow(upper, failures);
    }

    /// Makes the copying and borrowing bindings of `pattern`, which matched the value at
    /// `place`.
    pub(super) fn bind_copies_and_borrows(
        &mut self,
        place: &Place,
        ty: &Type,
        pattern: &Pattern,
        span: Span,
    ) {
        match pattern {
            Pattern::Binding { local, mode } => {
                let binding = self.bindings[local].clone();
                let rvalue = match mode {
                    BindingMode::Move => return,
                    BindingMode::Copy => Rvalue::Use(Operand::Copy(place.clone(), span)),
                    BindingMode::SharedBorrow | BindingMode::MutableBorrow => Rvalue::Borrow {
                        mutable: *mode == BindingMode::MutableBorrow,
                        origin: BorrowOrigin::Pattern,
                        place: place.clone(),
                    },
                };
                let binding_span = self.tree.locals[*local].span;
                self.assign(binding, rvalue, binding_span);
            }
            Pattern::NonNull(inner) => {
                let payload = match ty {
                    Type::Nullable(payload) => payload.as_ref().clone(),
                    _ => return,
                };
                self.bind_copies_and_borrows(
                    &place.project(Projection::Payload),
                    &payload,
                    inner,
                    span,
                );
            }
            _ => {
                if let Some((variant, fields)) = Self::constructor_parts(pattern) {
                    for (field, sub) in fields.iter().enumerate() {
                        let projection = match variant {
                            Some(variant) => Projection::VariantField { variant, field },
                            None => Projection::Field(field),
                        };
                        let part_ty = self.part_type(ty, variant, field);
                        self.bind_copies_and_borrows(
                            &place.project(projection),
                            &part_ty,
                            sub,
                            span,
                        );
                    }
                }
            }
        }
    }

    /// The parts each moving binding of `pattern`, matched against the value at `place`, reads
    /// in place while its branch's guard runs (D9 of `payload-patterns`).
    pub(super) fn moving_binding_places(
        &self,
        place: &Place,
        pattern: &Pattern,
        out: &mut Vec<(TreeLocalId, Place)>,
    ) {
        match pattern {
            Pattern::Binding {
                local,
                mode: BindingMode::Move,
            } => out.push((*local, place.clone())),
            Pattern::NonNull(inner) => {
                self.moving_binding_places(&place.project(Projection::Payload), inner, out)
            }
            _ => {
                if let Some((variant, fields)) = Self::constructor_parts(pattern) {
                    for (field, sub) in fields.iter().enumerate() {
                        let projection = match variant {
                            Some(variant) => Projection::VariantField { variant, field },
                            None => Projection::Field(field),
                        };
                        self.moving_binding_places(&place.project(projection), sub, out);
                    }
                }
            }
        }
    }

    /// Takes apart the value at `place`, of type `ty`, by `pattern`, which moves a part: the
    /// whole value moves into its parts (D15 of `payload-patterns`), each non-`Copy` part into
    /// its moving binding or into the next of `temporaries`, which a nested pattern takes apart
    /// in turn and an unbound part keeps until the branch ends. A place that is not a whole local
    /// (a nullable's payload) is first moved into a temporary. The statements carry `span`, the
    /// span of the first moving binding, where a later use reports the value moved.
    pub(super) fn take_apart(
        &mut self,
        place: Place,
        ty: &Type,
        pattern: &Pattern,
        temporaries: &mut std::slice::Iter<'_, LocalId>,
        span: Span,
    ) {
        if !pattern.moves() || self.current.is_none() {
            return;
        }
        if let Pattern::Binding {
            local,
            mode: BindingMode::Move,
        } = pattern
        {
            // A binding of the whole value (`val all ->` at the top of a branch) takes it whole.
            let binding = self.bindings[local].clone();
            self.assign(binding, Rvalue::Use(Operand::Move(place, span)), span);
            return;
        }
        if let Pattern::NonNull(inner) = pattern {
            let payload = match ty {
                Type::Nullable(payload) => payload.as_ref().clone(),
                _ => return,
            };
            let temporary = *temporaries.next().expect("a planned temporary");
            let rvalue = Rvalue::Use(Operand::Move(place.project(Projection::Payload), span));
            self.assign(Place::local(temporary), rvalue, span);
            return self.take_apart(Place::local(temporary), &payload, inner, temporaries, span);
        }
        let Some((variant, fields)) = Self::constructor_parts(pattern) else {
            return;
        };
        let source = place
            .as_local()
            .expect("internal error: a value taken apart is a whole local");
        let mut parts = Vec::new();
        let mut nested = Vec::new();
        for (field, sub) in fields.iter().enumerate() {
            let part_ty = self.part_type(ty, variant, field);
            if self.is_copy(&part_ty) {
                continue;
            }
            match sub {
                Pattern::Binding {
                    local,
                    mode: BindingMode::Move,
                } => {
                    let target = self.bindings[local]
                        .as_local()
                        .expect("a moving binding is a local");
                    parts.push((field, target));
                }
                _ => {
                    let temporary = *temporaries.next().expect("a planned temporary");
                    parts.push((field, temporary));
                    if sub.moves() {
                        nested.push((temporary, part_ty, sub));
                    }
                }
            }
        }
        self.push_statement(
            StatementKind::Destructure {
                source,
                variant,
                parts,
            },
            span,
        );
        for (temporary, part_ty, sub) in nested {
            self.take_apart(Place::local(temporary), &part_ty, sub, temporaries, span);
        }
    }

    /// The span of the first moving binding of `pattern`, if any.
    pub(super) fn first_move_span(&self, pattern: &Pattern) -> Option<Span> {
        pattern
            .bindings()
            .into_iter()
            .find(|(_, mode)| *mode == BindingMode::Move)
            .map(|(local, _)| self.tree.locals[local].span)
    }
}
