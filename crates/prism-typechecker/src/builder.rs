//! Building the typed tree while checking (D1 of `typed-tree`).
//!
//! A [`BodyBuilder`] holds the arenas of the body being checked, the closures open at the
//! current point (to record their captures, D11) and the loops open at the current point (to
//! name the target of a `break` or `continue`, D9). A [`FileBuilder`] collects the bodies and
//! items of the file as they are checked, which is not source order when a declaration is
//! inferred on demand.

use prism_ast::Span;
use prism_typed_tree::{
    Arena, Body, BodyId, Capture, CaptureMode, DeclarationPath, Expression, ExpressionId,
    ExpressionKind, Item, Local, LocalId, Statement, StatementId, StatementKind, Type,
};
use std::collections::{HashMap, HashSet};

/// A read of `s.value` of a type that is not `Copy`, built outside composable contexts (D7 of
/// `state-handles`): `E4033` when the body ends, unless a mutable use of it was reported.
pub(crate) struct PendingStateRead {
    pub(crate) node: ExpressionId,
    pub(crate) span: Span,
    /// The handle as written, for the help.
    pub(crate) handle: String,
}

/// A closure whose body is being checked.
struct OpenClosure {
    /// The number of bindings that existed when the closure opened: a binding below it belongs
    /// to an enclosing scope, so using it is a capture.
    first_local: usize,
    captures: Vec<Capture>,
}

/// The arenas of the body being checked, and its open closures and loops.
pub(crate) struct BodyBuilder {
    body: Body,
    closures: Vec<OpenClosure>,
    /// The loops enclosing the current point, innermost last. A closure body starts with none,
    /// since a jump cannot leave a closure.
    pub(crate) loops: Vec<StatementId>,
    /// The number of closures opened in the body, so that a body without any skips deciding
    /// captures.
    opened_closures: usize,
    /// The reads of `s.value` that only a composable context may make (D7 of `state-handles`).
    pub(crate) pending_state_reads: Vec<PendingStateRead>,
    /// The `s.value` nodes whose mutable use was reported, whose read is not reported again.
    pub(crate) reported_state_values: HashSet<ExpressionId>,
    /// The span of each `s.value` that is a place, with whether it is in a composable context,
    /// so that a move out of it is reported as one (`E4017`).
    pub(crate) state_value_spans: HashMap<Span, bool>,
}

impl BodyBuilder {
    pub(crate) fn new(owner: DeclarationPath) -> Self {
        Self {
            body: Body::new(owner),
            closures: Vec::new(),
            loops: Vec::new(),
            opened_closures: 0,
            pending_state_reads: Vec::new(),
            reported_state_values: HashSet::new(),
            state_value_spans: HashMap::new(),
        }
    }

    /// Adds an expression node.
    pub(crate) fn add(&mut self, kind: ExpressionKind, ty: Type, span: Span) -> ExpressionId {
        self.body.expressions.alloc(Expression { kind, ty, span })
    }

    /// Replaces node `id`, keeping its id: a node that a parent wraps or joins becomes the
    /// parent (D3).
    pub(crate) fn replace(&mut self, id: ExpressionId, kind: ExpressionKind, ty: Type, span: Span) {
        self.body.expressions[id] = Expression { kind, ty, span };
    }

    pub(crate) fn expression_mut(&mut self, id: ExpressionId) -> &mut Expression {
        &mut self.body.expressions[id]
    }

    pub(crate) fn kind(&self, id: ExpressionId) -> &ExpressionKind {
        &self.body.expressions[id].kind
    }

    pub(crate) fn type_of(&self, id: ExpressionId) -> &Type {
        &self.body.expressions[id].ty
    }

    pub(crate) fn span_of(&self, id: ExpressionId) -> Span {
        self.body.expressions[id].span
    }

    pub(crate) fn add_statement(&mut self, kind: StatementKind, span: Span) -> StatementId {
        self.body.statements.alloc(Statement { kind, span })
    }

    /// Allocates a statement whose kind is set later with [`Self::set_statement`]: a loop, which
    /// its body's jumps name before the loop is complete.
    pub(crate) fn reserve_statement(&mut self, span: Span) -> StatementId {
        let placeholder = StatementKind::Expression {
            value: ExpressionId(u32::MAX),
        };
        self.add_statement(placeholder, span)
    }

    pub(crate) fn set_statement(&mut self, id: StatementId, kind: StatementKind) {
        self.body.statements[id].kind = kind;
    }

    /// The number of closures open at the current point.
    pub(crate) fn closure_depth(&self) -> usize {
        self.closures.len()
    }

    pub(crate) fn add_local(&mut self, local: Local) -> LocalId {
        self.body.locals.alloc(local)
    }

    /// Records a use of binding `id` at `span`: a capture of every open closure the binding is
    /// older than, once per closure, in first-use order (D11).
    pub(crate) fn use_local(&mut self, id: LocalId, span: Span) {
        for closure in &mut self.closures {
            if (id.0 as usize) < closure.first_local
                && !closure.captures.iter().any(|c| c.local == id)
            {
                closure.captures.push(Capture {
                    local: id,
                    span,
                    mode: CaptureMode::Borrowed,
                });
            }
        }
    }

    /// Whether binding `id` is older than the innermost open closure, so that the closure
    /// captures it, and borrows it, when it uses it (D11 of `borrowck-exclusivity`).
    pub(crate) fn is_capture(&self, id: LocalId) -> bool {
        self.closures
            .last()
            .is_some_and(|closure| (id.0 as usize) < closure.first_local)
    }

    /// Whether the body has a closure.
    pub(crate) fn has_closures(&self) -> bool {
        self.opened_closures > 0
    }

    pub(crate) fn open_closure(&mut self) {
        self.opened_closures += 1;
        self.closures.push(OpenClosure {
            first_local: self.body.locals.len(),
            captures: Vec::new(),
        });
    }

    /// Closes the innermost open closure, returning its captures.
    pub(crate) fn close_closure(&mut self) -> Vec<Capture> {
        self.closures
            .pop()
            .map(|closure| closure.captures)
            .unwrap_or_default()
    }

    /// The name of binding `id`.
    pub(crate) fn local_name(&self, id: LocalId) -> String {
        self.body
            .locals
            .get(id)
            .map(|local| local.name.clone())
            .unwrap_or_default()
    }

    pub(crate) fn body_mut(&mut self) -> &mut Body {
        &mut self.body
    }

    pub(crate) fn finish(self) -> Body {
        self.body
    }
}

/// The bodies and items of the file, collected as they are checked.
#[derive(Default)]
pub(crate) struct FileBuilder {
    pub(crate) bodies: Arena<BodyId, Body>,
    /// The item of each checked function and top-level `val`, by its declaration's span.
    pub(crate) items: HashMap<Span, Item>,
}
