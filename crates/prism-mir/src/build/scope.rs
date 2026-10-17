//! Scopes, storage markers and scope-exit drops (D6 and D7 of `mir-lowering`).
//!
//! Every local and temporary belongs to the innermost scope open when it is created: a body's
//! arguments to its first scope, a binding to its block's scope, a temporary to the innermost
//! temporary scope (a statement, a block's tail, a body's value, or a part evaluated
//! conditionally or repeatedly). Where a scope ends, by falling through or by a jump out of it,
//! each of its locals is dropped, in reverse order of creation, when its type needs a drop, then
//! marked dead. Those drops and storage ends carry the span of where control leaves the scope
//! (D15 of `borrowck-exclusivity`): a braced block's closing brace when it falls through, the
//! jump's span for a `return`, `break` or `continue`, and otherwise the scope's own span.

use super::Builder;
use crate::mir::{
    LocalDeclaration, LocalId, LocalKind, Operand, Place, Rvalue, StatementKind, TerminatorKind,
};
use prism_ast::Span;
use prism_typed_tree::Type;

/// A local owned by a scope.
#[derive(Debug, Clone, Copy)]
struct ScopedLocal {
    local: LocalId,
    /// Whether the lowering moved the whole temporary out itself, so it needs no drop (D6).
    moved: bool,
    /// Whether the local has storage markers (every local but the arguments).
    marked: bool,
}

/// A scope: the locals it owns, in creation order, and the span its exit statements carry when
/// control falls through its end.
#[derive(Debug, Clone)]
pub(super) struct Scope {
    exit: Span,
    locals: Vec<ScopedLocal>,
}

impl Scope {
    /// A scope whose exit statements carry `exit` when it falls through.
    pub(super) fn new(exit: Span) -> Self {
        Self {
            exit,
            locals: Vec::new(),
        }
    }

    /// Adds a local without storage markers (an argument).
    pub(super) fn add_unmarked(&mut self, local: LocalId) {
        self.locals.push(ScopedLocal {
            local,
            moved: false,
            marked: false,
        });
    }
}

