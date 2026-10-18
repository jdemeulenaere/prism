//! Deciding which captures a closure owns (D1, D2 and D5 of `escaping-closures`).
//!
//! After a body is checked, one walk of its typed tree builds a *flow graph* whose nodes are the
//! body's local bindings and closure literals:
//!
//! - a node read at an *owning position* (returned, stored in a field, payload or box, written
//!   through a reference, assigned to a captured variable, passed for an `escaping` or generic
//!   parameter, the result of a function, closure or default value) is a *root*;
//! - a node at the top of the initializer of, or of a value assigned to, a binding `b` has a
//!   *source edge* to `b`, reaching through `if`, `when`, block results, `?:` and coercions;
//! - a closure has a *capture edge* to each binding it captures.
//!
//! A node is *escaping* when it reaches a root backwards along source edges, or is captured by
//! an escaping closure; an escaping closure owns every capture. A closure that is not escaping
//! owns each capture whose declaring block does not enclose every binding the closure may be
//! stored in, found over the strongly connected components of the source edges. A binding owned
//! that way becomes a root for one more search. Every step is linear in the body: one walk, one
//! component search, two graph searches, and a sort of the roots by position.
//!
//! The pass also reports `CannotCaptureByValue` (`E4028`) for a capture by value of a `var`, or
//! of a non-`Copy` binding the closure's builder does not own, and records such a capture as
//! borrowed.

use crate::error::{CaptureRefusal, TypeError};
use prism_ast::Span;
use prism_typed_tree::{
    Argument, BindingMode, Body, BuiltinFunction, Callee, Capture, CaptureMode, ConstructKind,
    ExpressionId, ExpressionKind, LocalId, LocalKind, OwnershipReason, OwnershipReasonKind,
    Pattern, StatementId, StatementKind, TypeTable,
};

/// What a body's value is, which names why a closure it returns owns its captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyKind {
    Function,
    TopLevelValue,
}

/// A node of the flow graph: locals first, then closures.
type Node = usize;

const NONE: u32 = u32::MAX;

struct ClosureInfo<'a> {
    expression: ExpressionId,
    /// The innermost closure enclosing this one, by index.
    parent: Option<usize>,
    /// Whether the closure is written in a default value, outside any other closure.
    in_default: bool,
    captures: &'a [Capture],
}

/// One block-like scope: its interval in a walk that numbers entries and exits, so that a scope
/// encloses another exactly when its interval contains the other's.
#[derive(Clone, Copy)]
struct Scope {
    enter: u32,
    exit: u32,
}

/// The outermost place a node may be stored in: the smallest interval containing the scopes of
/// every binding it reaches along source edges, with the store into the outermost of them.
#[derive(Clone, Copy)]
struct Extent {
    enter: u32,
    exit: u32,
    witness: Span,
}

impl Extent {
    fn join(self, other: Extent) -> Extent {
        Extent {
            enter: self.enter.min(other.enter),
            exit: self.exit.max(other.exit),
            witness: if other.enter < self.enter {
                other.witness
            } else {
                self.witness
            },
        }
    }
}

struct Graph<'a> {
    body: &'a Body,
    local_count: usize,
    /// The scope that declares each local; the function's own scope by default.
    local_scope: Vec<u32>,
    scopes: Vec<Scope>,
    scope_stack: Vec<u32>,
    clock: u32,
    /// The closure index of each closure expression, `NONE` for other expressions.
    closure_of: Vec<u32>,
    closures: Vec<ClosureInfo<'a>>,
    closure_stack: Vec<usize>,
    in_default: bool,
    /// Source edges: (source, binding, the store's span).
    edges: Vec<(Node, LocalId, Span)>,
    roots: Vec<(Node, OwnershipReason)>,
    /// A buffer for [`Graph::tops`].
    scratch: Vec<(Node, Span)>,
}

/// Decides the captures of every closure of `body`, recording each capture's mode and each
/// owning closure's reason in the body, and returns the `E4028` errors. The checker calls it
/// only for bodies with closures.
pub(crate) fn decide_captures(
    body: &mut Body,
    types: &TypeTable,
    kind: BodyKind,
) -> Vec<TypeError> {
    let decisions = {
        let mut graph = Graph::new(body);
        graph.walk_body(kind);
        graph.decide(types)
    };
    let mut errors = Vec::new();
    for decision in decisions {
        errors.extend(decision.errors);
        if let ExpressionKind::Closure {
            captures,
            ownership_reason,
            ..
        } = &mut body.expressions[decision.expression].kind
        {
            *captures = decision.captures;
            *ownership_reason = decision.reason;
        }
    }
    errors
}

struct Decision {
    expression: ExpressionId,
    captures: Vec<Capture>,
    reason: Option<OwnershipReason>,
    errors: Vec<TypeError>,
}

fn reason(span: Span, kind: OwnershipReasonKind) -> OwnershipReason {
    OwnershipReason { span, kind }
}

impl<'a> Graph<'a> {
    fn new(body: &'a Body) -> Self {
        let local_count = body.locals.len();
        Self {
            body,
            local_count,
            local_scope: vec![0; local_count],
            scopes: Vec::new(),
            scope_stack: Vec::new(),
            clock: 0,
            closure_of: vec![NONE; body.expressions.len()],
            closures: Vec::new(),
            closure_stack: Vec::new(),
            in_default: false,
            edges: Vec::new(),
            roots: Vec::new(),
            scratch: Vec::new(),
        }
    }

