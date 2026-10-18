//! The borrow checker's diagnostics (D9 of `borrowck-moves`, D13 of `borrowck-exclusivity`): a
//! primary location and related locations, each with a label, then notes and help lines.

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
    /// `E5003`: a place is borrowed or read while a conflicting borrow of an overlapping place
    /// is live. `place` names the accessed place and `borrowed` the borrowed one (`None` for a
    /// temporary); `loan_mutable` tells whether the live borrow is mutable.
    BorrowConflict {
        place: Option<String>,
        borrowed: Option<String>,
        access: AccessKind,
        loan_mutable: bool,
    },
    /// `E5004`: a place is assigned while a borrow of an overlapping place is live.
    AssignToBorrowed {
        place: Option<String>,
        borrowed: Option<String>,
    },
    /// `E5005`: a place is moved while a borrow of an overlapping place is live.
    MoveOutWhileBorrowed {
        place: Option<String>,
        borrowed: Option<String>,
    },
    /// `E5006`: a local (`name`) or a temporary (`None`) is dropped or ends while a borrow of it
    /// is live.
    DroppedWhileBorrowed { name: Option<String> },
    /// `E5007`: a function value that may be a closure capturing `capture`, or the
    /// function-typed parameter `parameter`, escapes.
    EscapingClosure {
        capture: Option<String>,
        parameter: Option<String>,
    },
}

/// How a statement or terminator accesses a place, which decides the borrows it conflicts
/// with (D6 of `borrowck-exclusivity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    /// A copy, a read, a discriminant, a switch operand or a function-value callee.
    Read,
    /// A shared borrow, or a shared closure capture.
    SharedBorrow,
    /// A mutable borrow, or a mutable closure capture.
    MutableBorrow,
    /// A move of a value that is not `Copy`.
    Move,
    /// An assignment or a call's destination.
    Write,
    /// A drop at the end of a scope.
    Drop,
    /// The end of a local's storage.
    StorageDead,
}

impl BorrowError {
    /// The error's code, in the borrow checker's `E5xxx` range.
    pub fn code(&self) -> &'static str {
        match self.kind {
            BorrowErrorKind::UseAfterMove { .. } => "E5001",
            BorrowErrorKind::UseOfUninitialized { .. } => "E5002",
            BorrowErrorKind::BorrowConflict { .. } => "E5003",
            BorrowErrorKind::AssignToBorrowed { .. } => "E5004",
            BorrowErrorKind::MoveOutWhileBorrowed { .. } => "E5005",
            BorrowErrorKind::DroppedWhileBorrowed { .. } => "E5006",
            BorrowErrorKind::EscapingClosure { .. } => "E5007",
        }
    }

    /// The error's title: "use of moved value 's'", "cannot assign to 'x' because it is
    /// borrowed".
    pub fn message(&self) -> String {
        let (what, name, possibly) = match &self.kind {
            BorrowErrorKind::UseAfterMove { name, possibly } => ("moved", name, *possibly),
            BorrowErrorKind::UseOfUninitialized { name, possibly } => {
                ("uninitialized", name, *possibly)
            }
            BorrowErrorKind::BorrowConflict {
                place,
                borrowed,
                access,
                loan_mutable,
            } => return conflict_message(place, borrowed, *access, *loan_mutable),
            BorrowErrorKind::AssignToBorrowed { place, .. } => {
                return format!("cannot assign to {} because it is borrowed", quoted(place))
            }
            BorrowErrorKind::MoveOutWhileBorrowed { place, .. } => {
                return format!(
                    "cannot move out of {} because it is borrowed",
                    quoted(place)
                )
            }
            BorrowErrorKind::DroppedWhileBorrowed { name: Some(name) } => {
                return format!("'{}' does not live long enough", name)
            }
            BorrowErrorKind::DroppedWhileBorrowed { name: None } => {
                return "temporary value dropped while borrowed".to_string()
            }
            BorrowErrorKind::EscapingClosure {
                parameter: Some(parameter),
                ..
            } => return format!("function-typed parameter '{}' cannot escape", parameter),
            BorrowErrorKind::EscapingClosure {
                capture: Some(capture),
                ..
            } => return format!("a closure that captures '{}' cannot escape", capture),
            BorrowErrorKind::EscapingClosure { .. } => {
                return "a closure that captures variables cannot escape".to_string()
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

/// `'name'`, or "a temporary value".
fn quoted(name: &Option<String>) -> String {
    crate::places::quoted(name)
}

/// The title of a `BorrowConflict`: what the access does, and how the place is borrowed.
fn conflict_message(
    place: &Option<String>,
    borrowed: &Option<String>,
    access: AccessKind,
    loan_mutable: bool,
) -> String {
    let mode = if loan_mutable { "mutable" } else { "shared" };
    let because = if place == borrowed {
        match access {
            AccessKind::MutableBorrow if loan_mutable => String::new(),
            AccessKind::Read => format!(" because it is borrowed as {}", mode),
            _ => format!(" because it is also borrowed as {}", mode),
        }
    } else {
        format!(" because {} is borrowed as {}", quoted(borrowed), mode)
    };
    match access {
        AccessKind::MutableBorrow if loan_mutable && place == borrowed => format!(
            "cannot borrow {} as mutable more than once at a time",
            quoted(place)
        ),
        AccessKind::MutableBorrow => {
            format!("cannot borrow {} as mutable{}", quoted(place), because)
        }
        AccessKind::SharedBorrow => format!("cannot borrow {} as shared{}", quoted(place), because),
        _ => format!("cannot use {}{}", quoted(place), because),
    }
}
