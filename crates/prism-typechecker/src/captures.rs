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
use prism_ast::{ParameterKind, Span};
use prism_typed_tree::{
    Argument, BindingMode, Body, BuiltinFunction, Callee, Capture, CaptureMode, CaptureProjection,
    ConstructKind, ExpressionId, ExpressionKind, LocalId, LocalKind, OwnershipReason,
    OwnershipReasonKind, Pattern, StatementId, StatementKind, Type, TypeTable, Widening,
    WideningReason,
};
use std::collections::HashSet;

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
    /// The bindings the closure captures, each once, in order of first use.
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
    types: &'a TypeTable,
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
    /// The bindings each closure writes, assigning them or a field of them or borrowing them
    /// mutably, itself or in a nested closure: (closure index, binding).
    writes: HashSet<(usize, LocalId)>,
    /// The closure each local is declared in, by index, `NONE` outside closures: a closure
    /// captures exactly the bindings it uses that are declared outside it.
    declared_in: Vec<u32>,
    /// Per local, its position among the captures of the closure being decided, `NONE`
    /// otherwise.
    position: Vec<u32>,
    /// Per local, the closure and the index in its `paths` of its last recorded whole use,
    /// which later whole uses in the same closure only update.
    whole_use: Vec<(u32, u32)>,
    /// The paths each closure's own body reads of the bindings it captures, in walk order, by
    /// closure index (D5 of `path-captures`).
    paths: Vec<Vec<PathEntry>>,
    /// A buffer for the nodes of a place chain.
    chain: Vec<ExpressionId>,
}

/// A captured path, as recorded or as decided (D1 to D3 of `path-captures`).
#[derive(Clone)]
struct PathEntry {
    local: LocalId,
    projections: Vec<CaptureProjection>,
    /// The first use of the path or of a path it covers.
    span: Span,
    /// The node the path ends at, whose type is the path's; `None` for a whole binding.
    node: Option<ExpressionId>,
    /// Whether the path dereferences a `&mut` reference.
    mutable_reference: bool,
    /// Whether the path reads a `var` field after dereferencing a reference.
    variable_field: bool,
    /// For a whole binding a nested closure owns, the path it was widened from.
    widened: Option<Widening>,
    /// How the closure takes the path, once decided.
    mode: CaptureMode,
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
        let mut graph = Graph::new(body, types);
        graph.walk_body(kind);
        graph.decide()
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
    fn new(body: &'a Body, types: &'a TypeTable) -> Self {
        let local_count = body.locals.len();
        Self {
            body,
            types,
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
            writes: HashSet::new(),
            declared_in: vec![NONE; local_count],
            position: vec![NONE; local_count],
            whole_use: vec![(NONE, NONE); local_count],
            paths: Vec::new(),
            chain: Vec::new(),
        }
    }

    /// Records, for the innermost closure being walked, the path that the place chain topped by
    /// `id`, a field or dereference, reads (D1 and D5 of `path-captures`), when the chain bottoms
    /// at a binding the closure captures, and returns whether it did; the chain's interior holds
    /// nothing else to walk.
    fn record_chain(&mut self, id: ExpressionId) -> bool {
        let Some(&closure) = self.closure_stack.last() else {
            return false;
        };
        let body = self.body;
        let mut chain = std::mem::take(&mut self.chain);
        chain.clear();
        let mut current = id;
        let local = loop {
            match &body.expressions[current].kind {
                ExpressionKind::Field { base, .. } => {
                    chain.push(current);
                    current = *base;
                }
                ExpressionKind::Deref { pointer } => {
                    chain.push(current);
                    current = *pointer;
                }
                ExpressionKind::Local(local) if self.captures(closure, *local) => {
                    break Some(*local)
                }
                _ => break None,
            }
        };
        let Some(local) = local else {
            self.chain = chain;
            return false;
        };
        let binding = &body.locals[local];
        let mut entry = PathEntry {
            local,
            projections: Vec::with_capacity(chain.len()),
            span: body.expressions[current].span,
            node: None,
            mutable_reference: false,
            variable_field: false,
            widened: None,
            mode: CaptureMode::Borrowed,
        };
        let mut through_reference = false;
        for &node in chain.iter().rev() {
            let projection = match &body.expressions[node].kind {
                ExpressionKind::Deref { pointer } => match &body.expressions[*pointer].ty {
                    Type::Heap(_) => CaptureProjection::Deref,
                    // Only the binding's own first dereference, of a `val` reference: any other
                    // reference may be re-pointed while the closure lives (D1).
                    Type::Reference { mutable, .. }
                        if entry.projections.is_empty() && !binding.mutable =>
                    {
                        through_reference = true;
                        entry.mutable_reference = *mutable;
                        CaptureProjection::Deref
                    }
                    _ => break,
                },
                ExpressionKind::Field { base, index, .. } => {
                    let Type::Struct { id: nominal, .. } = &body.expressions[*base].ty else {
                        break;
                    };
                    if through_reference {
                        let variable = self
                            .types
                            .struct_fields(nominal)
                            .and_then(|fields| fields.get(*index))
                            .is_some_and(|(_, variable, _)| *variable);
                        entry.variable_field |= variable;
                    }
                    CaptureProjection::Field(*index)
                }
                _ => unreachable!("a place chain holds fields and dereferences"),
            };
            entry.projections.push(projection);
            entry.span = body.expressions[node].span;
            entry.node = Some(node);
        }
        self.paths[closure].push(entry);
        self.chain = chain;
        true
    }