    // --- Recording

    fn enter_scope(&mut self) {
        let index = self.scopes.len() as u32;
        self.scopes.push(Scope {
            enter: self.clock,
            exit: 0,
        });
        self.clock += 1;
        self.scope_stack.push(index);
    }

    fn exit_scope(&mut self) {
        let index = self.scope_stack.pop().expect("an open scope");
        self.scopes[index as usize].exit = self.clock;
        self.clock += 1;
    }

    fn declare(&mut self, local: LocalId) {
        if let Some(slot) = self.local_scope.get_mut(local.0 as usize) {
            *slot = *self.scope_stack.last().expect("an open scope");
        }
    }

    fn closure_node(&mut self, expression: ExpressionId) -> Node {
        let index = self.closure_of[expression.0 as usize];
        let index = if index == NONE {
            let body = self.body;
            let ExpressionKind::Closure { captures, .. } = &body.expressions[expression].kind
            else {
                unreachable!("a closure node")
            };
            let index = self.closures.len();
            self.closures.push(ClosureInfo {
                expression,
                parent: self.closure_stack.last().copied(),
                in_default: self.in_default && self.closure_stack.is_empty(),
                captures,
            });
            self.closure_of[expression.0 as usize] = index as u32;
            index
        } else {
            index as usize
        };
        self.local_count + index
    }

    /// Whether a binding may hold a closure, so that it takes part in the graph.
    fn holds_function(&self, local: LocalId) -> bool {
        self.body.locals[local].ty.is_function_value()
    }

