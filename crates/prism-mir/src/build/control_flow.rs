//! Branches, loops and `?.` chains (D8 and D9 of `mir-lowering`).
//!
//! A condition is evaluated in its own temporary scope, which ends before the branch; its `Bool`
//! result lives in a scope of its own that the branch dissolves, marking the result dead at the
//! start of each successor. Blocks are created on demand, so every block is reachable.

use super::{Builder, ChainContext, LoopContext};
use crate::mir::{
    BlockId, BorrowOrigin, Constant, LocalId, Operand, Place, Projection, Rvalue, StatementKind,
    TerminatorKind,
};
use crate::types::RANGE_END_INCLUSIVE;
use prism_ast::Span;
use prism_typed_tree::{
    BinaryOperator, ExpressionId, ExpressionKind, LocalId as TreeLocalId, LogicalOperator, Pattern,
    StatementId, Type, WhenBranch,
};

impl Builder<'_> {
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
        let (on_true, on_false) = self.switch_on_bool(Operand::Move(Place::local(result)), span);
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
            let branches = self.switch_on_bool(Operand::Copy(place), span);
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
        self.current = Some(block);
        let span = self.expression(branch).span;
        self.push_scope(span);
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
        // Without `else`, the branch's value is discarded and the `if` gives `Unit`.
        let branch_destination = if else_branch.is_some() {
            destination
        } else {
            None
        };
        let then_end = self.lower_branch(on_true, branch_destination, then_branch);
        let else_end = match else_branch {
            Some(else_branch) => self.lower_branch(on_false, destination, else_branch),
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
        let right_end = self.lower_branch(evaluate, Some(&destination), right);
        self.join(&[short_end, right_end], span);
    }

    /// `n ?: f`: the consumed nullable in a temporary, a switch on its tag, its payload moved or
    /// copied out where it is present, the fallback in its own scope where it is `null` (D8).
    pub(super) fn lower_elvis(
        &mut self,
        destination: Option<&Place>,
        nullable: ExpressionId,
        fallback: ExpressionId,
        ty: &Type,
        span: Span,
    ) {
        let destination = self.destination_or_temporary(destination, ty, span);
        let nullable_ty = self.expression(nullable).ty.clone();
        let temporary = self.new_temporary(nullable_ty.clone(), span);
        self.expression_into(Some(&Place::local(temporary)), nullable);
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
            Operand::Copy(value)
        } else {
            Operand::Move(value)
        };
        self.assign(destination.clone(), Rvalue::Use(operand), span);
        let present_end = self.current;
        let fallback_end = self.lower_branch(absent, Some(&destination), fallback);
        self.join(&[present_end, fallback_end], span);
    }

    /// Reads the tag of the nullable `place` into a temporary and switches on it: returns the
    /// block where it is present and the block where it is `null`.
    pub(super) fn switch_on_tag(&mut self, place: Place, span: Span) -> (BlockId, BlockId) {
        let tag = self.new_temporary(Type::Int, span);
        self.assign(Place::local(tag), Rvalue::Discriminant(place), span);
        let present = self.new_block();
        let absent = self.new_block();
        let kind = TerminatorKind::SwitchInt {
            value: Operand::Move(Place::local(tag)),
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
                let wrap = Rvalue::WrapNullable(Operand::Move(Place::local(borrow)));
                self.assign(destination.clone(), wrap, span);
            }
        } else if &body_ty == ty || destination.is_none() {
            self.expression_into(destination, chain);
        } else if let Some(destination) = destination {
            let operand = self.operand(chain);
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
        self.exit_scopes_to(scope);
        let mut null_block = self.chains[index].null_block.take();
        self.goto_lazy(&mut null_block, span);
        self.chains[index].null_block = null_block;
        self.current = Some(present);
        place.project(Projection::Payload)
    }

    /// A `when` (D8): the subject once; one switch on its discriminant when every pattern is a
    /// payload-free variant or `else`; ordered tests otherwise, each in its own temporary scope.
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
        let subject = subject.map(|subject| {
            let subject_ty = self.expression(subject).ty.clone();
            (self.place(subject), subject_ty)
        });
        if self.current.is_none() {
            return;
        }
        let patterns = || branches.iter().flat_map(|b| b.patterns.iter());
        let ends = if patterns().all(|p| matches!(p, Pattern::Else)) {
            if let Some((place, _)) = &subject {
                self.push_statement(StatementKind::Read(place.clone()), span);
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
            && patterns().all(|p| matches!(p, Pattern::Variant { .. } | Pattern::Else))
        {
            let (place, _) = subject.clone().expect("checked above");
            self.lower_variant_switch(branch_destination, place, branches, exhaustive, span)
        } else {
            self.lower_ordered_tests(branch_destination, subject, branches, exhaustive, span)
        };
        self.join(&ends, span);
        if *ty == Type::Unit {
            if let Some(destination) = destination {
                self.assign_unit(destination, span);
            }
        }
    }

    /// One switch on the subject's discriminant, to each variant's branch, `otherwise` to the
    /// `else` branch, an unreachable terminator for an exhaustive `when`, or past the `when`.
    fn lower_variant_switch(
        &mut self,
        destination: Option<&Place>,
        subject: Place,
        branches: &[WhenBranch],
        exhaustive: bool,
        span: Span,
    ) -> Vec<Option<BlockId>> {
        let discriminant = self.new_temporary(Type::Int, span);
        self.assign(
            Place::local(discriminant),
            Rvalue::Discriminant(subject),
            span,
        );
        let mut targets: Vec<(u64, BlockId)> = Vec::new();
        let mut bodies = Vec::new();
        let mut otherwise = None;
        for branch in branches {
            if otherwise.is_some() {
                break;
            }
            // A branch whose variants all match earlier branches is never taken: it gets no
            // block, so every block stays reachable.
            let mut block = None;
            for pattern in &branch.patterns {
                match pattern {
                    Pattern::Variant { index, .. } => {
                        let value = *index as u64;
                        if !targets.iter().any(|(v, _)| *v == value) {
                            let target = *block.get_or_insert_with(|| self.new_block());
                            targets.push((value, target));
                        }
                    }
                    Pattern::Else => {
                        otherwise = Some(*block.get_or_insert_with(|| self.new_block()));
                    }
                    _ => unreachable!("a variant switch has only variants and else"),
                }
            }
            if let Some(block) = block {
                bodies.push((block, branch.body));
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
            value: Operand::Move(Place::local(discriminant)),
            targets,
            otherwise,
        };
        self.terminate(kind, span);
        for (block, body) in bodies {
            ends.push(self.lower_branch(block, destination, body));
        }
        ends
    }

    /// The branches' patterns tested in order, each test in its own temporary scope.
    fn lower_ordered_tests(
        &mut self,
        destination: Option<&Place>,
        subject: Option<(Place, Type)>,
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
            .any(|p| self.pattern_may_write(p));
        let subject = subject.map(|(place, ty)| {
            if patterns_may_write && self.is_copy(&ty) {
                match self.stabilize(Operand::Copy(place.clone()), ty.clone(), span) {
                    Operand::Move(stable) => (stable, ty),
                    _ => (place, ty),
                }
            } else {
                (place, ty)
            }
        });
        let subject =
            subject.map(|(place, ty)| self.comparison_subject(place, ty, compares_values, span));
        let mut ends = Vec::new();
        let mut matched_all = false;
        for branch in branches {
            if self.current.is_none() {
                break;
            }
            let mut matches = Vec::new();
            for pattern in &branch.patterns {
                if self.current.is_none() {
                    break;
                }
                match pattern {
                    Pattern::Else => {
                        matches.push(self.current.expect("checked above"));
                        self.current = None;
                        matched_all = true;
                    }
                    Pattern::Condition(condition) => {
                        if let Some((on_true, on_false)) = self.branch_on_expression(*condition) {
                            matches.push(on_true);
                            self.current = Some(on_false);
                        }
                    }
                    Pattern::Value(value) => {
                        let value = *value;
                        let pattern_span = self.expression(value).span;
                        let subject = subject.clone().expect("a value pattern has a subject");
                        let tested = self.branch_on(pattern_span, |builder, result| {
                            builder.equality_with_subject(result, &subject, value, pattern_span)
                        });
                        if let Some((on_true, on_false)) = tested {
                            matches.push(on_true);
                            self.current = Some(on_false);
                        }
                    }
                    Pattern::Variant { index, .. } => {
                        let index = *index as i64;
                        let place = subject.clone().expect("a variant pattern has a subject");
                        let tested = self.branch_on(span, |builder, result| {
                            let discriminant = builder.new_temporary(Type::Int, span);
                            builder.assign(
                                Place::local(discriminant),
                                Rvalue::Discriminant(place.subject_place().clone()),
                                span,
                            );
                            let rvalue = Rvalue::Binary(
                                BinaryOperator::Equal,
                                Operand::Move(Place::local(discriminant)),
                                Operand::Constant(Constant::Int(index)),
                            );
                            builder.assign(result.clone(), rvalue, span);
                        });
                        if let Some((on_true, on_false)) = tested {
                            matches.push(on_true);
                            self.current = Some(on_false);
                        }
                    }
                    Pattern::Range {
                        start,
                        end,
                        end_inclusive,
                    } => {
                        let (start, end, end_inclusive) = (*start, *end, *end_inclusive);
                        let subject = subject.clone().expect("a range pattern has a subject");
                        let value = subject.subject_place().clone();
                        let start_span = self.expression(start).span;
                        let lower = self.branch_on(start_span, |builder, result| {
                            let start = builder.operand(start);
                            let rvalue = Rvalue::Binary(
                                BinaryOperator::LessOrEqual,
                                start,
                                Operand::Copy(value.clone()),
                            );
                            builder.assign(result.clone(), rvalue, start_span);
                        });
                        let Some((above_start, below_start)) = lower else {
                            continue;
                        };
                        self.current = Some(above_start);
                        let end_span = self.expression(end).span;
                        let upper = self.branch_on(end_span, |builder, result| {
                            let end = builder.operand(end);
                            let operator = if end_inclusive {
                                BinaryOperator::LessOrEqual
                            } else {
                                BinaryOperator::Less
                            };
                            let rvalue = Rvalue::Binary(operator, Operand::Copy(value), end);
                            builder.assign(result.clone(), rvalue, end_span);
                        });
                        match upper {
                            Some((within, above_end)) => {
                                matches.push(within);
                                self.join(&[Some(below_start), Some(above_end)], span);
                            }
                            None => self.current = Some(below_start),
                        }
                    }
                }
            }
            let failed = self.current;
            if !matches.is_empty() {
                let body = self.new_block();
                for block in matches {
                    self.current = Some(block);
                    self.goto(body, branch.span);
                }
                ends.push(self.lower_branch(body, destination, branch.body));
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
        self.exit_scopes_to(self.loops[index].break_depth);
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
        self.exit_scopes_to(self.loops[index].continue_depth);
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
        let body_span = self.expression(body).span;
        self.push_scope(body_span);
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
            ExpressionKind::Block { statements, tail } => {
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
                self.exit_scope(body_scope);
                self.goto(start, span);
                self.current = Some(on_false);
                self.exit_scope(body_scope);
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
            Rvalue::Use(Operand::Copy(field(0))),
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
                let inclusive_flag = Operand::Copy(field(RANGE_END_INCLUSIVE));
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
        let body_span = self.expression(body).span;
        self.push_scope(body_span);
        let variable_local = self.declare_binding(variable, span);
        self.assign(
            Place::local(variable_local),
            Rvalue::Use(Operand::Copy(Place::local(value))),
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
                Operand::Copy(Place::local(value)),
                Operand::Copy(Place::local(last)),
            );
            self.assign(Place::local(test), done, span);
            let (finished, more) = self.switch_on_bool(Operand::Copy(Place::local(test)), span);
            self.current = Some(finished);
            let exit = self.loop_exit(index);
            self.goto(exit, span);
            self.current = Some(more);
            let next = Rvalue::Binary(
                BinaryOperator::Add,
                Operand::Copy(Place::local(value)),
                Operand::Constant(Constant::Int(1)),
            );
            self.assign(Place::local(value), next, span);
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
        let field = |i| Operand::Copy(Place::local(range).project(Projection::Field(i)));
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
        let (empty, not_empty) = self.switch_on_bool(Operand::Copy(Place::local(test)), span);
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