impl Builder<'_> {
    /// Opens a scope that is not a braced block: its exit statements carry `span`.
    pub(super) fn push_scope(&mut self, span: Span) {
        self.scopes.push(Scope::new(span));
    }

    /// Opens the scope of a braced block, whose exit statements carry the span of its closing
    /// brace `close_brace` when it falls through.
    pub(super) fn push_block_scope(&mut self, close_brace: Span) {
        self.scopes.push(Scope::new(close_brace));
    }

    /// Ends the innermost scope: drops and marks dead its locals on the current path.
    pub(super) fn pop_scope(&mut self) {
        let index = self.scopes.len() - 1;
        self.exit_scope(index, None);
        self.scopes.pop();
    }

    /// Removes the innermost scope without emitting anything: its locals are ended by the caller
    /// on each path that leaves it.
    pub(super) fn discard_scope(&mut self) {
        self.scopes.pop();
    }

    /// The number of open scopes.
    pub(super) fn scope_depth(&self) -> usize {
        self.scopes.len()
    }

    /// Emits, on the current path, the drops and storage markers of every scope from the
    /// innermost down to the one at index `depth`, leaving them open: the exit of a jump to a
    /// point where only `depth` scopes are open. They carry the span of the jump `jump`, or
    /// each scope's own exit span without one.
    pub(super) fn exit_scopes_to(&mut self, depth: usize, jump: Option<Span>) {
        for index in (depth..self.scopes.len()).rev() {
            self.exit_scope(index, jump);
        }
    }

    /// Emits the drops and storage markers of the scope at `index`, in reverse creation order,
    /// at the span of the jump `jump` that leaves it, or at its exit span without one.
    pub(super) fn exit_scope(&mut self, index: usize, jump: Option<Span>) {
        let span = jump.unwrap_or(self.scopes[index].exit);
        let locals = self.scopes[index].locals.clone();
        for scoped in locals.iter().rev() {
            if !scoped.moved {
                let ty = self.local_type(scoped.local).clone();
                if self.needs_drop(&ty) {
                    self.emit_drop(Place::local(scoped.local), span);
                }
            }
            if scoped.marked {
                self.push_statement(StatementKind::StorageDead(scoped.local), span);
            }
        }
    }

    /// The number of locals the scope at `index` owns so far.
    pub(super) fn scope_local_count(&self, index: usize) -> usize {
        self.scopes[index].locals.len()
    }

    /// Marks live, on the current path, the locals of the scope at `index` after its first
    /// `count`: a jump that skipped their declarations to a point where they are in scope (a
    /// `continue` to a `do`/`while` condition) leaves them live and uninitialized.
    pub(super) fn mark_live_from(&mut self, index: usize, count: usize, span: Span) {
        let locals = self.scopes[index].locals[count..].to_vec();
        for scoped in locals {
            if scoped.marked {
                self.push_statement(StatementKind::StorageLive(scoped.local), span);
            }
        }
    }

    /// Ends the current block with a drop of `place` and continues in a new block.
    pub(super) fn emit_drop(&mut self, place: Place, span: Span) {
        if self.current.is_none() {
            return;
        }
        let target = self.new_block();
        self.terminate(TerminatorKind::Drop { place, target }, span);
        self.current = Some(target);
    }

    /// Adds `local` to the innermost scope and marks it live.
    pub(super) fn schedule(&mut self, local: LocalId, span: Span) {
        let scope = self.scopes.last_mut().expect("a scope is open");
        scope.locals.push(ScopedLocal {
            local,
            moved: false,
            marked: true,
        });
        self.push_statement(StatementKind::StorageLive(local), span);
    }

    /// A new temporary of type `ty` in the innermost scope, marked live. After a diverging
    /// expression, where nothing is lowered, it is the return place, which nothing reads.
    pub(super) fn new_temporary(&mut self, ty: Type, span: Span) -> LocalId {
        if self.current.is_none() {
            return LocalId::RETURN;
        }
        let local = self.add_local(LocalDeclaration {
            ty,
            kind: LocalKind::Temporary,
            mutable: false,
            name: None,
            binding: None,
            span,
        });
        self.schedule(local, span);
        local
    }

    /// Records that `local`, a temporary of the innermost scope, was moved out of as a whole by
    /// the lowering itself, so its scope does not drop it (D6).
    pub(super) fn record_moved(&mut self, local: LocalId) {
        if let Some(scope) = self.scopes.last_mut() {
            if let Some(scoped) = scope.locals.iter_mut().find(|s| s.local == local) {
                scoped.moved = true;
            }
        }
    }

    /// Records that `local`, a temporary of any open scope, was moved out of as a whole by the
    /// lowering itself on every path where it holds a value, so its scope does not drop it.
    pub(super) fn record_moved_in_any_scope(&mut self, local: LocalId) {
        if self.locals[local].kind != LocalKind::Temporary {
            return;
        }
        for scope in self.scopes.iter_mut().rev() {
            if let Some(scoped) = scope.locals.iter_mut().find(|s| s.local == local) {
                scoped.moved = true;
                return;
            }
        }
    }

    /// Whether `local` is a temporary owned by the innermost scope.
    pub(super) fn is_temporary_of_innermost_scope(&self, local: LocalId) -> bool {
        self.locals[local].kind == LocalKind::Temporary
            && self
                .scopes
                .last()
                .is_some_and(|scope| scope.locals.iter().any(|s| s.local == local))
    }

    /// Records the temporaries `operand` moves out of as a whole.
    pub(super) fn record_operand_moved(&mut self, operand: &Operand) {
        if let Operand::Move(place, _) = operand {
            if let Some(local) = place.as_local() {
                if self.locals[local].kind == LocalKind::Temporary {
                    self.record_moved(local);
                }
            }
        }
    }

    /// Records the temporaries the operands of `rvalue` move out of as a whole.
    pub(super) fn record_operands_moved(&mut self, rvalue: &Rvalue) {
        match rvalue {
            Rvalue::Use(operand)
            | Rvalue::Unary(_, operand)
            | Rvalue::WrapNullable(operand)
            | Rvalue::Subtype(operand) => self.record_operand_moved(operand),
            Rvalue::Binary(_, left, right) => {
                self.record_operand_moved(left);
                self.record_operand_moved(right);
            }
            Rvalue::Aggregate(_, operands) => {
                for operand in operands {
                    self.record_operand_moved(operand);
                }
            }
            Rvalue::Borrow { .. } | Rvalue::Discriminant(_) | Rvalue::Closure { .. } => {}
        }
    }

    /// Moves the locals of the scope at `index` to the scope below it, which then owns them: the
    /// temporaries a `?.` chain evaluated before its first test belong to the enclosing scope
    /// (D7).
    pub(super) fn move_locals_to_parent(&mut self, index: usize) {
        let locals = std::mem::take(&mut self.scopes[index].locals);
        self.scopes[index - 1].locals.extend(locals);
    }
}