    /// The nodes whose value `id` may be: the closure literals and bindings read at its top,
    /// through branches, block results, `?:` and coercions, with the span of each.
    fn tops(&mut self, id: ExpressionId, out: &mut Vec<(Node, Span)>) {
        let body = self.body;
        let expression = &body.expressions[id];
        match &expression.kind {
            ExpressionKind::Closure { .. } => {
                let span = expression.span;
                let node = self.closure_node(id);
                out.push((node, span));
            }
            ExpressionKind::Local(local) => {
                if self.holds_function(*local) {
                    out.push((local.0 as usize, expression.span));
                }
            }
            ExpressionKind::Move { place } | ExpressionKind::Copy { place } => {
                if let ExpressionKind::Local(local) = self.body.expressions[*place].kind {
                    if self.holds_function(local) {
                        out.push((local.0 as usize, expression.span));
                    }
                }
            }
            ExpressionKind::Coerce { value, .. } | ExpressionKind::Temporary { value } => {
                let value = *value;
                self.tops(value, out)
            }
            ExpressionKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                let (then_branch, else_branch) = (*then_branch, *else_branch);
                self.tops(then_branch, out);
                if let Some(else_branch) = else_branch {
                    self.tops(else_branch, out);
                }
            }
            ExpressionKind::When { branches, .. } => {
                for branch in branches {
                    self.tops(branch.body, out);
                }
            }
            ExpressionKind::Block {
                tail: Some(tail), ..
            } => {
                let tail = *tail;
                self.tops(tail, out)
            }
            ExpressionKind::Elvis { nullable, fallback } => {
                let (nullable, fallback) = (*nullable, *fallback);
                self.tops(nullable, out);
                self.tops(fallback, out);
            }
            _ => {}
        }
    }

    /// Records the value `id` as written at an owning position of `kind`.
    fn own(&mut self, id: ExpressionId, kind: OwnershipReasonKind) {
        let mut tops = std::mem::take(&mut self.scratch);
        self.tops(id, &mut tops);
        for &(node, span) in &tops {
            self.roots.push((node, reason(span, kind.clone())));
        }
        tops.clear();
        self.scratch = tops;
    }

    /// Records the value `id` as stored into binding `local` by the store at `span`.
    fn store(&mut self, id: ExpressionId, local: LocalId, span: Span) {
        if !self.holds_function(local) {
            return;
        }
        let mut tops = std::mem::take(&mut self.scratch);
        self.tops(id, &mut tops);
        for &(node, _) in &tops {
            self.edges.push((node, local, span));
        }
        tops.clear();
        self.scratch = tops;
    }

    fn walk_body(&mut self, kind: BodyKind) {
        self.enter_scope();
        // Parameters and the receiver are declared in the function's own scope, scope 0.
        let defaults: Vec<ExpressionId> = self.body.defaults.iter().flatten().copied().collect();
        for default in defaults {
            self.in_default = true;
            self.own(default, OwnershipReasonKind::Returned);
            self.expression(default);
            self.in_default = false;
        }
        if let Some(value) = self.body.value {
            let reason = match kind {
                BodyKind::Function => OwnershipReasonKind::Returned,
                BodyKind::TopLevelValue => OwnershipReasonKind::TopLevelValue,
            };
            self.own(value, reason);
            self.expression(value);
        }
        self.exit_scope();
    }

    fn expression(&mut self, id: ExpressionId) {
        let body = self.body;
        match &body.expressions[id].kind {
            ExpressionKind::Block {
                statements, tail, ..
            } => {
                self.enter_scope();
                for statement in statements {
                    self.statement(*statement);
                }
                if let Some(tail) = tail {
                    self.expression(*tail);
                }
                self.exit_scope();
            }
            ExpressionKind::Closure {
                parameters,
                body: closure_body,
                ..
            } => {
                let node = self.closure_node(id);
                self.enter_scope();
                for parameter in parameters {
                    self.declare(*parameter);
                }
                self.closure_stack.push(node - self.local_count);
                self.own(*closure_body, OwnershipReasonKind::Returned);
                self.expression(*closure_body);
                self.closure_stack.pop();
                self.exit_scope();
            }
            ExpressionKind::When {
                subject, branches, ..
            } => {
                if let Some(subject) = subject {
                    self.expression(*subject);
                }
                for branch in branches {
                    self.enter_scope();
                    let mut children = Vec::new();
                    for pattern in &branch.patterns {
                        for (local, _) in pattern.bindings() {
                            self.declare(local);
                        }
                        // A binding of the whole subject, or of a nullable subject's payload,
                        // by value holds what the subject holds.
                        if let (Some(subject), Some(local)) = (subject, whole_binding(pattern)) {
                            let span = self.body.expressions[*subject].span;
                            self.store(*subject, local, span);
                        }
                        pattern.expressions(&mut children);
                    }
                    for child in children {
                        self.expression(child);
                    }
                    if let Some(guard) = branch.guard {
                        self.expression(guard);
                    }
                    self.expression(branch.body);
                    self.exit_scope();
                }
            }
            ExpressionKind::Call(call) => {
                if let Callee::Value(callee) = call.callee {
                    self.expression(callee);
                }
                for (index, argument) in call.arguments.iter().enumerate() {
                    let Argument::Written(value) = argument else {
                        continue;
                    };
                    let kind = if call.callee == Callee::Builtin(BuiltinFunction::HeapAllocate) {
                        Some(OwnershipReasonKind::StoredInBox)
                    } else if call.escaping.get(index).copied().unwrap_or(false) {
                        Some(OwnershipReasonKind::PassedToEscapingParameter)
                    } else if call
                        .parameters
                        .get(index)
                        .is_some_and(|(_, ty)| ty.mentions_parameter_outside_functions())
                    {
                        Some(OwnershipReasonKind::PassedToGenericParameter)
                    } else {
                        None
                    };
                    if let Some(kind) = kind {
                        self.own(*value, kind);
                    }
                    self.expression(*value);
                }
            }
            ExpressionKind::Construct { kind, fields, .. } => {
                let reason = match kind {
                    ConstructKind::Struct(_) => OwnershipReasonKind::StoredInField,
                    ConstructKind::Variant { .. } => OwnershipReasonKind::StoredInPayload,
                };
                for field in fields {
                    self.own(field.value, reason.clone());
                    self.expression(field.value);
                }
            }
            ExpressionKind::Return { value: Some(value) } => {
                self.own(*value, OwnershipReasonKind::Returned);
                self.expression(*value);
            }
            // The other nodes, visited without allocating their list of children.
            ExpressionKind::Field { base: child, .. }
            | ExpressionKind::Deref { pointer: child }
            | ExpressionKind::NonNullPayload { nullable: child }
            | ExpressionKind::Temporary { value: child }
            | ExpressionKind::Coerce { value: child, .. }
            | ExpressionKind::Move { place: child }
            | ExpressionKind::Copy { place: child }
            | ExpressionKind::Borrow { place: child, .. }
            | ExpressionKind::SafeNavigation { chain: child, .. }
            | ExpressionKind::Unary { operand: child, .. } => self.expression(*child),
            ExpressionKind::Index {
                base: left,
                index: right,
            }
            | ExpressionKind::Range {
                start: left,
                end: right,
                ..
            }
            | ExpressionKind::Binary { left, right, .. }
            | ExpressionKind::Logical { left, right, .. }
            | ExpressionKind::Elvis {
                nullable: left,
                fallback: right,
            } => {
                self.expression(*left);
                self.expression(*right);
            }
            ExpressionKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expression(*condition);
                self.expression(*then_branch);
                if let Some(else_branch) = else_branch {
                    self.expression(*else_branch);
                }
            }
            kind => {
                let mut statements = Vec::new();
                for child in kind.children(&mut statements) {
                    self.expression(child);
                }
                for statement in statements {
                    self.statement(statement);
                }
            }
        }
    }

    fn statement(&mut self, id: StatementId) {
        let body = self.body;
        let statement = &body.statements[id];
        match &statement.kind {
            StatementKind::Let { local, initializer } => {
                self.declare(*local);
                self.store(*initializer, *local, statement.span);
                self.expression(*initializer);
            }
            StatementKind::Assign { target, value } => {
                match self.assigned_binding(*target) {
                    AssignTarget::Binding(local) => self.store(*value, local, statement.span),
                    AssignTarget::CapturedBinding => {
                        self.own(*value, OwnershipReasonKind::AssignedToCapturedVariable)
                    }
                    AssignTarget::Field => self.own(*value, OwnershipReasonKind::StoredInField),
                    AssignTarget::ThroughReference => {
                        self.own(*value, OwnershipReasonKind::WrittenThroughReference)
                    }
                }
                self.expression(*target);
                self.expression(*value);
            }
            StatementKind::For {
                variable,
                range,
                body: loop_body,
            } => {
                self.expression(*range);
                self.enter_scope();
                self.declare(*variable);
                self.expression(*loop_body);
                self.exit_scope();
            }
            StatementKind::Destructure { pattern, value } => {
                for (local, _) in pattern.bindings() {
                    self.declare(local);
                }
                self.expression(*value);
            }
            StatementKind::CompoundAssign { target, value, .. } => {
                self.expression(*target);
                self.expression(*value);
            }
            StatementKind::Expression { value } => self.expression(*value),
            StatementKind::While { condition, body }
            | StatementKind::DoWhile { body, condition } => {
                self.expression(*condition);
                self.expression(*body);
            }
        }
    }

    /// What an assignment's target is: a whole binding (captured by the closure being walked or
    /// not), a field path or a place reached through a reference or box.
    fn assigned_binding(&self, target: ExpressionId) -> AssignTarget {
        match &self.body.expressions[target].kind {
            ExpressionKind::Local(local) => {
                let captured = self.closure_stack.last().is_some_and(|&closure| {
                    self.closures[closure]
                        .captures
                        .iter()
                        .any(|c| c.local == *local)
                });
                if captured {
                    AssignTarget::CapturedBinding
                } else {
                    AssignTarget::Binding(*local)
                }
            }
            ExpressionKind::Deref { .. } => AssignTarget::ThroughReference,
            ExpressionKind::Field { base, .. } => {
                // A field reached through a reference is written through it.
                let mut base = *base;
                loop {
                    match &self.body.expressions[base].kind {
                        ExpressionKind::Field { base: inner, .. } => base = *inner,
                        ExpressionKind::Deref { .. } => return AssignTarget::ThroughReference,
                        _ => return AssignTarget::Field,
                    }
                }
            }
            _ => AssignTarget::Field,
        }
    }

    // --- Deciding

    fn decide(mut self, types: &TypeTable) -> Vec<Decision> {
        let node_count = self.local_count + self.closures.len();
        // Sources of each binding (backwards) and bindings of each source (forwards), as
        // compressed adjacency lists.
        let mut sources_start = vec![0usize; node_count + 1];
        let mut targets_start = vec![0usize; node_count + 1];
        for &(source, binding, _) in &self.edges {
            sources_start[binding.0 as usize + 1] += 1;
            targets_start[source + 1] += 1;
        }
        for index in 0..node_count {
            sources_start[index + 1] += sources_start[index];
            targets_start[index + 1] += targets_start[index];
        }
        let mut sources = vec![0usize; self.edges.len()];
        let mut targets = vec![(0usize, Span::dummy()); self.edges.len()];
        let mut source_fill = sources_start.clone();
        let mut target_fill = targets_start.clone();
        for &(source, binding, span) in &self.edges {
            let binding = binding.0 as usize;
            sources[source_fill[binding]] = source;
            source_fill[binding] += 1;
            targets[target_fill[source]] = (binding, span);
            target_fill[source] += 1;
        }

        // Escaping nodes, each with the reason of the first root in source order reaching it, as
        // an index into `reasons`.
        let mut escaping: Vec<u32> = vec![NONE; node_count];
        let mut roots = std::mem::take(&mut self.roots);
        let mut reasons: Vec<OwnershipReason> = Vec::new();
        self.search(
            &mut roots,
            &mut escaping,
            &mut reasons,
            &sources_start,
            &sources,
        );

        // Closures that outlive a captured binding own it.
        let extents = self.extents(node_count, &targets_start, &targets);
        // Whether each capture is owned because its closure outlives it, per closure.
        let mut outlived: Vec<Vec<bool>> = Vec::with_capacity(self.closures.len());
        let mut outlived_reasons: Vec<Option<OwnershipReason>> = vec![None; self.closures.len()];
        let mut second_roots = Vec::new();
        for (index, closure) in self.closures.iter().enumerate() {
            let node = self.local_count + index;
            let mut owned = vec![false; closure.captures.len()];
            if escaping[node] == NONE {
                if let Some(extent) = extents[node] {
                    for (position, capture) in closure.captures.iter().enumerate() {
                        let scope =
                            self.scopes[self.local_scope[capture.local.0 as usize] as usize];
                        let encloses = scope.enter <= extent.enter && extent.exit <= scope.exit;
                        if !encloses {
                            owned[position] = true;
                            if outlived_reasons[index].is_none() {
                                let name = self.body.locals[capture.local].name.clone();
                                outlived_reasons[index] = Some(reason(
                                    extent.witness,
                                    OwnershipReasonKind::StoredOutsideBlock { name },
                                ));
                            }
                            second_roots.push((
                                capture.local.0 as usize,
                                reason(capture.span, OwnershipReasonKind::CapturedByOwningClosure),
                            ));
                        }
                    }
                }
            }
            outlived.push(owned);
        }
        self.search(
            &mut second_roots,
            &mut escaping,
            &mut reasons,
            &sources_start,
            &sources,
        );

        // Each closure's captures.
        let mut decisions = Vec::with_capacity(self.closures.len());
        for (index, closure) in self.closures.iter().enumerate() {
            let node = self.local_count + index;
            let (owns_all, mut reason) = match escaping[node] {
                NONE => (false, outlived_reasons[index].clone()),
                found => (true, Some(reasons[found as usize].clone())),
            };
            let mut captures = closure.captures.to_vec();
            let mut errors = Vec::new();
            for (position, capture) in captures.iter_mut().enumerate() {
                if !(owns_all || outlived[index][position]) {
                    capture.mode = CaptureMode::Borrowed;
                    continue;
                }
                let local = &self.body.locals[capture.local];
                let copy = types.is_copy(&local.ty);
                let refusal = if local.mutable {
                    Some(CaptureRefusal::Variable)
                } else if copy {
                    None
                } else if self.captured_by_parent(index, capture.local) {
                    Some(CaptureRefusal::EnclosingClosure)
                } else if closure.in_default
                    && matches!(local.kind, LocalKind::Parameter | LocalKind::Receiver)
                {
                    Some(CaptureRefusal::DefaultValueInput)
                } else {
                    None
                };
                match refusal {
                    Some(refusal) => {
                        capture.mode = CaptureMode::Borrowed;
                        let owning = reason.clone().expect("an owned capture has a reason");
                        errors.push(TypeError::CannotCaptureByValue {
                            name: local.name.clone(),
                            refusal,
                            help: capture_help(&local.name, refusal, types.is_clone(&local.ty)),
                            reason: owning,
                            span: capture.span,
                        });
                    }
                    None => {
                        capture.mode = if copy {
                            CaptureMode::Copied
                        } else {
                            CaptureMode::Moved
                        };
                    }
                }
            }
            if !captures.iter().any(|c| c.mode.is_owned()) {
                reason = None;
            }
            decisions.push(Decision {
                expression: closure.expression,
                captures,
                reason,
                errors,
            });
        }
        decisions
    }

    /// Whether `local` is a capture of the closure enclosing closure `index`.
    fn captured_by_parent(&self, index: usize, local: LocalId) -> bool {
        self.closures[index].parent.is_some_and(|parent| {
            self.closures[parent]
                .captures
                .iter()
                .any(|c| c.local == local)
        })
    }

    /// Marks every node reaching one of `roots`, taken in source order: a binding reaches its
    /// sources, which escape for the same reason, and an escaping closure its captures, which
    /// escape because it captures them.
    fn search(
        &self,
        roots: &mut [(Node, OwnershipReason)],
        escaping: &mut [u32],
        reasons: &mut Vec<OwnershipReason>,
        sources_start: &[usize],
        sources: &[usize],
    ) {
        roots.sort_by_key(|(_, reason)| (reason.span.start, reason.span.end));
        // Nodes to visit, each with the index of its reason, or with `NONE` and the capture that
        // reaches it, whose reason is made when the node is visited.
        let mut stack: Vec<(Node, u32, Span)> = Vec::new();
        for (root, reason) in roots.iter() {
            if escaping[*root] != NONE {
                continue;
            }
            reasons.push(reason.clone());
            stack.push((*root, (reasons.len() - 1) as u32, reason.span));
            while let Some((node, mut found, span)) = stack.pop() {
                if escaping[node] != NONE {
                    continue;
                }
                if found == NONE {
                    reasons.push(OwnershipReason {
                        span,
                        kind: OwnershipReasonKind::CapturedByOwningClosure,
                    });
                    found = (reasons.len() - 1) as u32;
                }
                escaping[node] = found;
                if node < self.local_count {
                    for &source in &sources[sources_start[node]..sources_start[node + 1]] {
                        if escaping[source] == NONE {
                            stack.push((source, found, span));
                        }
                    }
                } else {
                    for capture in self.closures[node - self.local_count].captures.iter().rev() {
                        let local = capture.local.0 as usize;
                        if escaping[local] == NONE {
                            stack.push((local, NONE, capture.span));
                        }
                    }
                }
            }
        }
    }

    /// The extent of every node: where the bindings it reaches along source edges are declared.
    /// Computed once per strongly connected component of the source edges (cycles come from
    /// reassigned `var`s), in the reverse topological order Tarjan's search finishes them in.
    fn extents(
        &self,
        node_count: usize,
        targets_start: &[usize],
        targets: &[(Node, Span)],
    ) -> Vec<Option<Extent>> {
        let mut extents: Vec<Option<Extent>> = vec![None; node_count];
        let mut index_of = vec![NONE; node_count];
        let mut low = vec![0u32; node_count];
        let mut on_stack = vec![false; node_count];
        // The component each finished node belongs to, by its root.
        let mut component_of = vec![NONE; node_count];
        let mut component: Vec<Node> = Vec::new();
        let mut next_index = 0u32;
        // The iterative search's frames: a node and the position of its next edge.
        let mut frames: Vec<(Node, usize)> = Vec::new();
        for start in 0..node_count {
            if index_of[start] != NONE || targets_start[start] == targets_start[start + 1] {
                continue;
            }
            frames.push((start, targets_start[start]));
            index_of[start] = next_index;
            low[start] = next_index;
            next_index += 1;
            component.push(start);
            on_stack[start] = true;
            while let Some(&mut (node, ref mut edge)) = frames.last_mut() {
                if *edge < targets_start[node + 1] {
                    let (target, _) = targets[*edge];
                    *edge += 1;
                    if index_of[target] == NONE {
                        index_of[target] = next_index;
                        low[target] = next_index;
                        next_index += 1;
                        component.push(target);
                        on_stack[target] = true;
                        frames.push((target, targets_start[target]));
                    } else if on_stack[target] {
                        low[node] = low[node].min(index_of[target]);
                    }
                    continue;
                }
                frames.pop();
                if let Some(&(parent, _)) = frames.last() {
                    low[parent] = low[parent].min(low[node]);
                }
                if low[node] != index_of[node] {
                    continue;
                }
                // `node` roots a component: everything above it on the stack.
                let split = component
                    .iter()
                    .rposition(|&member| member == node)
                    .expect("the root is on the stack");
                let members: Vec<Node> = component.drain(split..).collect();
                let mut extent: Option<Extent> = None;
                for &member in &members {
                    on_stack[member] = false;
                    component_of[member] = node as u32;
                }
                for &member in &members {
                    for &(target, span) in
                        &targets[targets_start[member]..targets_start[member + 1]]
                    {
                        let scope = self.scopes[self.local_scope[target] as usize];
                        let own = Extent {
                            enter: scope.enter,
                            exit: scope.exit,
                            witness: span,
                        };
                        let reached = match extents[target] {
                            // Successors in other components are finished; those in this one
                            // are covered by the component's own edges.
                            Some(further) if component_of[target] != node as u32 => {
                                own.join(further)
                            }
                            _ => own,
                        };
                        extent = Some(match extent {
                            Some(extent) => extent.join(reached),
                            None => reached,
                        });
                    }
                }
                for &member in &members {
                    extents[member] = extent;
                }
            }
        }
        extents
    }
}

