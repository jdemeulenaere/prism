//! Regions of composable bodies (D5 and D6 of `compose-transform`).
//!
//! A composable body runs in one group, and each region of it that contains a composable call
//! runs in a replaceable group of its own: a branch of an `if` or a `when`, the right operand
//! of `&&` or `||`, the fallback of `?:`, a `?.` chain, and a loop around all its iterations.
//! Lowering marks each such region with a [`StatementKind::GroupStart`] where it begins and a
//! [`StatementKind::GroupEnd`] on every path that leaves it. A region is held by the scope
//! lowering opens for its construct, so leaving the scope, by falling through or by `return`,
//! `break` or `continue`, ends the region in the order drops are emitted, innermost first
//! ([`super::scope`]). The compose transform turns the markers into group starts and ends with
//! keys derived from each region's step.
//!
//! Which constructs contain a composable call is decided before lowering by
//! [`ComposableMarks`]: one memoized walk of the typed tree, post-order, that never descends
//! into closures, whose calls run in their own bodies. Each region's step carries its ordinal
//! among the regions of the same construct kind in its parent region, so inserting code that
//! composes nothing shifts no key.

use super::Builder;
use crate::mir::{ClosureOrigin, ClosureStep, GroupId, GroupRegion, GroupStep, StatementKind};
use prism_ast::Span;
use prism_typed_tree::{BuiltinFunction, ExpressionId, ExpressionKind, StatementId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Which expressions and statements of a typed body contain a composable call outside
/// closures, memoized: each node is visited once, so marking a body is linear in its size.
#[derive(Debug, Default)]
pub(crate) struct ComposableMarks {
    /// Per expression: 0 when not computed yet, 1 when it contains no composable call, 2 when
    /// it does.
    expressions: Vec<u8>,
    statements: Vec<u8>,
}

/// The marks of one typed body, shared by the builders of the body and of its closures, which
/// live in the same arenas.
pub(crate) type SharedMarks = Rc<RefCell<ComposableMarks>>;

impl ComposableMarks {
    /// Whether `id` of `tree` contains a composable call outside closures: a call of a `compose
    /// fun`, of a composable function value or of an intrinsic composable (`remember`, `key`,
    /// `currentRecomposeScope`).
    pub(crate) fn expression(&mut self, tree: &prism_typed_tree::Body, id: ExpressionId) -> bool {
        let index = id.0 as usize;
        if self.expressions.len() <= index {
            self.expressions
                .resize(tree.expressions.len().max(index + 1), 0);
        }
        match self.expressions[index] {
            1 => return false,
            2 => return true,
            _ => {}
        }
        let expression = &tree.expressions[id];
        let marked = match &expression.kind {
            // A closure's calls run in its own body, which has its own groups.
            ExpressionKind::Closure { .. } => false,
            // The composer of the running body is read from its argument: no group (D3 of
            // `compose-library-in-prism`).
            ExpressionKind::Call(prism_typed_tree::Call {
                callee: prism_typed_tree::Callee::Builtin(BuiltinFunction::CurrentComposer),
                ..
            }) => self.children(tree, id),
            ExpressionKind::Call(call) if call.is_compose => {
                // Still mark the children, so that a region inside an argument is known.
                self.children(tree, id);
                true
            }
            _ => self.children(tree, id),
        };
        self.expressions[index] = if marked { 2 } else { 1 };
        marked
    }

    /// Whether any child of `id`, or any statement of a block `id`, contains a composable call.
    fn children(&mut self, tree: &prism_typed_tree::Body, id: ExpressionId) -> bool {
        let mut statements = Vec::new();
        let children = tree.expressions[id].kind.children(&mut statements);
        let mut marked = false;
        for child in children {
            marked |= self.expression(tree, child);
        }
        for statement in statements {
            marked |= self.statement(tree, statement);
        }
        marked
    }

    /// Whether the statement `id` contains a composable call outside closures.
    pub(crate) fn statement(&mut self, tree: &prism_typed_tree::Body, id: StatementId) -> bool {
        let index = id.0 as usize;
        if self.statements.len() <= index {
            self.statements
                .resize(tree.statements.len().max(index + 1), 0);
        }
        match self.statements[index] {
            1 => return false,
            2 => return true,
            _ => {}
        }
        let mut marked = false;
        for child in tree.statements[id].kind.children() {
            marked |= self.expression(tree, child);
        }
        self.statements[index] = if marked { 2 } else { 1 };
        marked
    }
}

/// The kinds of constructs whose regions are counted apart for ordinals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum RegionKind {
    If,
    When,
    And,
    Or,
    Elvis,
    SafeCall,
    Loop,
}

