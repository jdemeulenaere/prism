//! The borrow checker's diagnostics (D9 of `borrowck-moves`, D13 of `borrowck-exclusivity`): a
//! primary location and related locations, each with a label, then notes and help lines.

use crate::elision::Rule;
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
    /// `E5007`: a function value that may hold a borrow escapes; `holder` says what holds it
    /// (D6 of `escaping-closures`).
    EscapingClosure { holder: EscapingHolder },
    /// `E5008`: a body returns a borrow of memory it owns, under the elision rule `rule` of its
    /// signature (D5 and D6 of `borrowck-elision`).
    DanglingBorrowReturn { borrowed: Borrowed, rule: Rule },
    /// `E5009`: a body returns a borrow derived from the input `parameter` (an argument, or a
    /// closure's capture), which its elision rule does not tie the result to, for `reason`.
    UntiedBorrowReturn {
        parameter: Option<String>,
        reason: UntiedReason,
    },
    /// `E5010`: a value attached to the composition escapes elsewhere than to an `attached`
    /// parameter (D9 and D24 of `owned-state`); `holder` says what holds it.
    AttachedFunctionEscapes { holder: AttachedHolder },
}

/// What attaches an escaping value to the composition (D9 and D24 of `owned-state`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachedHolder {
    /// The value of an `attached` parameter, named `name`.
    Parameter(Option<String>),
    /// A closure capturing the state `name` weakly.
    Closure(Option<String>),
    /// A composition whose content captures the state `name` weakly.
    Composition(Option<String>),
}

/// What makes an escaping function value hold a borrow (D6 of `escaping-closures`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscapingHolder {
    /// The value is the unmarked function-typed parameter itself.
    Parameter(Option<String>),
    /// A closure that owns the unmarked function-typed parameter.
    ClosureOwningParameter(Option<String>),
    /// A closure that owns a reference, or a value holding a borrow, named `name`.
    ClosureOwningReference(Option<String>),
    /// A closure that borrows `capture`.
    ClosureBorrowing(Option<String>),
    /// A closure that owns `holder`, a function value that borrows `capture`.
    ClosureOwningBorrower {
        holder: Option<String>,
        capture: Option<String>,
    },
    /// A function value holding a borrow of `borrowed`.
    Borrow(Option<String>),
}

/// What a dangling returned borrow borrows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Borrowed {
    /// A local `val` or `var`.
    Local(String),
    /// A by-value parameter, which ends when the body returns.
    Parameter(String),
    /// A capture the closure owns, which its environment holds (D10 of `escaping-closures`).
    OwnedCapture(String),
    /// A temporary value.
    Temporary,
}