/// The binding of a pattern that binds the whole matched value by value, or the payload of a
/// nullable one.
fn whole_binding(pattern: &Pattern) -> Option<LocalId> {
    match pattern {
        Pattern::Binding {
            local,
            mode: BindingMode::Move | BindingMode::Copy,
        } => Some(*local),
        Pattern::NonNull(inner) => whole_binding(inner),
        _ => None,
    }
}

enum AssignTarget {
    Binding(LocalId),
    CapturedBinding,
    Field,
    ThroughReference,
}

/// The help of `E4028`: copy a `var` into a `val`, or clone what the closure cannot take.
fn capture_help(name: &str, refusal: CaptureRefusal, cloneable: bool) -> String {
    let fresh = if name == "m" { "k" } else { "m" };
    match refusal {
        CaptureRefusal::Variable => format!(
            "copy it into a 'val' before the closure and capture that instead: 'val {} = {}'",
            fresh, name
        ),
        CaptureRefusal::DefaultValueInput => format!(
            "give '{}' to the closure when it is called, as a parameter, instead of capturing it",
            name
        ),
        _ if cloneable => format!(
            "clone it into a 'val' before the closure and capture that instead: 'val {} = {}.clone()'",
            fresh, name
        ),
        _ => format!(
            "capture a value the closure can own instead, such as a clone of '{}'",
            name
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::checker::TypeChecker;
    use prism_typed_tree::{CaptureMode, ExpressionKind, OwnershipReasonKind, TypedFile};

    /// Checks `source`, returning the typed tree and the error codes.
    fn check(source: &str) -> (TypedFile, Vec<&'static str>) {
        let file = prism_parser::parse_source(source).expect("parses");
        let mut checker = TypeChecker::new(None, "main");
        let typed = checker.check_module(&file);
        let codes = checker.errors.iter().map(|e| e.code()).collect();
        (typed, codes)
    }

    /// A closure's captures, by name, and the kind of its reason.
    type ClosureCaptures = (Vec<(String, CaptureMode)>, Option<OwnershipReasonKind>);

    /// The captures of every closure of function `name`, in creation order.
    fn closures(file: &TypedFile, name: &str) -> Vec<ClosureCaptures> {
        let body = file
            .bodies
            .iter()
            .find(|(_, body)| body.owner.name == name)
            .map(|(_, body)| body)
            .unwrap_or_else(|| panic!("no body {}", name));
        body.expressions
            .iter()
            .filter_map(|(_, e)| match &e.kind {
                ExpressionKind::Closure {
                    captures,
                    ownership_reason,
                    ..
                } => Some((
                    captures
                        .iter()
                        .map(|c| (body.locals[c.local].name.clone(), c.mode))
                        .collect(),
                    ownership_reason.as_ref().map(|r| r.kind.clone()),
                )),
                _ => None,
            })
            .collect()
    }

    use CaptureMode::{Borrowed, Copied, Moved};

    fn modes(file: &TypedFile, name: &str) -> Vec<Vec<CaptureMode>> {
        closures(file, name)
            .into_iter()
            .map(|(captures, _)| captures.into_iter().map(|(_, mode)| mode).collect())
            .collect()
    }

    const PRELUDE: &str = "struct Button { val onClick: () -> Int }\nenum Action { Run(() -> Int), Stop }\nfun greet(s: &String): Int = 1\nfun store(f: escaping () -> Int): Int = 1\nfun run(f: () -> Int): Int = f()\nfun <T> keep(x: T): T = x\n";

    fn checked(body: &str) -> TypedFile {
        let (file, codes) = check(&format!("{}{}", PRELUDE, body));
        assert!(codes.is_empty(), "{:?} for {}", codes, body);
        file
    }

    #[test]
    fn every_owning_position_owns() {
        let file = checked(
            "fun a(s: String): () -> Int = { greet(s) }\n\
             fun b(s: String): Button = Button({ greet(s) })\n\
             fun c(s: String): Action = Action.Run({ greet(s) })\n\
             fun d(s: String): Int { val h = Heap({ greet(s) }); return 1 }\n\
             fun e(slot: &mut (() -> Int), s: String) { *slot = { greet(s) } }\n\
             fun f(s: String): Int = store { greet(s) }\n\
             fun g(s: String): Int = keep({ greet(s) })()\n\
             fun h(s: String): () -> Int { return { greet(s) } }\n\
             fun i(n: Int, p: () -> Int = { n }): Int = p()\n",
        );
        for (name, kind) in [
            ("a", OwnershipReasonKind::Returned),
            ("b", OwnershipReasonKind::StoredInField),
            ("c", OwnershipReasonKind::StoredInPayload),
            ("d", OwnershipReasonKind::StoredInBox),
            ("e", OwnershipReasonKind::WrittenThroughReference),
            ("f", OwnershipReasonKind::PassedToEscapingParameter),
            ("g", OwnershipReasonKind::PassedToGenericParameter),
            ("h", OwnershipReasonKind::Returned),
            ("i", OwnershipReasonKind::Returned),
        ] {
            let found = closures(&file, name);
            assert_eq!(found.len(), 1, "{}", name);
            let (captures, reason) = &found[0];
            let expected_mode = if name == "i" { Copied } else { Moved };
            assert!(
                captures.iter().all(|(_, mode)| *mode == expected_mode),
                "{}: {:?}",
                name,
                captures
            );
            assert_eq!(reason.as_ref(), Some(&kind), "{}", name);
        }
    }

    #[test]
    fn a_closure_reached_through_locals_a_nullable_and_an_elvis() {
        let file = checked(
            "fun f(s: String): Button { val h = { greet(s) }; val k: (() -> Int)? = h; return Button(k ?: { 0 }) }\n",
        );
        assert_eq!(modes(&file, "f"), vec![vec![Moved], vec![]]);
        let (_, reason) = &closures(&file, "f")[0];
        assert_eq!(reason, &Some(OwnershipReasonKind::StoredInField));
    }

    #[test]
    fn a_nested_closure_captured_by_an_escaping_one() {
        let file = checked(
            "fun f(s: String): () -> Int { val inner = { greet(s) }; return { inner() + 1 } }\n",
        );
        let found = closures(&file, "f");
        assert_eq!(found[0].0, vec![("s".to_string(), Moved)]);
        assert_eq!(
            found[0].1,
            Some(OwnershipReasonKind::CapturedByOwningClosure)
        );
        assert_eq!(found[1].0, vec![("inner".to_string(), Moved)]);
    }

    #[test]
    fn unmarked_parameters_keep_borrowing() {
        let file = checked(
            "fun <T> remember(calc: () -> T): T = calc()\nfun take(s: String) {}\nfun f(s: String) { run { greet(s) }; remember { greet(s) }; take(s) }\n",
        );
        assert_eq!(modes(&file, "f"), vec![vec![Borrowed], vec![Borrowed]]);
    }

    #[test]
    fn a_conflict_does_not_make_a_closure_own_its_capture() {
        let file = checked(
            "fun take(s: String) {}\nfun f(s: String) { val g = { greet(s) }; take(s); run(g) }\n",
        );
        assert_eq!(modes(&file, "f"), vec![vec![Borrowed]]);
    }

    #[test]
    fn a_variable_that_may_escape_owns_every_closure_stored_in_it() {
        let file = checked(
            "fun f(s: String): Int { var h = { greet(s) }; h(); h = { 0 }; store(h); return 1 }\n",
        );
        assert_eq!(modes(&file, "f"), vec![vec![Moved], vec![]]);
        assert_eq!(
            closures(&file, "f")[0].1,
            Some(OwnershipReasonKind::PassedToEscapingParameter)
        );
    }

    #[test]
    fn a_closure_kept_past_a_block_owns_only_the_outlived_capture() {
        let file = checked(
            "fun f(): Int { var count = 0; var h: () -> Int = { 0 }; var i = 0; while (i < 3) { val s = \"a\"; h = { count += 1; greet(s) }; i += 1 }; h(); return count }\n",
        );
        let found = closures(&file, "f");
        assert_eq!(
            found[1].0,
            vec![("count".to_string(), Borrowed), ("s".to_string(), Moved)]
        );
        assert_eq!(
            found[1].1,
            Some(OwnershipReasonKind::StoredOutsideBlock {
                name: "s".to_string()
            })
        );
    }

    #[test]
    fn a_loop_binding_stored_outside_its_loop_is_owned() {
        let file = checked(
            "fun f(): Int { var h: () -> Int = { 0 }; for (i in 0..3) { h = { i + 1 } }; return h() }\n",
        );
        assert_eq!(modes(&file, "f"), vec![vec![], vec![Copied]]);
    }

    #[test]
    fn reassigned_variables_in_a_cycle_share_their_extent() {
        let file = checked(
            "fun f(c: Bool): Int { var a: () -> Int = { 0 }; if (c) { val s = \"x\"; var b: () -> Int = { greet(s) }; b = a; a = b }; return a() }\n",
        );
        // `{ greet(s) }` reaches `b`, then `a`, declared outside the block of `s`.
        assert_eq!(modes(&file, "f"), vec![vec![], vec![Moved]]);
    }

    #[test]
    fn a_closure_stored_outside_through_another_closure() {
        let file = checked(
            "fun f(c: Bool): Int { var h: () -> Int = { 0 }; if (c) { val s = \"x\"; val g = { greet(s) }; h = { g() } }; return h() }\n",
        );
        // The outer closure owns `g`, which outlives its block, so `g` owns `s`.
        assert_eq!(modes(&file, "f"), vec![vec![], vec![Moved], vec![Moved]]);
    }

    #[test]
    fn captures_that_cannot_be_taken_by_value() {
        let (_, codes) = check(&format!(
            "{}fun f(): Button {{ var n = 1; val b = Button({{ n }}); n = 2; return b }}\n",
            PRELUDE
        ));
        assert_eq!(codes, vec!["E4028"]);
        let (_, codes) = check(&format!(
            "{}fun f(s: String): Int = run {{ store {{ greet(s) }} }}\n",
            PRELUDE
        ));
        assert_eq!(codes, vec!["E4028"]);
        let (_, codes) = check(&format!(
            "{}fun g(a: String, p: () -> Int = {{ greet(a) }}): Int = p()\n",
            PRELUDE
        ));
        assert_eq!(codes, vec!["E4028"]);
        let file = checked("fun h(n: Int): Int = run { store { n } }\n");
        assert_eq!(modes(&file, "h"), vec![vec![Copied], vec![Borrowed]]);
    }
}

/// The cost of deciding captures (D2 of `escaping-closures`) on generated bodies of growing
/// size: thousands of closures, locals and reassigned `var`s in one function, some escaping,
/// some kept past a block, some reassigned in cycles. It reports the pass's time against the
/// whole check of the file and the growth from one size to the next. It is ignored by default;
/// run it on a release build with
///
/// ```text
/// cargo test --release -p prism-typechecker capture_decision_timing -- --ignored --nocapture
/// ```
///
/// With `PRISM_ASSERT_TIMING=1` it fails when the pass takes over a fifth of the check, or grows
/// more than twice as fast as the body. The generated bodies are far denser in closures than
/// real code (three closures per four statements); on them the pass took 10% to 14% of the
/// check, growing like the body (design, "Risks / Trade-offs").
#[cfg(test)]
mod timing {
    use super::{decide_captures, BodyKind};
    use crate::checker::TypeChecker;
    use std::fmt::Write;
    use std::time::{Duration, Instant};

    fn generate(n: usize) -> String {
        let mut body = String::from("    var count = 0\n");
        for i in 0..n {
            writeln!(body, "    val s{i} = \"x\"").unwrap();
            writeln!(body, "    var h{i}: () -> Int = {{ 0 }}").unwrap();
            writeln!(
                body,
                "    if (c) {{ val t{i} = \"y\"; h{i} = {{ count += 1; greet(t{i}) }} }}"
            )
            .unwrap();
            writeln!(body, "    run {{ greet(s{i}) }}").unwrap();
            if i % 2 == 0 {
                writeln!(body, "    val k{i} = {{ greet(s{i}) }}").unwrap();
                writeln!(body, "    var a{i}: () -> Int = k{i}").unwrap();
                writeln!(body, "    var b{i}: () -> Int = {{ 1 }}").unwrap();
                writeln!(body, "    b{i} = a{i}").unwrap();
                writeln!(body, "    a{i} = b{i}").unwrap();
                writeln!(body, "    store(a{i})").unwrap();
            }
        }
        format!(
            "fun greet(s: &String): Int = 1\nfun store(f: escaping () -> Int): Int = 1\nfun run(f: () -> Int): Int = f()\nfun f(c: Bool) {{\n{body}}}\n"
        )
    }

    fn measure(n: usize) -> (Duration, Duration) {
        let source = generate(n);
        let file = prism_parser::parse_source(&source).expect("parses");
        let mut checker = TypeChecker::new(None, "main");
        let start = Instant::now();
        let mut typed = checker.check_module(&file);
        let check = start.elapsed();
        assert!(checker.errors.is_empty(), "{:?}", &checker.errors[..1]);
        let types = typed.types.clone();
        let start = Instant::now();
        for (_, body) in typed.bodies.iter_mut() {
            decide_captures(body, &types, BodyKind::Function);
        }
        (start.elapsed(), check)
    }

    #[test]
    #[ignore = "a benchmark: run it on a release build with --ignored --nocapture"]
    fn capture_decision_timing() {
        let assert = std::env::var("PRISM_ASSERT_TIMING").is_ok_and(|value| value == "1");
        let mut previous: Option<Duration> = None;
        for n in [1000, 2000, 4000, 8000] {
            let (pass, check) = measure(n);
            let share = pass.as_secs_f64() / check.as_secs_f64();
            let growth = previous.map(|p| pass.as_secs_f64() / p.as_secs_f64().max(1e-9));
            println!(
                "captures n={n:5}: pass {:8.3} ms, check {:9.3} ms, share {:5.1}%{}",
                pass.as_secs_f64() * 1000.0,
                check.as_secs_f64() * 1000.0,
                share * 100.0,
                growth.map_or(String::new(), |g| format!(", growth x{g:.2}"))
            );
            if assert {
                assert!(
                    share < 0.2,
                    "the capture pass takes {:.1}% of the check",
                    share * 100.0
                );
                if let Some(growth) = growth {
                    assert!(
                        growth < 4.0 || pass < Duration::from_millis(5),
                        "the capture pass grows x{growth:.2} when the body doubles"
                    );
                }
            }
            previous = Some(pass);
        }
    }
}