/// What a builder of a composable body knows of its regions: the marks of its typed body, the
/// regions open where lowering is, and the ordinals given so far.
#[derive(Debug, Default)]
pub(super) struct RegionState {
    /// The marks, for a composable body; `None` elsewhere, which opens no region.
    pub(super) marks: Option<SharedMarks>,
    /// The open regions, innermost last.
    pub(super) open: Vec<GroupId>,
    /// The next ordinal of each construct kind in each region (`None` for the body's top).
    ordinals: HashMap<(Option<GroupId>, RegionKind), u32>,
    /// The next ordinal of each callee and parameter for composable closures written as
    /// arguments, in each region.
    argument_ordinals: HashMap<(Option<GroupId>, String, u32), u32>,
    /// The next ordinal of other composable closures, in each region.
    value_ordinals: HashMap<Option<GroupId>, u32>,
    /// The composable closures written as call arguments, by expression: the callee's name and
    /// the parameter's index.
    pub(super) arguments: HashMap<ExpressionId, (String, u32)>,
    /// The region of each loop statement that is one, so that a `continue` marks it.
    pub(super) loops: HashMap<StatementId, GroupId>,
}

impl Builder<'_> {
    /// Whether `id` contains a composable call outside closures, in a composable body.
    pub(super) fn composes(&self, id: ExpressionId) -> bool {
        match &self.regions.marks {
            Some(marks) => marks.borrow_mut().expression(self.tree, id),
            None => false,
        }
    }

    /// Whether the statement `id` contains a composable call outside closures, in a composable
    /// body.
    pub(super) fn statement_composes(&self, id: StatementId) -> bool {
        match &self.regions.marks {
            Some(marks) => marks.borrow_mut().statement(self.tree, id),
            None => false,
        }
    }

    /// Whether a pattern or guard of `branches` contains a composable call outside closures, in
    /// a composable body: the tests that run then vary from pass to pass, so the whole `when`
    /// is a region around its tests and branches.
    pub(super) fn when_tests_compose(&self, branches: &[prism_typed_tree::WhenBranch]) -> bool {
        if self.regions.marks.is_none() {
            return false;
        }
        let mut expressions = Vec::new();
        for branch in branches {
            for pattern in &branch.patterns {
                pattern.expressions(&mut expressions);
            }
            expressions.extend(branch.guard);
        }
        expressions.into_iter().any(|id| self.composes(id))
    }

    /// Opens the region of the loop statement `id` in its own scope, which was just opened.
    pub(super) fn open_loop_region_of(&mut self, id: StatementId, step: GroupStep, span: Span) {
        self.open_region(step, span);
        let group = *self.regions.open.last().expect("the region just opened");
        self.regions.loops.insert(id, group);
    }

    /// Marks the region of the loop statement `id`, if it is one, as targeted by a `continue`.
    pub(super) fn mark_continued(&mut self, id: StatementId) {
        if let Some(&group) = self.regions.loops.get(&id) {
            self.groups[group].continued = true;
        }
    }

    /// The next ordinal of a construct of `kind` in the innermost open region.
    pub(super) fn region_ordinal(&mut self, kind: RegionKind) -> u32 {
        let key = (self.regions.open.last().copied(), kind);
        let next = self.regions.ordinals.entry(key).or_insert(0);
        let ordinal = *next;
        *next += 1;
        ordinal
    }

    /// Opens a region of `step` in the innermost scope, which must be the construct's own:
    /// marks its start on the current path and makes the scope end it wherever it is left.
    pub(super) fn open_region(&mut self, step: GroupStep, span: Span) {
        let group = self.groups.alloc(GroupRegion {
            parent: self.regions.open.last().copied(),
            step,
            span,
            continued: false,
        });
        self.push_statement(StatementKind::GroupStart(group), span);
        self.scopes
            .last_mut()
            .expect("a region belongs to an open scope")
            .region = Some(group);
        self.regions.open.push(group);
    }

    /// Where a composable closure `id` is written: its innermost open region, and its step
    /// from there, as an argument of the call it is written in or as the next closure value.
    pub(super) fn closure_origin(&mut self, id: ExpressionId) -> ClosureOrigin {
        let region = self.regions.open.last().copied();
        let step = match self.regions.arguments.get(&id).cloned() {
            Some((callee, parameter)) => {
                let next = self
                    .regions
                    .argument_ordinals
                    .entry((region, callee.clone(), parameter))
                    .or_insert(0);
                let ordinal = *next;
                *next += 1;
                ClosureStep::Argument {
                    callee,
                    ordinal,
                    parameter,
                }
            }
            None => {
                let next = self.regions.value_ordinals.entry(region).or_insert(0);
                let ordinal = *next;
                *next += 1;
                ClosureStep::Value { ordinal }
            }
        };
        ClosureOrigin { region, step }
    }

    /// Records that the argument `value` of a call of `callee`, its parameter `parameter`, is
    /// a closure, under its coercions, so that its origin names the call (D5 of
    /// `compose-transform`).
    pub(super) fn record_closure_argument(
        &mut self,
        value: ExpressionId,
        callee: &str,
        parameter: usize,
    ) {
        let mut id = value;
        while let ExpressionKind::Coerce { value, .. } = self.expression(id).kind {
            id = value;
        }
        if matches!(
            self.expression(id).kind,
            ExpressionKind::Closure { .. }
                | ExpressionKind::FunctionReference(_)
                | ExpressionKind::VariantConstructor { .. }
        ) {
            self.regions
                .arguments
                .insert(id, (callee.to_string(), parameter as u32));
        }
    }
}