/// Why a returned borrow's input is not one its elision rule ties the result to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UntiedReason {
    /// A parameter of the method `method` other than its borrowed receiver (rule 2).
    NotReceiver { method: String },
    /// A function-typed parameter, which is not an input borrow (D9).
    FunctionParameter,
    /// A capture of a closure whose type has an input borrow, whose result is tied to its
    /// reference parameters only (D8).
    Capture,
    /// An earlier by-value argument of the default value of parameter `default`, which the
    /// call takes (D10).
    DefaultByValue { default: String },
    /// An earlier argument of the default value of parameter `default` that the call borrows
    /// mutably beside it (D10).
    DefaultMutable { default: String },
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
            BorrowErrorKind::DanglingBorrowReturn { .. } => "E5008",
            BorrowErrorKind::UntiedBorrowReturn { .. } => "E5009",
            BorrowErrorKind::AttachedFunctionEscapes { .. } => "E5010",
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
            BorrowErrorKind::EscapingClosure { holder } => {
                return match holder {
                    EscapingHolder::Parameter(name) => {
                        format!("the parameter {} cannot escape", quoted(name))
                    }
                    EscapingHolder::ClosureOwningParameter(name) => format!(
                        "a closure that captures the parameter {} cannot escape",
                        quoted(name)
                    ),
                    EscapingHolder::ClosureOwningReference(name) => format!(
                        "a closure that captures the reference {} cannot escape",
                        quoted(name)
                    ),
                    EscapingHolder::ClosureBorrowing(name) => {
                        format!("a closure that captures {} cannot escape", quoted(name))
                    }
                    EscapingHolder::ClosureOwningBorrower { holder, capture } => format!(
                        "a closure that captures {}, which borrows {}, cannot escape",
                        quoted(holder),
                        quoted(capture)
                    ),
                    EscapingHolder::Borrow(name) => format!(
                        "a function value that holds a borrow of {} cannot escape",
                        quoted(name)
                    ),
                }
            }
            BorrowErrorKind::DanglingBorrowReturn { borrowed, .. } => {
                return match borrowed {
                    Borrowed::Local(name) => format!("cannot return a borrow of local '{}'", name),
                    Borrowed::Parameter(name) => {
                        format!("cannot return a borrow of parameter '{}'", name)
                    }
                    Borrowed::OwnedCapture(name) => format!(
                        "cannot return a borrow of captured '{}', which the closure owns",
                        name
                    ),
                    Borrowed::Temporary => {
                        "cannot return a borrow of a temporary value".to_string()
                    }
                }
            }
            BorrowErrorKind::AttachedFunctionEscapes { holder } => {
                return match holder {
                    AttachedHolder::Parameter(name) => format!(
                        "the attached parameter {} cannot escape: an attached function value is kept only by the composition",
                        quoted(name)
                    ),
                    AttachedHolder::Closure(name) => format!(
                        "a closure that captures {} weakly cannot escape: an attached function value is kept only by the composition",
                        quoted(name)
                    ),
                    AttachedHolder::Composition(name) => format!(
                        "a composition whose content captures {} weakly cannot be stored here",
                        quoted(name)
                    ),
                }
            }
            BorrowErrorKind::UntiedBorrowReturn { parameter, reason } => {
                let parameter = quoted(parameter);
                return match reason {
                    UntiedReason::NotReceiver { method } => format!(
                        "method '{}' cannot return a borrow derived from parameter {}",
                        method, parameter
                    ),
                    UntiedReason::FunctionParameter => format!(
                        "cannot return a borrow obtained from function-typed parameter {}",
                        parameter
                    ),
                    UntiedReason::Capture => format!(
                        "the closure cannot return a borrow derived from captured {}",
                        parameter
                    ),
                    UntiedReason::DefaultByValue { default } => format!(
                        "default value of '{}' cannot borrow parameter {}",
                        default, parameter
                    ),
                    UntiedReason::DefaultMutable { default } => format!(
                        "default value of '{}' cannot borrow from {}, which the call borrows mutably",
                        default, parameter
                    ),
                };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn error(kind: BorrowErrorKind) -> BorrowError {
        BorrowError {
            kind,
            primary: Label {
                span: Span::dummy(),
                message: String::new(),
            },
            related: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }

    #[test]
    fn returned_borrow_codes_and_titles() {
        let dangling = |borrowed| {
            error(BorrowErrorKind::DanglingBorrowReturn {
                borrowed,
                rule: Rule::NoInput,
            })
        };
        let cases = [
            (
                dangling(Borrowed::Local("n".to_string())),
                "E5008",
                "cannot return a borrow of local 'n'",
            ),
            (
                dangling(Borrowed::Parameter("s".to_string())),
                "E5008",
                "cannot return a borrow of parameter 's'",
            ),
            (
                dangling(Borrowed::Temporary),
                "E5008",
                "cannot return a borrow of a temporary value",
            ),
        ];
        for (error, code, message) in cases {
            assert_eq!((error.code(), error.message().as_str()), (code, message));
        }
        let untied = |parameter: &str, reason| {
            error(BorrowErrorKind::UntiedBorrowReturn {
                parameter: Some(parameter.to_string()),
                reason,
            })
        };
        let cases = [
            (
                untied(
                    "o",
                    UntiedReason::NotReceiver {
                        method: "other".to_string(),
                    },
                ),
                "method 'other' cannot return a borrow derived from parameter 'o'",
            ),
            (
                untied("h", UntiedReason::FunctionParameter),
                "cannot return a borrow obtained from function-typed parameter 'h'",
            ),
            (
                untied("n", UntiedReason::Capture),
                "the closure cannot return a borrow derived from captured 'n'",
            ),
            (
                untied(
                    "a",
                    UntiedReason::DefaultByValue {
                        default: "b".to_string(),
                    },
                ),
                "default value of 'b' cannot borrow parameter 'a'",
            ),
            (
                untied(
                    "this",
                    UntiedReason::DefaultMutable {
                        default: "n".to_string(),
                    },
                ),
                "default value of 'n' cannot borrow from 'this', which the call borrows mutably",
            ),
        ];
        for (error, message) in cases {
            assert_eq!((error.code(), error.message().as_str()), ("E5009", message));
        }
    }
}
