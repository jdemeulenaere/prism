//! Expression syntax trees for Prism.

use crate::span::Span;
use crate::statement::Block;
use std::fmt;

/// Literal values in Prism source code.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    String(String),
    Char(char),
    Bool(bool),
    Null,
}

impl Eq for Literal {}

impl std::hash::Hash for Literal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Literal::Int(v) => {
                0u8.hash(state);
                v.hash(state);
            }
            Literal::Float(v) => {
                1u8.hash(state);
                v.to_bits().hash(state);
            }
            Literal::String(v) => {
                2u8.hash(state);
                v.hash(state);
            }
            Literal::Char(v) => {
                3u8.hash(state);
                v.hash(state);
            }
            Literal::Bool(v) => {
                4u8.hash(state);
                v.hash(state);
            }
            Literal::Null => {
                5u8.hash(state);
            }
        }
    }
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    // Comparison
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    // Logical
    And,
    Or,
    // Prism specifics
    Elvis,         // ?:
    Range,         // ..
    HalfOpenRange, // ..<
}

impl BinaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Eq => "==",
            BinaryOp::NotEq => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::LtEq => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::GtEq => ">=",
            BinaryOp::And => "&&",
            BinaryOp::Or => "||",
            BinaryOp::Elvis => "?:",
            BinaryOp::Range => "..",
            BinaryOp::HalfOpenRange => "..<",
        }
    }
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    Neg,       // -
    Not,       // !
    Borrow,    // &
    BorrowMut, // &mut
}

impl UnaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            UnaryOp::Neg => "-",
            UnaryOp::Not => "!",
            UnaryOp::Borrow => "&",
            UnaryOp::BorrowMut => "&mut ",
        }
    }
}

/// A call argument, optionally named (e.g. `padding = 16`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallArg {
    pub name: Option<String>,
    pub value: Expr,
    pub span: Span,
}

/// Pattern in a `when` branch.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WhenPattern {
    /// Single expression or constant match, e.g. `0` or `1, 2, 3`.
    Expr(Expr),
    /// Range pattern: `in start..end`, or `in start..<end`, which excludes `end`.
    InRange {
        start: Expr,
        end: Expr,
        end_inclusive: bool,
    },
    /// Fallback `else` branch.
    Else,
}

/// A single branch in a `when` expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhenBranch {
    pub patterns: Vec<WhenPattern>,
    pub body: Expr,
    pub span: Span,
}

/// The specific kind of an expression.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExprKind {
    Literal(Literal),
    Ident(String),
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<CallArg>,
    },
    MemberAccess {
        target: Box<Expr>,
        member: String,
        is_safe_nav: bool, // true for `?.`, false for `.`
    },
    /// A block in block position: a function body, a branch body, or a statement-position
    /// scope. Its value is its tail expression.
    Block(Block),
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Option<Box<Expr>>,
    },
    When {
        target: Option<Box<Expr>>,
        branches: Vec<WhenBranch>,
    },
    Return(Option<Box<Expr>>),
    /// `break`: leaves the innermost enclosing loop. It has type `Nothing`.
    Break,
    /// `continue`: starts the next iteration of the innermost enclosing loop. It has type
    /// `Nothing`.
    Continue,
    /// A closure `{ a, b -> ... }`, or `{ ... }` with no parameters. The body is
    /// always a block, whatever the number of statements.
    Closure {
        params: Vec<String>,
        body: Block,
    },
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
    },
}

/// A complete expression node with its source span.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ExprKind::Literal(Literal::Int(v)) => write!(f, "{}", v),
            ExprKind::Literal(Literal::Float(v)) => write!(f, "{}", v),
            ExprKind::Literal(Literal::String(v)) => write!(f, "\"{}\"", v),
            ExprKind::Literal(Literal::Char(v)) => write!(f, "'{}'", v),
            ExprKind::Literal(Literal::Bool(v)) => write!(f, "{}", v),
            ExprKind::Literal(Literal::Null) => write!(f, "null"),
            ExprKind::Ident(id) => write!(f, "{}", id),
            ExprKind::Binary { op, left, right } => {
                write!(f, "({} {} {})", left, op.as_str(), right)
            }
            ExprKind::Unary { op, expr } => write!(f, "({}{})", op.as_str(), expr),
            ExprKind::Call { callee, args } => {
                write!(f, "{}(", callee)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    if let Some(name) = &arg.name {
                        write!(f, "{} = ", name)?;
                    }
                    write!(f, "{}", arg.value)?;
                }
                write!(f, ")")
            }
            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav,
            } => {
                let sep = if *is_safe_nav { "?." } else { "." };
                write!(f, "{}{}{}", target, sep, member)
            }
            ExprKind::Block(block) => write!(f, "{}", block),
            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                write!(f, "if ({}) {}", cond, then_branch)?;
                if let Some(els) = else_branch {
                    write!(f, " else {}", els)?;
                }
                Ok(())
            }
            ExprKind::When { target, branches } => {
                write!(f, "when ")?;
                if let Some(t) = target {
                    write!(f, "({}) ", t)?;
                }
                write!(f, "{{ ... {} branches }}", branches.len())
            }
            ExprKind::Return(Some(val)) => write!(f, "return {}", val),
            ExprKind::Return(None) => write!(f, "return"),
            ExprKind::Break => write!(f, "break"),
            ExprKind::Continue => write!(f, "continue"),
            ExprKind::Closure { params, body } => {
                write!(f, "{{ ")?;
                if !params.is_empty() {
                    write!(f, "{} -> ", params.join(", "))?;
                }
                write!(f, "{} }}", body.summary())
            }
            ExprKind::Index { target, index } => {
                write!(f, "{}[{}]", target, index)
            }
        }
    }
}
