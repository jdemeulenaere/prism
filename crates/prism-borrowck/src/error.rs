//! The borrow checker's diagnostics (D9 of `borrowck-moves`): a primary location and related
//! locations, each with a label, then notes and help lines.

use prism_ast::Span;

/// One error of the borrow checker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowError {
    pub kind: BorrowErrorKind,
    /// Where the error is: the use of the value.
    pub primary: Label,
    /// Locations that explain the error (where the value moved), in source order.
    pub related: Vec<Label>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
}

/// A location of a diagnostic and what happens there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

/// What kind of error a [`BorrowError`] is, which decides its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BorrowErrorKind {
    /// `E5001`: a value is used on a path on which it was moved; `possibly` when it is moved on
    /// some paths only.
    UseAfterMove { name: String, possibly: bool },
    /// `E5002`: a local is read on a path on which it was never assigned; `possibly` when it is
    /// assigned on some paths only.
    UseOfUninitialized { name: String, possibly: bool },
}

impl BorrowError {
    /// The error's code, in the borrow checker's `E5xxx` range.
    pub fn code(&self) -> &'static str {
        match self.kind {
            BorrowErrorKind::UseAfterMove { .. } => "E5001",
            BorrowErrorKind::UseOfUninitialized { .. } => "E5002",
        }
    }

    /// The error's title: "use of moved value 's'".
    pub fn message(&self) -> String {
        let (what, name, possibly) = match &self.kind {
            BorrowErrorKind::UseAfterMove { name, possibly } => ("moved", name, *possibly),
            BorrowErrorKind::UseOfUninitialized { name, possibly } => {
                ("uninitialized", name, *possibly)
            }
        };
        format!(
            "use of {}{} value '{}'",
            if possibly { "possibly " } else { "" },
            what,
            name
        )
    }
}
