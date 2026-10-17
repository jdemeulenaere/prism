//! Statement syntax trees and blocks for Prism.

use crate::expr::Expr;
use crate::span::Span;
use crate::types::Type;
use std::fmt;

/// A block: statements followed by an optional tail expression, which gives the block its
/// value. A block without a tail (empty, or ending in a declaration, an assignment or a loop)
/// has the value `Unit`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

impl Block {
    /// A short description for diagnostics and debug output, such as `... (2 statements)`.
    pub fn summary(&self) -> String {
        let tail = if self.tail.is_some() { ", tail" } else { "" };
        format!("... ({} statements{})", self.statements.len(), tail)
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{ {} }}", self.summary())
    }
}

/// The specific kind of a statement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StatementKind {
    /// A local `val`/`var` declaration.
    Local {
        is_var: bool,
        name: String,
        ty: Option<Type>,
        initializer: Expr,
    },
    /// An assignment `target = value`. Assignments are statements, never expressions.
    Assign { target: Expr, value: Expr },
    /// An expression whose value is discarded.
    Expression(Expr),
    /// `while (condition) body`.
    While { condition: Expr, body: Block },
    /// `do body while (condition)`. The condition can see the body's top-level locals.
    DoWhile { body: Block, condition: Expr },
    /// `for (variable in iterable) body`. The variable is a fresh `val` in each iteration.
    For {
        variable: String,
        variable_span: Span,
        iterable: Expr,
        body: Block,
    },
}

/// A statement node with its source span.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
}

impl Statement {
    pub fn new(kind: StatementKind, span: Span) -> Self {
        Self { kind, span }
    }
}

impl fmt::Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            StatementKind::Local {
                is_var,
                name,
                ty,
                initializer,
            } => {
                let keyword = if *is_var { "var" } else { "val" };
                match ty {
                    Some(t) => write!(f, "{} {}: {} = {}", keyword, name, t, initializer),
                    None => write!(f, "{} {} = {}", keyword, name, initializer),
                }
            }
            StatementKind::Assign { target, value } => write!(f, "{} = {}", target, value),
            StatementKind::Expression(expr) => write!(f, "{}", expr),
            StatementKind::While { condition, body } => {
                write!(f, "while ({}) {}", condition, body)
            }
            StatementKind::DoWhile { body, condition } => {
                write!(f, "do {} while ({})", body, condition)
            }
            StatementKind::For {
                variable,
                iterable,
                body,
                ..
            } => write!(f, "for ({} in {}) {}", variable, iterable, body),
        }
    }
}