    /// Records the innermost closure being walked as reading the whole binding `local`.
    fn record_whole(&mut self, local: LocalId, span: Span) {
        let Some(&closure) = self.closure_stack.last() else {
            return;
        };
        if !self.captures(closure, local) {
            return;
        }
        let (owner, last) = self.whole_use[local.0 as usize];
        if owner == closure as u32 {
            if let Some(entry) = self.paths[closure].get_mut(last as usize) {
                if span.start < entry.span.start {
                    entry.span = span;
                }
                return;
            }
        }
        let paths = &mut self.paths[closure];
        self.whole_use[local.0 as usize] = (closure as u32, paths.len() as u32);
        paths.push(PathEntry::whole(local, span));
    }

    /// Whether closure `closure` captures `local`, which it uses: whether `local` is declared
    /// outside it.
    fn captures(&self, closure: usize, local: LocalId) -> bool {
        self.declared_in
            .get(local.0 as usize)
            .is_some_and(|&declared| declared != closure as u32)
    }

    // --- Recording

    /// Records that the closures being walked write the binding at the root of `place`, when
    /// it is a binding or a field path of one.
    fn written(&mut self, place: ExpressionId) {
        if self.closure_stack.is_empty() {
            return;
        }
        let mut place = place;
        let local = loop {
            match &self.body.expressions[place].kind {
                ExpressionKind::Local(local) => break *local,
                ExpressionKind::Field { base, .. } => place = *base,
                _ => return,
            }
        };
        for &closure in &self.closure_stack {
            self.writes.insert((closure, local));
        }
    }

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
        let closure = self
            .closure_stack
            .last()
            .map_or(NONE, |&closure| closure as u32);
        if let Some(slot) = self.declared_in.get_mut(local.0 as usize) {
            *slot = closure;
        }
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
            // The checker records each captured binding once, whole.
            debug_assert!(captures.iter().all(|c| c.projections.is_empty()));
            self.paths.push(Vec::new());
            self.closures.push(ClosureInfo {
                expression,
                parent: self.closure_stack.last().copied(),
                in_default: self.in_default && self.closure_stack.is_empty(),
                captures: captures.as_slice(),
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
            ExpressionKind::Local(local) => {
                let span = body.expressions[id].span;
                self.record_whole(*local, span);
            }
            ExpressionKind::Field { .. } | ExpressionKind::Deref { .. }
                if self.record_chain(id) => {}
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
                // The closure declares its parameters: they are none of its captures.
                self.closure_stack.push(node - self.local_count);
                for parameter in parameters {
                    self.declare(*parameter);
                }
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
                    } else if let Some(kind) =
                        call.parameter_kinds.get(index).and_then(|kind| match kind {
                            ParameterKind::Escaping => {
                                Some(OwnershipReasonKind::PassedToEscapingParameter)
                            }
                            ParameterKind::Attached => {
                                Some(OwnershipReasonKind::PassedToAttachedParameter)
                            }
                            ParameterKind::Plain => None,
                        })
                    {
                        Some(kind)
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
            ExpressionKind::Borrow {
                mutable: true,
                place,
                ..
            } => {
                self.written(*place);
                self.expression(*place);
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
                self.written(*target);
                match self.assigned_binding(*target) {
                    AssignTarget::Binding(local) => self.store(*value, local, statement.span),
                    AssignTarget::CapturedBinding => {
                        self.own(*value, OwnershipReasonKind::AssignedToCapturedVariable)
                    }
                    AssignTarget::Field => self.own(*value, OwnershipReasonKind::StoredInField),
                    AssignTarget::ThroughReference => {
                        self.own(*value, OwnershipReasonKind::WrittenThroughReference)
                    }
                    AssignTarget::State => self.own(*value, OwnershipReasonKind::StoredInState),
                }
                self.expression(*target);
                self.expression(*value);
            }
            StatementKind::For {
                variable,
                iterable,
                body: loop_body,
                ..
            } => {
                self.expression(*iterable);
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
                self.written(*target);
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
            // A state keeps its value beyond any block (D18 of `state-handles`).
            ExpressionKind::StatePlace { .. } => AssignTarget::State,
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

    fn decide(mut self) -> Vec<Decision> {
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

        // Each closure's captures, innermost first, so that a closure's paths include its
        // children's (D5 of `path-captures`): a closure's decided paths go to its parent's
        // pending ones.
        let mut recorded = std::mem::take(&mut self.paths);
        let mut position = std::mem::take(&mut self.position);
        let mut pending: Vec<Vec<PathEntry>> = vec![Vec::new(); self.closures.len()];
        let mut entries: Vec<PathEntry> = Vec::new();
        let mut merged: Vec<PathEntry> = Vec::new();
        let mut decisions = Vec::with_capacity(self.closures.len());
        for index in (0..self.closures.len()).rev() {
            let closure = &self.closures[index];
            let node = self.local_count + index;
            let (owns_all, mut reason) = match escaping[node] {
                NONE => (false, outlived_reasons[index].clone()),
                found => (true, Some(reasons[found as usize].clone())),
            };
            entries.clear();
            entries.append(&mut recorded[index]);
            entries.append(&mut pending[index]);
            merged.clear();
            merge_paths(closure.captures, &mut entries, &mut merged, &mut position);
            let mut captures = Vec::with_capacity(merged.len());
            let mut errors = Vec::new();
            let mut start = 0;
            // The k-th run of `merged` holds the paths of the k-th capture of the checker.
            for (ordinal, _) in closure.captures.iter().enumerate() {
                let end = start
                    + merged[start..]
                        .iter()
                        .take_while(|entry| entry.local == merged[start].local)
                        .count();
                let group = &mut merged[start..end];
                start = end;
                let local_id = group[0].local;
                let owned = owns_all || outlived[index][ordinal];
                let decided = if !owned {
                    for entry in group.iter_mut() {
                        entry.mode = CaptureMode::Borrowed;
                    }
                    None
                } else {
                    self.decide_owned(index, local_id, group, &reason, &mut errors)
                };
                let finals: &[PathEntry] = match &decided {
                    Some(whole) => std::slice::from_ref(whole),
                    None => group,
                };
                for entry in finals {
                    captures.push(entry.capture());
                    if let Some(parent) = closure.parent {
                        if self.captures(parent, entry.local) {
                            let mut entry = entry.clone();
                            entry.mode = CaptureMode::Borrowed;
                            pending[parent].push(entry);
                        }
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

    /// Decides how closure `index`, which owns the binding `local_id`, takes the paths `group`
    /// of it: copies each one, setting its mode, when it can (D3 of `path-captures`) and
    /// returns `None`, or returns the whole binding, by today's rules, reporting `E4028` into
    /// `errors` when it cannot take it.
    fn decide_owned(
        &self,
        index: usize,
        local_id: LocalId,
        group: &mut [PathEntry],
        reason: &Option<OwnershipReason>,
        errors: &mut Vec<TypeError>,
    ) -> Option<PathEntry> {
        let types = self.types;
        let closure = &self.closures[index];
        let local = &self.body.locals[local_id];
        let widening = if group.len() == 1 && group[0].projections.is_empty() {
            None
        } else {
            group.iter().find_map(|entry| {
                entry
                    .refusal(local.mutable, self.body, types)
                    .map(|reason| Widening {
                        projections: entry.projections.clone(),
                        reason,
                    })
            })
        };
        if !group[0].projections.is_empty() && widening.is_none() {
            for entry in group.iter_mut() {
                entry.mode = CaptureMode::Copied;
            }
            return None;
        }
        let span = group
            .iter()
            .map(|entry| entry.span)
            .min_by_key(|span| span.start)
            .expect("a group has a path");
        let widened = widening.or_else(|| {
            // A whole binding a nested closure took for a path keeps that path, when this
            // closure reads nothing else of it.
            (group.len() == 1)
                .then(|| group[0].widened.clone())
                .flatten()
        });
        let mut entry = PathEntry::whole(local_id, span);
        entry.widened = widened;
        let copy = types.is_copy(&local.ty);
        let refusal = if local.mutable {
            Some(CaptureRefusal::Variable)
        } else if copy {
            None
        } else if self.captured_by_parent(index, local_id) {
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
                entry.mode = CaptureMode::Borrowed;
                entry.widened = None;
                let owning = reason.clone().expect("an owned capture has a reason");
                // A `var` read through a path is copied by that path (D10).
                let path = match (widened_path(group), refusal) {
                    (Some(projections), CaptureRefusal::Variable) => Some(
                        prism_typed_tree::path_text(&local.name, &local.ty, projections, types),
                    ),
                    _ => None,
                };
                errors.push(TypeError::CannotCaptureByValue {
                    name: local.name.clone(),
                    refusal,
                    help: capture_help(
                        &local.name,
                        path.as_deref(),
                        refusal,
                        types.is_clone(&local.ty),
                        self.writes.contains(&(index, local_id)),
                    ),
                    reason: owning,
                    span,
                });
            }
            None => {
                entry.mode = if copy {
                    CaptureMode::Copied
                } else {
                    CaptureMode::Moved
                };
            }
        }
        Some(entry)
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

impl PathEntry {
    fn whole(local: LocalId, span: Span) -> Self {
        PathEntry {
            local,
            projections: Vec::new(),
            span,
            node: None,
            mutable_reference: false,
            variable_field: false,
            widened: None,
            mode: CaptureMode::Borrowed,
        }
    }

    fn capture(&self) -> Capture {
        Capture {
            local: self.local,
            projections: self.projections.clone(),
            span: self.span,
            mode: self.mode,
            widened: self.widened.clone(),
        }
    }

    /// Why an owning closure cannot copy this path, which has projections: its type is not
    /// `Copy`, or a later write could change it (D3 of `path-captures`).
    fn refusal(&self, variable: bool, body: &Body, types: &TypeTable) -> Option<WideningReason> {
        let ty = match self.node {
            Some(node) => &body.expressions[node].ty,
            None => &body.locals[self.local].ty,
        };
        if variable {
            Some(WideningReason::VariableBinding)
        } else if self.node.is_none() || !types.is_copy(ty) {
            Some(WideningReason::NotCopy)
        } else if self.mutable_reference {
            Some(WideningReason::MutableReference)
        } else if self.variable_field {
            Some(WideningReason::VariableField)
        } else {
            None
        }
    }
}

/// Merges the paths `entries` of a closure into `merged`, grouped by binding in the order of
/// `captures` (the bindings in order of first use), each group with no path a prefix of another
/// and in the order of the paths' steps, fields in declaration order (D2 of `path-captures`). A
/// captured binding with no recorded path is captured whole. Sorting puts a path right before
/// the paths it is a prefix of, so one scan merges them. `position` is all `NONE` before and
/// after.
fn merge_paths(
    captures: &[Capture],
    entries: &mut Vec<PathEntry>,
    merged: &mut Vec<PathEntry>,
    position: &mut [u32],
) {
    for (index, capture) in captures.iter().enumerate() {
        position[capture.local.0 as usize] = index as u32;
    }
    let at = |position: &[u32], local: LocalId| position[local.0 as usize];
    entries.retain(|entry| at(position, entry.local) != NONE);
    entries.sort_by(|a, b| {
        (at(position, a.local), &a.projections).cmp(&(at(position, b.local), &b.projections))
    });
    let mut next = 0;
    for entry in entries.drain(..) {
        let ordinal = at(position, entry.local) as usize;
        while next < ordinal {
            merged.push(PathEntry::whole(captures[next].local, captures[next].span));
            next += 1;
        }
        if ordinal < next {
            let last = merged.last_mut().expect("an earlier path of the binding");
            if entry.projections.starts_with(&last.projections) {
                if entry.span.start < last.span.start {
                    last.span = entry.span;
                }
                // A prefix taken whole by a nested closure keeps its reason only when nothing
                // longer was merged into it.
                if entry.projections.len() > last.projections.len() {
                    last.widened = None;
                }
                continue;
            }
        }
        merged.push(entry);
        next = ordinal + 1;
    }
    while next < captures.len() {
        merged.push(PathEntry::whole(captures[next].local, captures[next].span));
        next += 1;
    }
    for capture in captures {
        position[capture.local.0 as usize] = NONE;
    }
}

/// The first path of `group` with projections, which a whole capture was widened from.
fn widened_path(group: &[PathEntry]) -> Option<&[CaptureProjection]> {
    if group.iter().any(|entry| entry.projections.is_empty()) {
        return None;
    }
    group.first().map(|entry| entry.projections.as_slice())
}

enum AssignTarget {
    Binding(LocalId),
    CapturedBinding,
    Field,
    ThroughReference,
    /// The value of a state.
    State,
}

/// The help of `E4028`: copy a `var` into a `val`, or clone what the closure cannot take. A
/// `var` the closure writes cannot be copied into a `val`, which the closure could not write.
fn capture_help(
    name: &str,
    path: Option<&str>,
    refusal: CaptureRefusal,
    cloneable: bool,
    written: bool,
) -> String {
    let fresh = if name == "m" { "k" } else { "m" };
    match refusal {
        CaptureRefusal::Variable if !written && path.is_some() => {
            let path = path.unwrap_or(name).trim_start_matches('*');
            let field = path.rsplit('.').next().unwrap_or(fresh);
            let fresh = if field == name { fresh } else { field };
            format!(
                "copy it into a 'val' before the closure and capture that instead: 'val {} = {}'",
                fresh, path
            )
        }
        CaptureRefusal::Variable if written => format!(
            "the closure writes '{}', and a closure cannot write what it owns: keep it from escaping, so that it borrows '{}'",
            name, name
        ),
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
        let mut checker =
            TypeChecker::new(None, "main").with_prelude(crate::prelude::test_prelude());
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

    /// The captures of every closure of function `name`, as their paths' text and modes.
    fn paths(file: &TypedFile, name: &str) -> Vec<Vec<(String, CaptureMode)>> {
        let body = file
            .bodies
            .iter()
            .find(|(_, body)| body.owner.name == name)
            .map(|(_, body)| body)
            .unwrap_or_else(|| panic!("no body {}", name));
        body.expressions
            .iter()
            .filter_map(|(_, e)| match &e.kind {
                ExpressionKind::Closure { captures, .. } => Some(
                    captures
                        .iter()
                        .map(|c| {
                            let local = &body.locals[c.local];
                            (c.path_text(&local.name, &local.ty, &file.types), c.mode)
                        })
                        .collect(),
                ),
                _ => None,
            })
            .collect()
    }

    fn path_list(found: &[(String, CaptureMode)]) -> Vec<(&str, CaptureMode)> {
        found
            .iter()
            .map(|(path, mode)| (path.as_str(), *mode))
            .collect()
    }

    const PATH_TYPES: &str = "struct P { var x: Int, var y: Int }\nstruct Q { val p: P, val n: Int }\nstruct Todo { val id: Int, val title: String }\nstruct Cell { val inner: Heap<P> }\nstruct Bag { val items: List<Int>, val tag: P? }\nstruct Counter { var n: Int, val label: String }\nfun size(p: &P): Int = p.x\n";

    fn checked_paths(body: &str) -> TypedFile {
        checked(&format!("{}{}", PATH_TYPES, body))
    }

    #[test]
    fn a_closure_captures_the_fields_it_uses() {
        let file =
            checked_paths("fun f(): Int { var p = P(1, 2); return run { p.y + p.x + p.y } }\n");
        assert_eq!(
            path_list(&paths(&file, "f")[0]),
            vec![("p.x", Borrowed), ("p.y", Borrowed)]
        );
    }

    #[test]
    fn a_use_of_the_whole_binding_subsumes_its_paths() {
        let file = checked_paths(
            "fun f(q: Q): Int = run { q.p.x + size(q.p) + q.n }\nfun g(q: Q): Int = run { q.n + size(q.p) + q.p.y + q.p.x }\nfun h(q: Q): Int = run { q.p.x + q.n + measure(q) }\nfun measure(q: &Q): Int = 1\n",
        );
        for name in ["f", "g"] {
            assert_eq!(
                path_list(&paths(&file, name)[0]),
                vec![("q.p", Borrowed), ("q.n", Borrowed)],
                "{}",
                name
            );
        }
        assert_eq!(path_list(&paths(&file, "h")[0]), vec![("q", Borrowed)]);
    }

    #[test]
    fn paths_go_through_val_references_and_boxes() {
        let file = checked_paths(
            "fun f(t: &Todo, c: &Cell, r: &Int): Int = run { t.id + c.inner.x + r }\n",
        );
        assert_eq!(
            path_list(&paths(&file, "f")[0]),
            vec![
                ("t.id", Borrowed),
                ("c.inner.x", Borrowed),
                ("*r", Borrowed)
            ]
        );
    }

    #[test]
    fn paths_stop_at_indices_safe_navigations_states_and_other_references() {
        let file = checked_paths(
            "fun f(b: &Bag, i: Int, rr: &&P): Int = run { b.items[i] + (b.tag?.x ?: 0) + rr.x }\nfun g(a: &Todo): Int { var r = a; return run { r.id } }\nfun h(s: MutableState<Int>, t: &Todo): Int = run { s.value + t.id }\n",
        );
        assert_eq!(
            path_list(&paths(&file, "f")[0]),
            vec![
                ("b.items", Borrowed),
                ("b.tag", Borrowed),
                ("i", Borrowed),
                ("*rr", Borrowed)
            ]
        );
        assert_eq!(path_list(&paths(&file, "g")[0]), vec![("r", Borrowed)]);
        assert_eq!(
            path_list(&paths(&file, "h")[0]),
            vec![("s", Borrowed), ("t.id", Borrowed)]
        );
    }

    #[test]
    fn owning_closures_copy_the_paths_they_can() {
        let file = checked_paths(
            "fun a(t: &Todo): Button = Button({ t.id * 2 })\n\
             fun b(r: &Int): () -> Int = { r }\n\
             fun c(u: Todo): Button = Button({ u.id + 1 })\n\
             fun d(c: Counter): Button = Button({ c.n })\n\
             fun e(u: Todo): () -> Int = { u.title.length + u.id }\n\
             fun g(c: &Counter): () -> Int = { c.n }\n\
             fun h(t: &mut Todo): () -> Int = { t.id }\n",
        );
        assert_eq!(path_list(&paths(&file, "a")[0]), vec![("t.id", Copied)]);
        assert_eq!(path_list(&paths(&file, "b")[0]), vec![("*r", Copied)]);
        assert_eq!(path_list(&paths(&file, "c")[0]), vec![("u.id", Copied)]);
        assert_eq!(path_list(&paths(&file, "d")[0]), vec![("c.n", Copied)]);
        assert_eq!(path_list(&paths(&file, "e")[0]), vec![("u", Moved)]);
        // References are `Copy`: the closure owns the reference itself, which the borrow
        // checker rejects as it escapes.
        assert_eq!(path_list(&paths(&file, "g")[0]), vec![("c", Copied)]);
        assert_eq!(path_list(&paths(&file, "h")[0]), vec![("t", Moved)]);
    }

    #[test]
    fn widened_captures_record_the_path_and_why() {
        use prism_typed_tree::{CaptureProjection, WideningReason};
        let file = checked_paths(
            "fun e(u: Todo): () -> Int = { u.id + u.title.length }\n\
             fun g(c: &Counter): () -> Int = { c.n }\n\
             fun h(t: &mut Todo): () -> Int = { t.id }\n",
        );
        let widening = |name: &str| {
            let (_, body) = file
                .bodies
                .iter()
                .find(|(_, body)| body.owner.name == name)
                .unwrap();
            body.expressions
                .iter()
                .find_map(|(_, e)| match &e.kind {
                    ExpressionKind::Closure { captures, .. } => captures[0].widened.clone(),
                    _ => None,
                })
                .expect("a widened capture")
        };
        let found = widening("e");
        assert_eq!(found.projections, vec![CaptureProjection::Field(1)]);
        assert_eq!(found.reason, WideningReason::NotCopy);
        assert_eq!(widening("g").reason, WideningReason::VariableField);
        assert_eq!(widening("h").reason, WideningReason::MutableReference);
    }

    /// Closures are listed inner first, in the order their nodes were built.
    #[test]
    fn nested_closures_take_their_children_s_paths() {
        let file = checked_paths(
            "fun f(t: &Todo): Int = run { store { t.id } }\nfun g(u: Todo): Int = run { store { u.id } }\nfun h(t: &Todo): Int = run { t.title.length + store { t.id } }\n",
        );
        assert_eq!(
            paths(&file, "f")
                .iter()
                .map(|c| path_list(c))
                .collect::<Vec<_>>(),
            vec![vec![("t.id", Copied)], vec![("t.id", Borrowed)]]
        );
        assert_eq!(
            paths(&file, "g")
                .iter()
                .map(|c| path_list(c))
                .collect::<Vec<_>>(),
            vec![vec![("u.id", Copied)], vec![("u.id", Borrowed)]]
        );
        assert_eq!(
            paths(&file, "h")
                .iter()
                .map(|c| path_list(c))
                .collect::<Vec<_>>(),
            vec![
                vec![("t.id", Copied)],
                vec![("t.id", Borrowed), ("t.title", Borrowed)]
            ]
        );
    }

    #[test]
    fn a_var_read_through_a_path_is_refused_naming_the_path() {
        let (file, codes) = check(&format!(
            "{}{}fun f(): Button {{ var p = P(1, 2); val b = Button({{ p.x }}); p = P(2, 3); return b }}\n",
            PRELUDE, PATH_TYPES
        ));
        assert_eq!(codes, vec!["E4028"]);
        assert_eq!(path_list(&paths(&file, "f")[0]), vec![("p", Borrowed)]);
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
/// some kept past a block, some reassigned in cycles, and field paths through references and
/// boxes, borrowed, copied and taken from nested closures (`path-captures`). It reports the pass's time against the
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
    use prism_typed_tree::{Capture, CaptureMode, ExpressionKind};
    use std::collections::HashSet;
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
            // Field paths, through references and boxes, borrowed and copied (`path-captures`).
            writeln!(body, "    val p{i} = P({i}, {i}, Heap(R({i})))").unwrap();
            writeln!(body, "    run {{ p{i}.x + p{i}.inner.z + t.id }}").unwrap();
            writeln!(body, "    store {{ p{i}.y + t.id + r + run {{ p{i}.x }} }}").unwrap();
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
            "struct R {{ val z: Int }}\nstruct P {{ val x: Int, val y: Int, val inner: Heap<R> }}\nstruct Todo {{ val id: Int }}\nfun greet(s: &String): Int = 1\nfun store(f: escaping () -> Int): Int = 1\nfun run(f: () -> Int): Int = f()\nfun f(c: Bool, t: &Todo, r: &Int) {{\n{body}}}\n"
        )
    }

    fn measure(n: usize) -> (Duration, Duration) {
        let source = generate(n);
        let file = prism_parser::parse_source(&source).expect("parses");
        let mut checker =
            TypeChecker::new(None, "main").with_prelude(crate::prelude::test_prelude());
        let start = Instant::now();
        let mut typed = checker.check_module(&file);
        let check = start.elapsed();
        assert!(checker.errors.is_empty(), "{:?}", &checker.errors[..1]);
        let types = typed.types.clone();
        // The pass's input as the checker builds it: each closure's captured bindings, whole and
        // borrowed, once each in first-use order.
        for (_, body) in typed.bodies.iter_mut() {
            for (_, expression) in body.expressions.iter_mut() {
                if let ExpressionKind::Closure { captures, .. } = &mut expression.kind {
                    let mut seen = HashSet::new();
                    captures.retain(|capture| seen.insert(capture.local));
                    for capture in captures.iter_mut() {
                        *capture =
                            Capture::whole(capture.local, capture.span, CaptureMode::Borrowed);
                    }
                }
            }
        }
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
