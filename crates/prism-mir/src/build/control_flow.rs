//! Branches, loops and `?.` chains (D8 and D9 of `mir-lowering`).
//!
//! A condition is evaluated in its own temporary scope, which ends before the branch; its `Bool`
//! result lives in a scope of its own that the branch dissolves, marking the result dead at the
//! start of each successor. Blocks are created on demand, so every block is reachable.

use super::groups::RegionKind;
use super::pattern::BranchLocals;
use super::{Builder, ChainContext, LoopContext};
use crate::mir::{
    BlockId, BorrowOrigin, Callee, Constant, GroupStep, LocalId, Operand, Place, Projection,
    Rvalue, StatementKind, TerminatorKind,
};
use crate::types::RANGE_END_INCLUSIVE;
use prism_ast::Span;
use prism_typed_tree::ParameterKind;
use prism_typed_tree::{
    BinaryOperator, BuiltinFunction, ExpressionId, ExpressionKind, LocalId as TreeLocalId,
    LogicalOperator, Pattern, StatementId, Type, WhenBranch,
};

impl Builder<'_> {
    /// Writes into `destination` whether `element` is in the range `range` (D8 of
    /// `operator-conventions`): `start <= element`, then `element <= end` or `element < end` as
    /// the range's end flag says.
    pub(super) fn range_contains(
        &mut self,
        destination: Place,
        range: Operand,
        range_ty: Type,
        element: Operand,
        span: Span,
    ) {
        let range_local = self.new_temporary(range_ty, span);
        self.assign(Place::local(range_local), Rvalue::Use(range), span);
        let value = self.new_temporary(Type::Int, span);
        self.assign(Place::local(value), Rvalue::Use(element), span);
        let field = |index| Place::local(range_local).project(Projection::Field(index));
        let from_start = Rvalue::Binary(
            BinaryOperator::LessOrEqual,
            Operand::Copy(field(0), span),
            Operand::Copy(Place::local(value), span),
        );
        self.assign(destination.clone(), from_start, span);
        let (above_start, below_start) =
            self.switch_on_bool(Operand::Copy(destination.clone(), span), span);
        self.current = Some(above_start);
        let (closed, half_open) =
            self.switch_on_bool(Operand::Copy(field(RANGE_END_INCLUSIVE), span), span);
        let mut ends = vec![Some(below_start)];
        for (block, operator) in [
            (closed, BinaryOperator::LessOrEqual),
            (half_open, BinaryOperator::Less),
        ] {
            self.current = Some(block);
            let to_end = Rvalue::Binary(
                operator,
                Operand::Copy(Place::local(value), span),
                Operand::Copy(field(1), span),
            );
            self.assign(destination.clone(), to_end, span);
            ends.push(self.current);
        }
        self.join(&ends, span);
    }

    /// Evaluates a `Bool` into a result temporary with `evaluate`, in its own temporary scope,
    /// and branches on it: returns the blocks reached when it is `true` and when it is `false`,
    /// or `None` when the evaluation diverges. There is no current block afterwards.
    pub(super) fn branch_on(
        &mut self,
        span: Span,
        evaluate: impl FnOnce(&mut Self, &Place),
    ) -> Option<(BlockId, BlockId)> {
        self.current?;
        // The result's own scope, dissolved by the branch.
        self.push_scope(span);
        let result = self.new_temporary(Type::Bool, span);
        self.push_scope(span);
        evaluate(self, &Place::local(result));
        self.pop_scope();
        if self.current.is_none() {
            self.pop_scope();
            return None;
        }
        self.discard_scope();
        let (on_true, on_false) =
            self.switch_on_bool(Operand::Move(Place::local(result), span), span);
        for block in [on_true, on_false] {
            self.current = Some(block);
            self.push_statement(StatementKind::StorageDead(result), span);
        }
        self.current = None;
        Some((on_true, on_false))
    }

    /// Branches on the `Bool` operand `value`, returning the `true` and `false` blocks.
    fn switch_on_bool(&mut self, value: Operand, span: Span) -> (BlockId, BlockId) {
        let on_true = self.new_block();
        let on_false = self.new_block();
        let kind = TerminatorKind::SwitchInt {
            value,
            targets: vec![(0, on_false)],
            otherwise: on_true,
        };
        self.terminate(kind, span);
        (on_true, on_false)
    }

    /// Branches on the condition `condition` (D8): a binding is read where it lives, anything
    /// else is evaluated in the condition's temporary scope.
    fn branch_on_expression(&mut self, condition: ExpressionId) -> Option<(BlockId, BlockId)> {
        self.current?;
        let span = self.expression(condition).span;
        if let ExpressionKind::Local(binding) = self.expression(condition).kind {
            let place = self.bindings[&binding].clone();
            let branches = self.switch_on_bool(Operand::Copy(place, span), span);
            return Some(branches);
        }
        self.branch_on(span, |builder, result| {
            builder.expression_into(Some(result), condition)
        })
    }

    /// Lowers `branch` in its own temporary scope, returning the block it falls through to.
    fn lower_branch(
        &mut self,
        block: BlockId,
        destination: Option<&Place>,
        branch: ExpressionId,
    ) -> Option<BlockId> {
        self.lower_branch_in(block, destination, branch, None)
    }

    /// Lowers `branch` in its own temporary scope, which holds the region of `step` when it
    /// has one (D6 of `compose-transform`), returning the block it falls through to.
    fn lower_branch_in(
        &mut self,
        block: BlockId,
        destination: Option<&Place>,
        branch: ExpressionId,
        step: Option<GroupStep>,
    ) -> Option<BlockId> {
        self.current = Some(block);
        let span = self.expression(branch).span;
        self.push_scope(span);
        if let Some(step) = step {
            self.open_region(step, span);
        }
        self.expression_into(destination, branch);
        self.pop_scope();
        self.current
    }

    pub(super) fn lower_if(
        &mut self,
        destination: Option<&Place>,
        condition: ExpressionId,
        then_branch: ExpressionId,
        else_branch: Option<ExpressionId>,
        span: Span,
    ) {
        let Some((on_true, on_false)) = self.branch_on_expression(condition) else {
            return;
        };
        // Each branch containing a composable call is a region (D5 of `compose-transform`).
        let then_composes = self.composes(then_branch);
        let else_composes = else_branch.is_some_and(|branch| self.composes(branch));
        let ordinal = (then_composes || else_composes).then(|| self.region_ordinal(RegionKind::If));
        let step = |composes: bool, branch: u32| {
            ordinal
                .filter(|_| composes)
                .map(|ordinal| GroupStep::If { ordinal, branch })
        };
        // Without `else`, the branch's value is discarded and the `if` gives `Unit`.
        let branch_destination = if else_branch.is_some() {
            destination
        } else {
            None
        };
        let then_end = self.lower_branch_in(
            on_true,
            branch_destination,
            then_branch,
            step(then_composes, 0),
        );
        let else_end = match else_branch {
            Some(else_branch) => {
                self.lower_branch_in(on_false, destination, else_branch, step(else_composes, 1))
            }
            None => Some(on_false),
        };
        self.join(&[then_end, else_end], span);
        if else_branch.is_none() {
            if let Some(destination) = destination {
                self.assign_unit(destination, span);
            }
        }
    }

    /// `a && b` and `a || b`: only the edge that needs the right operand evaluates it, in its
    /// own temporary scope; the other gives `false` (`&&`) or `true` (`||`) (D8).
    pub(super) fn lower_logical(
        &mut self,
        destination: Option<&Place>,
        operator: LogicalOperator,
        left: ExpressionId,
        right: ExpressionId,
        span: Span,
    ) {
        let destination = self.destination_or_temporary(destination, &Type::Bool, span);
        let left = self.operand(left);
        if self.current.is_none() {
            return;
        }
        let (on_true, on_false) = self.switch_on_bool(left, span);
        let (evaluate, short) = match operator {
            LogicalOperator::And => (on_true, on_false),
            LogicalOperator::Or => (on_false, on_true),
        };
        self.current = Some(short);
        let value = Constant::Bool(operator == LogicalOperator::Or);
        self.assign(
            destination.clone(),
            Rvalue::Use(Operand::Constant(value)),
            span,
        );
        let short_end = self.current;
        let step = self.composes(right).then(|| match operator {
            LogicalOperator::And => GroupStep::And {
                ordinal: self.region_ordinal(RegionKind::And),
            },
            LogicalOperator::Or => GroupStep::Or {
                ordinal: self.region_ordinal(RegionKind::Or),
            },
        });
        let right_end = self.lower_branch_in(evaluate, Some(&destination), right, step);
        self.join(&[short_end, right_end], span);
    }

    /// `n ?: f`: the consumed nullable in a temporary, a switch on its tag, its payload moved or
    /// copied out where it is present, the fallback in its own scope where it is `null` (D8).
    /// A left side that cannot be null, which the type checker accepts, is the value: it is
    /// lowered alone and the fallback, which can never run, is not lowered. A left side whose
    /// type is a type parameter `T` may be null in an instance whose argument is nullable, so it
    /// is wrapped into a `T?` temporary and tested like a nullable (D7 of `monomorphization`).
    pub(super) fn lower_elvis(
        &mut self,
        destination: Option<&Place>,
        nullable: ExpressionId,
        fallback: ExpressionId,
        ty: &Type,
        span: Span,
    ) {
        let left_ty = self.expression(nullable).ty.clone();
        let wraps = matches!(left_ty, Type::Parameter(_));
        if !self.is_nullable(nullable) && !wraps {
            self.expression_into(destination, nullable);
            return;
        }
        let destination = self.destination_or_temporary(destination, ty, span);
        let nullable_ty = left_ty.nullable();
        let temporary = self.new_temporary(nullable_ty.clone(), span);
        if wraps {
            let operand = self.operand(nullable);
            self.assign(Place::local(temporary), Rvalue::WrapNullable(operand), span);
        } else {
            self.expression_into(Some(&Place::local(temporary)), nullable);
        }
        if self.current.is_none() {
            return;
        }
        let (present, absent) = self.switch_on_tag(Place::local(temporary), span);
        // Moved out where present, `null` where absent: no drop is needed.
        self.record_moved(temporary);
        self.current = Some(present);
        let value = if &nullable_ty == ty {
            Place::local(temporary)
        } else {
            Place::local(temporary).project(Projection::Payload)
        };
        let operand = if self.is_copy(ty) {
            Operand::Copy(value, span)
        } else {
            Operand::Move(value, span)
        };
        self.assign(destination.clone(), Rvalue::Use(operand), span);
        let present_end = self.current;
        let step = self.composes(fallback).then(|| GroupStep::Elvis {
            ordinal: self.region_ordinal(RegionKind::Elvis),
        });
        let fallback_end = self.lower_branch_in(absent, Some(&destination), fallback, step);
        self.join(&[present_end, fallback_end], span);
    }

    /// Reads the tag of the nullable `place` into a temporary and switches on it: returns the
    /// block where it is present and the block where it is `null`.
    pub(super) fn switch_on_tag(&mut self, place: Place, span: Span) -> (BlockId, BlockId) {
        self.take_guard_read_by(&place);
        let tag = self.new_temporary(Type::Int, span);
        self.assign(Place::local(tag), Rvalue::Discriminant(place), span);
        let present = self.new_block();
        let absent = self.new_block();
        let kind = TerminatorKind::SwitchInt {
            value: Operand::Move(Place::local(tag), span),
            targets: vec![(0, absent)],
            otherwise: present,
        };
        self.terminate(kind, span);
        (present, absent)
    }

    /// A `?.` chain: its body lowered into the destination (wrapped unless already nullable),
    /// or, with `borrow_final`, a shared borrow of its final place, wrapped; each payload test
    /// leaves it with `null` (D8, D14). The chain's scope owns what the chain evaluates after
    /// its first test.
    pub(super) fn lower_chain(
        &mut self,
        destination: Option<&Place>,
        chain: ExpressionId,
        ty: &Type,
        borrow_final: bool,
        span: Span,
    ) {
        self.push_scope(span);
        // A chain containing a composable call is a region, ended on its `null` exits too.
        if self.composes(chain) {
            let ordinal = self.region_ordinal(RegionKind::SafeCall);
            self.open_region(GroupStep::SafeCall { ordinal }, span);
        }
        self.chains.push(ChainContext {
            scope: self.scope_depth() - 1,
            tested: false,
            null_block: None,
        });
        let body_ty = self.expression(chain).ty.clone();
        if borrow_final {
            let place = self.place(chain);
            let reference = Type::Reference {
                mutable: false,
                inner: Box::new(body_ty),
            };
            let borrow = self.new_temporary(reference, span);
            let rvalue = Rvalue::Borrow {
                mutable: false,
                origin: BorrowOrigin::Operator,
                place,
            };
            self.assign(Place::local(borrow), rvalue, span);
            if let Some(destination) = destination {
                let wrap = Rvalue::WrapNullable(Operand::Move(Place::local(borrow), span));
                self.assign(destination.clone(), wrap, span);
            }
        } else if &body_ty == ty || destination.is_none() {
            self.expression_into(destination, chain);
        } else if let Some(destination) = destination {
            let operand = self.operand(chain);
            // The body's temporary was created before the first test, so it belongs to the
            // enclosing scope; it is moved here or never assigned, so it needs no drop (D6).
            if let Some(local) = operand.place().and_then(Place::as_local) {
                self.record_moved_in_any_scope(local);
            }
            self.assign(destination.clone(), Rvalue::WrapNullable(operand), span);
        }
        let context = self.chains.pop().expect("the chain is open");
        if !context.tested {
            self.move_locals_to_parent(context.scope);
        }
        self.pop_scope();
        let present_end = self.current;
        let null_end = context.null_block.and_then(|block| {
            self.current = Some(block);
            if let Some(destination) = destination {
                let null = Rvalue::Use(Operand::Constant(Constant::Null(ty.clone())));
                self.assign(destination.clone(), null, span);
            }
            self.current
        });
        self.join(&[present_end, null_end], span);
    }

    /// The payload of `nullable` in the innermost chain: a test of its tag whose `null` edge
    /// leaves the chain, dropping what the chain created so far (D8).
    pub(super) fn lower_payload(&mut self, nullable: ExpressionId, span: Span) -> Place {
        let place = self.place(nullable);
        if self.current.is_none() {
            return place.project(Projection::Payload);
        }
        let (present, absent) = self.switch_on_tag(place.clone(), span);
        let index = self
            .chains
            .len()
            .checked_sub(1)
            .expect("internal error: a payload outside a chain");
        if !self.chains[index].tested {
            // What the chain evaluated before its first test belongs to the enclosing scope.
            self.chains[index].tested = true;
            let scope = self.chains[index].scope;
            self.move_locals_to_parent(scope);
        }
        self.current = Some(absent);
        let scope = self.chains[index].scope;
        self.exit_scopes_to(scope, None);
        let mut null_block = self.chains[index].null_block.take();
        self.goto_lazy(&mut null_block, span);
        self.chains[index].null_block = null_block;
        self.current = Some(present);
        place.project(Projection::Payload)
    }

    /// A `when` (D8): the subject once; one switch on its discriminant when every pattern is a
    /// variant pattern whose sub-patterns are wildcards and bindings, or `else`, and no branch has
    /// a guard; ordered tests otherwise, each in its own temporary scope (D15 of
    /// `payload-patterns`).
    pub(super) fn lower_when(
        &mut self,
        destination: Option<&Place>,
        subject: Option<ExpressionId>,
        branches: &[WhenBranch],
        exhaustive: bool,
        ty: &Type,
        span: Span,
    ) {
        // A `Unit` `when` discards its branches' values.
        let branch_destination = if *ty == Type::Unit { None } else { destination };
        // The subject is read at its own span, so a use of a moved subject points at it.
        let subject_span = subject.map_or(span, |subject| self.expression(subject).span);
        let subject = subject.map(|subject| {
            let subject_ty = self.expression(subject).ty.clone();
            (self.place(subject), subject_ty)
        });
        if self.current.is_none() {
            return;
        }
        // When a pattern or guard composes, the whole `when` is a region around its tests and
        // branches, so the tests that run vary only inside it (D26 of `compose-transform`).
        let tests_compose = self.when_tests_compose(branches);
        if tests_compose {
            let ordinal = self.region_ordinal(RegionKind::When);
            self.push_scope(span);
            self.open_region(GroupStep::WhenTests { ordinal }, span);
        }
        // Each branch body containing a composable call is a region (D5 of
        // `compose-transform`); guards and conditions stay in the enclosing group.
        let composes = branches.iter().any(|branch| self.composes(branch.body));
        let ordinal = composes.then(|| self.region_ordinal(RegionKind::When));
        let saved_when = std::mem::replace(
            &mut self.when_region,
            ordinal.map(|o| (o, branches.as_ptr())),
        );
        let patterns = || branches.iter().flat_map(|b| b.patterns.iter());
        let switchable = |p: &Pattern| match p {
            Pattern::Else => true,
            Pattern::Variant { fields, .. } => fields
                .iter()
                .all(|f| matches!(f, Pattern::Wildcard | Pattern::Binding { .. })),
            _ => false,
        };
        let ends = if patterns().all(|p| matches!(p, Pattern::Else)) {
            if let Some((place, _)) = &subject {
                self.push_statement(StatementKind::Read(place.clone()), subject_span);
            }
            match branches.first() {
                Some(branch) => {
                    let block = self.new_block();
                    self.goto(block, span);
                    vec![self.lower_branch(block, branch_destination, branch.body)]
                }
                None => vec![self.current],
            }
        } else if subject.is_some()
            && branches.iter().all(|b| b.guard.is_none())
            && patterns().all(switchable)
        {
            let (place, subject_ty) = subject.clone().expect("checked above");
            self.lower_variant_switch(
                branch_destination,
                place,
                &subject_ty,
                subject_span,
                branches,
                exhaustive,
                span,
            )
        } else {
            self.lower_ordered_tests(
                branch_destination,
                subject,
                subject_span,
                branches,
                exhaustive,
                span,
            )
        };
        self.when_region = saved_when;
        self.join(&ends, span);
        if tests_compose {
            self.pop_scope();
        }
        if *ty == Type::Unit {
            if let Some(destination) = destination {
                self.assign_unit(destination, span);
            }
        }
    }

    /// Lowers the body of a branch whose scope (holding its locals) is open, from `block`, then
    /// closes that scope: returns the block it falls through to.
    fn lower_branch_body(
        &mut self,
        block: BlockId,
        destination: Option<&Place>,
        branch: &WhenBranch,
    ) -> Option<BlockId> {
        // The region of a composable branch: the `when`'s ordinal and the branch's position.
        let step = self.when_region.and_then(|(ordinal, first)| {
            if !self.composes(branch.body) {
                return None;
            }
            let position = (branch as *const WhenBranch as usize - first as usize)
                / std::mem::size_of::<WhenBranch>();
            Some(GroupStep::When {
                ordinal,
                branch: position as u32,
            })
        });
        let end = self.lower_branch_in(block, destination, branch.body, step);
        self.current = end;
        self.pop_scope();
        self.current
    }

    /// On the current path, where `pattern` (the `index`-th of its branch) matched the subject
    /// at `place`: marks the branch's locals live and makes the copying and borrowing bindings;
    /// then lowers the guard, reading moving bindings' parts in place, whose `false` edge, with
    /// the locals marked dead again, is pushed onto `failures`; then takes the subject apart.
    #[allow(clippy::too_many_arguments)]
    fn bind_branch_pattern(
        &mut self,
        place: &Place,
        ty: &Type,
        pattern: &Pattern,
        index: usize,
        branch: &WhenBranch,
        locals: &BranchLocals,
        failures: &mut Vec<BlockId>,
        span: Span,
    ) {
        if self.current.is_none() {
            return;
        }
        self.mark_branch_locals(locals, true, branch.span);
        self.bind_copies_and_borrows(place, ty, pattern, span);
        let move_span = self.first_move_span(pattern).unwrap_or(span);
        let guard_borrow = locals.guard_borrows.get(index).copied().flatten();
        if let Some(guard) = branch.guard {
            if let Some(borrow) = guard_borrow {
                let rvalue = Rvalue::Borrow {
                    mutable: false,
                    origin: BorrowOrigin::Guard,
                    place: place.clone(),
                };
                self.assign(Place::local(borrow), rvalue, move_span);
            }
            let mut in_place = Vec::new();
            self.moving_binding_places(place, pattern, &mut in_place);
            let saved: Vec<(TreeLocalId, Place)> = in_place
                .into_iter()
                .map(|(binding, part)| {
                    let own = self
                        .bindings
                        .insert(binding, part)
                        .expect("a declared binding");
                    (binding, own)
                })
                .collect();
            let tested = self.branch_on_expression(guard);
            for (binding, own) in saved {
                self.bindings.insert(binding, own);
            }
            let Some((on_true, on_false)) = tested else {
                return;
            };
            self.current = Some(on_false);
            self.mark_branch_locals(locals, false, branch.span);
            failures.push(self.current.take().expect("the false edge"));
            self.current = Some(on_true);
            if let Some(borrow) = guard_borrow {
                self.push_statement(StatementKind::Read(Place::local(borrow)), move_span);
            }
        }
        let temporaries = locals.temporaries[index].clone();
        self.take_apart(
            place.clone(),
            ty,
            pattern,
            &mut temporaries.iter(),
            move_span,
        );
    }

    /// One switch on the subject's discriminant, to each variant's branch, `otherwise` to the
    /// `else` branch, an unreachable terminator for an exhaustive `when`, or past the `when`. A
    /// branch with bindings gets one block per variant, which binds them before the body.
    #[allow(clippy::too_many_arguments)]
    fn lower_variant_switch(
        &mut self,
        destination: Option<&Place>,
        subject: Place,
        subject_ty: &Type,
        subject_span: Span,
        branches: &[WhenBranch],
        exhaustive: bool,
        span: Span,
    ) -> Vec<Option<BlockId>> {
        let discriminant = self.new_temporary(Type::Int, subject_span);
        self.assign(
            Place::local(discriminant),
            Rvalue::Discriminant(subject.clone()),
            subject_span,
        );
        let mut targets: Vec<(u64, BlockId)> = Vec::new();
        // The variants `targets` already switches on.
        let mut switched: std::collections::HashSet<usize> = std::collections::HashSet::new();
        // Per taken branch: its body block, and the block of each pattern that binds.
        type TakenBranch<'b> = (BlockId, &'b WhenBranch, Vec<(usize, BlockId)>);
        let mut bodies: Vec<TakenBranch> = Vec::new();
        let mut otherwise = None;
        for branch in branches {
            if otherwise.is_some() {
                break;
            }
            let binds = branch.patterns.iter().any(|p| !p.bindings().is_empty());
            // A branch whose variants all match earlier branches is never taken: it gets no
            // block, so every block stays reachable.
            let mut body = None;
            let mut pattern_blocks = Vec::new();
            for (index, pattern) in branch.patterns.iter().enumerate() {
                let fresh = match pattern {
                    Pattern::Variant { index, .. } => !switched.contains(index),
                    Pattern::Else => true,
                    _ => unreachable!("a variant switch has only variants and else"),
                };
                if !fresh {
                    continue;
                }
                let body_block = *body.get_or_insert_with(|| self.new_block());
                let target = if binds {
                    let block = self.new_block();
                    pattern_blocks.push((index, block));
                    block
                } else {
                    body_block
                };
                match pattern {
                    Pattern::Variant { index, .. } => {
                        switched.insert(*index);
                        targets.push((*index as u64, target));
                    }
                    _ => otherwise = Some(target),
                }
            }
            if let Some(body) = body {
                bodies.push((body, branch, pattern_blocks));
            }
        }
        let mut ends = Vec::new();
        let otherwise = match otherwise {
            Some(block) => block,
            None => {
                let block = self.new_block();
                if exhaustive {
                    let saved = self.current;
                    self.current = Some(block);
                    self.terminate(TerminatorKind::Unreachable, span);
                    self.current = saved;
                } else {
                    ends.push(Some(block));
                }
                block
            }
        };
        let kind = TerminatorKind::SwitchInt {
            value: Operand::Move(Place::local(discriminant), subject_span),
            targets,
            otherwise,
        };
        self.terminate(kind, span);
        for (body, branch, pattern_blocks) in bodies {
            self.push_scope(branch.span);
            let patterns: Vec<&Pattern> = branch.patterns.iter().collect();
            let locals = self.declare_branch_locals(&patterns, subject_ty);
            for (index, block) in pattern_blocks {
                self.current = Some(block);
                let mut failures = Vec::new();
                self.bind_branch_pattern(
                    &subject,
                    subject_ty,
                    &branch.patterns[index],
                    index,
                    branch,
                    &locals,
                    &mut failures,
                    span,
                );
                self.goto(body, branch.span);
            }
            ends.push(self.lower_branch_body(body, destination, branch));
        }
        ends
    }

    /// The branches' patterns tested in order, each test in its own temporary scope; a pattern
    /// that matched makes its bindings, runs its branch's guard and takes the subject apart
    /// before jumping to the branch's body.
    #[allow(clippy::too_many_arguments)]
    fn lower_ordered_tests(
        &mut self,
        destination: Option<&Place>,
        subject: Option<(Place, Type)>,
        subject_span: Span,
        branches: &[WhenBranch],
        exhaustive: bool,
        span: Span,
    ) -> Vec<Option<BlockId>> {
        let compares_values = branches
            .iter()
            .flat_map(|b| b.patterns.iter())
            .any(|p| matches!(p, Pattern::Value(value) if !self.is_null(*value)));
        // A `Copy` subject is read by each test; when a pattern may write its place, its value
        // is first copied out, so the subject is evaluated once.
        let patterns_may_write = branches
            .iter()
            .flat_map(|b| b.patterns.iter())
            .any(|p| self.pattern_may_write(p))
            || branches
                .iter()
                .any(|b| b.guard.is_some_and(|guard| self.may_write(guard)));
        let subject = subject.map(|(place, ty)| {
            if patterns_may_write && self.is_copy(&ty) {
                let operand = Operand::Copy(place.clone(), subject_span);
                match self.stabilize(operand, ty.clone(), subject_span) {
                    Operand::Move(stable, _) => (stable, ty),
                    _ => (place, ty),
                }
            } else {
                (place, ty)
            }
        });
        let subject_ty = subject.as_ref().map(|(_, ty)| ty.clone());
        let subject = subject
            .map(|(place, ty)| self.comparison_subject(place, ty, compares_values, subject_span));
        let mut ends = Vec::new();
        let mut matched_all = false;
        for branch in branches {
            if self.current.is_none() {
                break;
            }
            self.push_scope(branch.span);
            let patterns: Vec<&Pattern> = branch.patterns.iter().collect();
            let locals = match &subject_ty {
                Some(ty) => {
                    let mut locals = self.declare_branch_locals(&patterns, ty);
                    if branch.guard.is_some() {
                        self.declare_guard_borrows(&mut locals, &patterns, ty);
                    }
                    locals
                }
                None => BranchLocals::default(),
            };
            let mut matches = Vec::new();
            for (index, pattern) in branch.patterns.iter().enumerate() {
                if self.current.is_none() {
                    break;
                }
                let mut failures = Vec::new();
                match pattern {
                    Pattern::Else => {
                        matched_all = branch.guard.is_none();
                    }
                    Pattern::Condition(condition) => {
                        if let Some((on_true, on_false)) = self.branch_on_expression(*condition) {
                            failures.push(on_false);
                            self.current = Some(on_true);
                        }
                    }
                    Pattern::Value(value) => {
                        let value = *value;
                        let pattern_span = self.expression(value).span;
                        let subject = subject.clone().expect("a value pattern has a subject");
                        let tested = self.branch_on(pattern_span, |builder, result| {
                            builder.equality_with_subject(result, &subject, value, pattern_span)
                        });
                        match tested {
                            Some((on_true, on_false)) => {
                                failures.push(on_false);
                                self.current = Some(on_true);
                            }
                            None => continue,
                        }
                    }
                    pattern => {
                        let place = subject
                            .as_ref()
                            .expect("a pattern has a subject")
                            .subject_place()
                            .clone();
                        let ty = subject_ty.clone().expect("a pattern has a subject");
                        self.test_pattern(&place, &ty, pattern, &mut failures, subject_span, span);
                    }
                }
                if self.current.is_some() {
                    if let (Some(subject), Some(ty)) = (&subject, &subject_ty) {
                        let place = subject.subject_place().clone();
                        self.bind_branch_pattern(
                            &place,
                            ty,
                            pattern,
                            index,
                            branch,
                            &locals,
                            &mut failures,
                            span,
                        );
                    } else if let Some(guard) = branch.guard {
                        if let Some((on_true, on_false)) = self.branch_on_expression(guard) {
                            failures.push(on_false);
                            self.current = Some(on_true);
                        }
                    }
                    if let Some(block) = self.current.take() {
                        matches.push(block);
                    }
                }
                let failed: Vec<Option<BlockId>> = failures.into_iter().map(Some).collect();
                match failed.len() {
                    0 => self.current = None,
                    1 => self.current = failed[0],
                    _ => self.join(&failed, span),
                }
                if matched_all {
                    self.current = None;
                }
            }
            let failed = self.current;
            if matches.is_empty() {
                self.discard_scope();
            } else {
                let body = self.new_block();
                for block in matches {
                    self.current = Some(block);
                    self.goto(body, branch.span);
                }
                ends.push(self.lower_branch_body(body, destination, branch));
            }
            self.current = failed;
            if matched_all {
                break;
            }
        }
        if self.current.is_some() {
            if exhaustive {
                self.terminate(TerminatorKind::Unreachable, span);
            } else {
                ends.push(self.current.take());
            }
        }
        ends
    }

    // Loops.

    /// Where control leaves the loop body `body` when it falls through: its closing brace, or
    /// its span when it is not a block.
    fn block_exit_span(&self, body: ExpressionId) -> Span {
        let expression = self.expression(body);
        match expression.kind {
            ExpressionKind::Block { close_brace, .. } => close_brace,
            _ => expression.span,
        }
    }

    /// The index of the open loop `target` names.
    fn loop_index(&self, target: Option<StatementId>) -> usize {
        let target = target.expect("internal error: a jump that names no loop");
        self.loops
            .iter()
            .rposition(|l| l.statement == target)
            .expect("internal error: a jump out of a loop that is not open")
    }

    /// The exit block of the open loop `index`, created if needed.
    fn loop_exit(&mut self, index: usize) -> BlockId {
        match self.loops[index].exit {
            Some(block) => block,
            None => {
                let block = self.new_block();
                self.loops[index].exit = Some(block);
                block
            }
        }
    }

    /// The block that starts the next iteration of the open loop `index`, created if needed.
    fn loop_next(&mut self, index: usize) -> BlockId {
        match self.loops[index].next {
            Some(block) => block,
            None => {
                let block = self.new_block();
                self.loops[index].next = Some(block);
                block
            }
        }
    }

    pub(super) fn lower_break(&mut self, target: Option<StatementId>, span: Span) {
        let index = self.loop_index(target);
        self.exit_scopes_to(self.loops[index].break_depth, Some(span));
        if self.current.is_some() {
            let exit = self.loop_exit(index);
            self.goto(exit, span);
        }
    }

    /// `continue`: the exit of the scopes inside the loop's body, then a jump to the next
    /// iteration. In a `do`/`while`, the condition still sees the body's locals (the `loops`
    /// spec), including those declared after the `continue`, which it skipped: they are marked
    /// live without a value once the body is lowered (see `lower_do_while`), so the borrow
    /// checker's initialization analysis can reject the condition's read of one of them.
    pub(super) fn lower_continue(&mut self, target: Option<StatementId>, span: Span) {
        let index = self.loop_index(target);
        // The loop's direct composable calls get call-site groups (D26 of `compose-transform`).
        self.mark_continued(self.loops[index].statement);
        self.exit_scopes_to(self.loops[index].continue_depth, Some(span));
        if self.current.is_none() {
            return;
        }
        match self.loops[index].body_scope {
            Some(body_scope) => {
                let block = self.new_block();
                self.goto(block, span);
                let count = self.scope_local_count(body_scope);
                self.loops[index].continues.push((block, count, span));
            }
            None => {
                let next = self.loop_next(index);
                self.goto(next, span);
            }
        }
    }

    /// Ends the open loop: its exit, if anything reaches it, is the current block.
    fn close_loop(&mut self) {
        let context = self.loops.pop().expect("the loop is open");
        self.current = context.exit;
    }

    /// `while (c) body`: the condition at the loop head, which `continue` jumps to.
    pub(super) fn lower_while(
        &mut self,
        id: StatementId,
        condition: ExpressionId,
        body: ExpressionId,
        span: Span,
    ) {
        let head = self.new_block();
        self.goto(head, span);
        self.current = Some(head);
        let depth = self.scope_depth();
        self.loops.push(LoopContext {
            statement: id,
            break_depth: depth,
            continue_depth: depth,
            exit: None,
            next: Some(head),
            body_scope: None,
            continues: Vec::new(),
        });
        if let Some((on_true, on_false)) = self.branch_on_expression(condition) {
            self.current = Some(on_false);
            let exit = self.loop_exit(self.loops.len() - 1);
            self.goto(exit, span);
            if let Some(end) = self.lower_branch(on_true, None, body) {
                self.current = Some(end);
                self.goto(head, span);
            }
        }
        self.close_loop();
    }

    /// `do body while (c)`: the condition is evaluated in the body block's scope, whose locals
    /// are dropped on the back edge and on the exit. A `continue` reaches the condition with
    /// the body's locals declared after it marked live but without a value.
    pub(super) fn lower_do_while(
        &mut self,
        id: StatementId,
        body: ExpressionId,
        condition: ExpressionId,
        span: Span,
    ) {
        let start = self.new_block();
        self.goto(start, span);
        self.current = Some(start);
        let break_depth = self.scope_depth();
        let body_exit = self.block_exit_span(body);
        self.push_block_scope(body_exit);
        let body_scope = self.scope_depth() - 1;
        self.loops.push(LoopContext {
            statement: id,
            break_depth,
            continue_depth: self.scope_depth(),
            exit: None,
            next: None,
            body_scope: Some(body_scope),
            continues: Vec::new(),
        });
        match &self.expression(body).kind {
            ExpressionKind::Block {
                statements, tail, ..
            } => {
                for &statement in statements {
                    if self.current.is_none() {
                        break;
                    }
                    self.statement(statement);
                }
                if let Some(tail) = tail {
                    let tail_span = self.expression(*tail).span;
                    self.push_scope(tail_span);
                    self.expression_into(None, *tail);
                    self.pop_scope();
                }
            }
            _ => self.expression_into(None, body),
        }
        let index = self.loops.len() - 1;
        if self.current.is_some() {
            let next = self.loop_next(index);
            self.goto(next, span);
        }
        for (block, count, continue_span) in std::mem::take(&mut self.loops[index].continues) {
            self.current = Some(block);
            self.mark_live_from(body_scope, count, continue_span);
            let next = self.loop_next(index);
            self.goto(next, continue_span);
        }
        if let Some(next) = self.loops[index].next {
            self.current = Some(next);
            if let Some((on_true, on_false)) = self.branch_on_expression(condition) {
                self.current = Some(on_true);
                self.exit_scope(body_scope, None);
                self.goto(start, span);
                self.current = Some(on_false);
                self.exit_scope(body_scope, None);
                let exit = self.loop_exit(index);
                self.goto(exit, span);
            }
        }
        self.discard_scope();
        self.close_loop();
    }

    /// `for (i in range) body` (D8): the range once into a hidden local; the loop skipped when
    /// the range is empty; the variable bound to the current value in a fresh scope per
    /// iteration; and, in the latch (`continue`'s target), an exit once the current value is the
    /// last element, else an increment, so no addition overflows.
    pub(super) fn lower_for(
        &mut self,
        id: StatementId,
        variable: TreeLocalId,
        range: ExpressionId,
        body: ExpressionId,
        span: Span,
    ) {
        let range_ty = self.expression(range).ty.clone();
        let range_local = self.new_temporary(range_ty, span);
        let range_span = self.expression(range).span;
        self.push_scope(range_span);
        self.expression_into(Some(&Place::local(range_local)), range);
        self.pop_scope();
        if self.current.is_none() {
            return;
        }
        let value = self.new_temporary(Type::Int, span);
        let last = self.new_temporary(Type::Int, span);
        let test = self.new_temporary(Type::Bool, span);
        let field = |index| Place::local(range_local).project(Projection::Field(index));
        let depth = self.scope_depth();
        self.loops.push(LoopContext {
            statement: id,
            break_depth: depth,
            continue_depth: depth,
            exit: None,
            next: None,
            body_scope: None,
            continues: Vec::new(),
        });
        let index = self.loops.len() - 1;
        self.assign(
            Place::local(value),
            Rvalue::Use(Operand::Copy(field(0), span)),
            span,
        );
        let static_inclusive = match self.expression(range).kind {
            ExpressionKind::Range { end_inclusive, .. } => Some(end_inclusive),
            _ => None,
        };
        let entries: Vec<Option<BlockId>> = match static_inclusive {
            Some(inclusive) => {
                vec![self.for_bounds(index, range_local, last, test, inclusive, span)]
            }
            None => {
                let inclusive_flag = Operand::Copy(field(RANGE_END_INCLUSIVE), span);
                let (closed, half_open) = self.switch_on_bool(inclusive_flag, span);
                self.current = Some(closed);
                let closed_entry = self.for_bounds(index, range_local, last, test, true, span);
                self.current = Some(half_open);
                let half_open_entry = self.for_bounds(index, range_local, last, test, false, span);
                vec![closed_entry, half_open_entry]
            }
        };
        self.join(&entries, span);
        if self.current.is_none() {
            self.close_loop();
            return;
        }
        let iteration = self.new_block();
        self.goto(iteration, span);
        self.current = Some(iteration);
        let body_exit = self.block_exit_span(body);
        self.push_block_scope(body_exit);
        let variable_local = self.declare_binding(variable, span);
        self.assign(
            Place::local(variable_local),
            Rvalue::Use(Operand::Copy(Place::local(value), span)),
            span,
        );
        self.expression_into(None, body);
        self.pop_scope();
        if self.current.is_some() {
            let latch = self.loop_next(index);
            self.goto(latch, span);
        }
        if let Some(latch) = self.loops[index].next {
            self.current = Some(latch);
            let done = Rvalue::Binary(
                BinaryOperator::Equal,
                Operand::Copy(Place::local(value), span),
                Operand::Copy(Place::local(last), span),
            );
            self.assign(Place::local(test), done, span);
            let (finished, more) =
                self.switch_on_bool(Operand::Copy(Place::local(test), span), span);
            self.current = Some(finished);
            let exit = self.loop_exit(index);
            self.goto(exit, span);
            self.current = Some(more);
            let next = Rvalue::Binary(
                BinaryOperator::Add,
                Operand::Copy(Place::local(value), span),
                Operand::Constant(Constant::Int(1)),
            );
            self.assign(Place::local(value), next, span);
            self.goto(iteration, span);
        }
        self.close_loop();
    }

    /// A `for` loop over a list (D16 of `collections`): the iterable, a borrow of the list by
    /// the loop, is evaluated once into a hidden local `_l`, in the scope of the `for` statement
    /// so that a temporary list lives until the loop ends; `_n = builtin size(&*_l)` is read once
    /// and the loop is skipped when it is 0. Each iteration binds the variable, in a fresh
    /// scope, to a borrow of the loop's kind of `(*_l)[_i]`; the latch increments the counter
    /// `_i` and exits once it equals `_n`, so `_i` stays below `_n` and never overflows.
    pub(super) fn lower_list_for(
        &mut self,
        id: StatementId,
        variable: TreeLocalId,
        iterable: ExpressionId,
        mutable: bool,
        body: ExpressionId,
        span: Span,
    ) {
        let borrow_ty = self.expression(iterable).ty.clone();
        let element = match &borrow_ty {
            Type::Reference { inner, .. } => match inner.as_ref() {
                Type::List(element) => element.as_ref().clone(),
                other => panic!("internal error: a list loop over a {}", other),
            },
            other => panic!("internal error: a list loop over a {}", other),
        };
        let list = self.new_temporary(borrow_ty, span);
        self.expression_into(Some(&Place::local(list)), iterable);
        if self.current.is_none() {
            return;
        }
        let size = self.new_temporary(Type::Int, span);
        let counter = self.new_temporary(Type::Int, span);
        let test = self.new_temporary(Type::Bool, span);
        let depth = self.scope_depth();
        self.loops.push(LoopContext {
            statement: id,
            break_depth: depth,
            continue_depth: depth,
            exit: None,
            next: None,
            body_scope: None,
            continues: Vec::new(),
        });
        let index = self.loops.len() - 1;
        let shared_list = Type::Reference {
            mutable: false,
            inner: Box::new(Type::List(Box::new(element.clone()))),
        };
        let receiver = self.new_temporary(shared_list, span);
        self.assign(
            Place::local(receiver),
            Rvalue::Borrow {
                mutable: false,
                origin: BorrowOrigin::Iteration,
                place: Self::deref(Place::local(list)),
            },
            span,
        );
        let parameters = BuiltinFunction::ListSize
            .list_member_parameters()
            .expect("a list member")
            .into_iter()
            .map(|(_, ty)| ty)
            .collect();
        self.emit_call(
            Callee::Builtin(BuiltinFunction::ListSize),
            vec![Operand::Move(Place::local(receiver), span)],
            parameters,
            vec![ParameterKind::Plain],
            vec![element],
            Place::local(size),
            false,
            true,
            span,
        );
        self.assign(
            Place::local(counter),
            Rvalue::Use(Operand::Constant(Constant::Int(0))),
            span,
        );
        let empty = Rvalue::Binary(
            BinaryOperator::Equal,
            Operand::Copy(Place::local(size), span),
            Operand::Constant(Constant::Int(0)),
        );
        self.assign(Place::local(test), empty, span);
        let (skipped, entered) = self.switch_on_bool(Operand::Copy(Place::local(test), span), span);
        self.current = Some(skipped);
        let exit = self.loop_exit(index);
        self.goto(exit, span);
        self.current = Some(entered);
        let iteration = self.new_block();
        self.goto(iteration, span);
        self.current = Some(iteration);
        let body_exit = self.block_exit_span(body);
        self.push_block_scope(body_exit);
        let variable_local = self.declare_binding(variable, span);
        let element_place = Self::deref(Place::local(list)).project(Projection::Index(counter));
        self.assign(
            Place::local(variable_local),
            Rvalue::Borrow {
                mutable,
                origin: BorrowOrigin::Iteration,
                place: element_place,
            },
            span,
        );
        self.expression_into(None, body);
        self.pop_scope();
        if self.current.is_some() {
            let latch = self.loop_next(index);
            self.goto(latch, span);
        }
        if let Some(latch) = self.loops[index].next {
            self.current = Some(latch);
            let next = Rvalue::Binary(
                BinaryOperator::Add,
                Operand::Copy(Place::local(counter), span),
                Operand::Constant(Constant::Int(1)),
            );
            self.assign(Place::local(counter), next, span);
            let done = Rvalue::Binary(
                BinaryOperator::Equal,
                Operand::Copy(Place::local(counter), span),
                Operand::Copy(Place::local(size), span),
            );
            self.assign(Place::local(test), done, span);
            let (finished, more) =
                self.switch_on_bool(Operand::Copy(Place::local(test), span), span);
            self.current = Some(finished);
            let exit = self.loop_exit(index);
            self.goto(exit, span);
            self.current = Some(more);
            self.goto(iteration, span);
        }
        self.close_loop();
    }

    /// Exits the loop `index` when the range in `range` is empty, else computes its last
    /// element into `last`; returns the block where the range is not empty.
    fn for_bounds(
        &mut self,
        index: usize,
        range: LocalId,
        last: LocalId,
        test: LocalId,
        inclusive: bool,
        span: Span,
    ) -> Option<BlockId> {
        let field = |i| Operand::Copy(Place::local(range).project(Projection::Field(i)), span);
        let operator = if inclusive {
            BinaryOperator::Greater
        } else {
            BinaryOperator::GreaterOrEqual
        };
        self.assign(
            Place::local(test),
            Rvalue::Binary(operator, field(0), field(1)),
            span,
        );
        let (empty, not_empty) = self.switch_on_bool(Operand::Copy(Place::local(test), span), span);
        self.current = Some(empty);
        let exit = self.loop_exit(index);
        self.goto(exit, span);
        self.current = Some(not_empty);
        let rvalue = if inclusive {
            Rvalue::Use(field(1))
        } else {
            Rvalue::Binary(
                BinaryOperator::Subtract,
                field(1),
                Operand::Constant(Constant::Int(1)),
            )
        };
        self.assign(Place::local(last), rvalue, span);
        self.current
    }
}
