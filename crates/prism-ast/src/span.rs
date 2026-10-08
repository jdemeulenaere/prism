//! Source span representation for AST nodes and tokens.

use std::fmt;

/// Represents a contiguous span in a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    /// Zero-based byte offset where the span starts.
    pub start: usize,
    /// Zero-based byte offset where the span ends (exclusive).
    pub end: usize,
    /// 1-based line number of the start.
    pub line: u32,
    /// 1-based column number of the start.
    pub column: u32,
}

impl Span {
    /// Creates a new span.
    pub fn new(start: usize, end: usize, line: u32, column: u32) -> Self {
        Self {
            start,
            end,
            line,
            column,
        }
    }

    /// Creates a zero/dummy span for synthetic or test nodes.
    pub fn dummy() -> Self {
        Self {
            start: 0,
            end: 0,
            line: 1,
            column: 1,
        }
    }

    /// Merges two spans into a single span spanning from the start of `self` to the end of `other`.
    pub fn merge(self, other: Span) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            line: self.line.min(other.line),
            column: if self.start <= other.start {
                self.column
            } else {
                other.column
            },
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}-{}:{}",
            self.line, self.column, self.start, self.end
        )
    }
}

/// A wrapper attaching a [`Span`] to any syntax element.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }
}
