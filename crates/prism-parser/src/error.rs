//! Parser error definitions.

use prism_ast::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Diagnostic code from the catalog in `openspec/specs/diagnostics`.
    pub code: &'static str,
    pub message: String,
    pub span: Span,
}

impl ParseError {
    /// A syntax error (`E2001`).
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            code: "E2001",
            message: message.into(),
            span,
        }
    }

    /// Expressions or types nested deeper than the parser accepts (`E2002`).
    pub fn nesting_too_deep(limit: usize, span: Span) -> Self {
        Self {
            code: "E2002",
            message: format!(
                "Expression or type nesting exceeds the limit of {} levels",
                limit
            ),
            span,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Parse error at {}: {}", self.span, self.message)
    }
}

impl std::error::Error for ParseError {}
