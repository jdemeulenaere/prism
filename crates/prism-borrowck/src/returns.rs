//! The callee's half of the elision rules (D5 to D10 of `borrowck-elision`): what a body may
//! return a borrow of.
//!
//! In a body whose return type contains a reference outside a function type, each write of the
//! whole return place (an assignment, or a call's destination) is checked against the items the
//! written value carries, by the loan flow before it, and the inputs its elision rule ties the
//! result to ([`elision::tied_inputs`]), the same rule its callers apply:
//!
//! - a loan of a place reached through a dereference of a reference from its root local is not
//!   judged itself, since the value also holds the root local's items; in a default value's
//!   generator, a loan of an earlier argument's own temporary (`(*_i)`, not through a further
//!   reference) borrows what the call passes by value, `E5009`;
//! - any other loan borrows memory the body owns, a local, a temporary or a by-value parameter,
//!   `DanglingBorrowReturn` (`E5008`), except, in a closure, a place it captures, which only a
//!   closure whose type has no input borrow may return (`E5009` otherwise);
//! - the input of an argument at a tied position is allowed, and in a closure whose type has no
//!   input borrow the input of a capture; any other input is `UntiedBorrowReturn` (`E5009`).
//!
//! Borrows of top-level values are constants that carry no loan, so they may be returned under
//! any rule. Each write reports at most one error: `E5008` for its earliest owned loan, else
//! `E5009` for its first untied input, by local order.

use crate::borrows::{BorrowSet, Item, LoanId, LoanKind};
use crate::check::sort_labels;
use crate::dataflow::Results;
use crate::elision::{self, Rule, Signature};
use crate::error::{BorrowError, BorrowErrorKind, Borrowed, Label, UntiedReason};
use crate::loan_flow::LoanFlow;
use crate::location::locations;
use crate::places;
use prism_ast::Span;
use prism_mir::{
    Body, BodyOwner, BorrowOrigin, LocalId, LocalKind, MirFile, Operand, Place, Projection, Rvalue,
    StatementKind, TerminatorKind,
};
use prism_typed_tree::{ArenaId, Type};

/// The returned borrows of `body` of `file` that its elision rule does not allow.
pub(crate) fn check_returns(
    file: &MirFile,
    body: &Body,
    borrows: &BorrowSet,
    flow: &Results<LoanFlow>,
) -> Vec<BorrowError> {
    let keeper = places::contains_keeper(body.return_type());
    if !places::is_input_borrow(body.return_type()) && !keeper {
        return Vec::new();
    }
    let parameters: Vec<Type> = body
        .arguments()
        .map(|local| body.locals[local].ty.clone())
        .collect();
    let (rule, mut tied) = elision::tied_inputs(Signature::of_body(body), &parameters);
    // A returned keeper is tied to the `attached` parameters too, whose values it keeps (D24
    // of `owned-state`).
    if keeper {
        for (position, local) in body.arguments().enumerate() {
            if body.locals[local].parameter_kind.is_attached() && !tied.contains(&position) {
                tied.push(position);
            }
        }
        tied.sort_unstable();
    }
    let checker = Returns {
        file,
        body,
        borrows,
        rule,
        tied,
    };
    let mut cursor = flow.cursor(body);
    let mut errors = Vec::new();
    for location in locations(body) {
        let block = &body.blocks[location.block];
        let (items, span) = match block.statements.get(location.statement) {
            Some(statement) => {
                let StatementKind::Assign(destination, rvalue) = &statement.kind else {
                    continue;
                };
                if *destination != Place::return_place() {
                    continue;
                }
                let state = cursor.seek(location);
                let items = flow.analysis.rvalue_items(state, rvalue, location);
                let span = match rvalue.as_ref() {
                    Rvalue::Use(operand)
                    | Rvalue::WrapNullable(operand)
                    | Rvalue::Subtype(operand) => operand_span(operand, statement.span),
                    _ => statement.span,
                };
                (items, span)
            }
            None => {
                let TerminatorKind::Call {
                    callee,
                    arguments,
                    parameters,
                    parameter_kinds,
                    destination,
                    ..
                } = &block.terminator.kind
                else {
                    continue;
                };
                if *destination != Place::return_place() {
                    continue;
                }
                let state = cursor.seek(location);
                let mut items = flow
                    .analysis
                    .call_items(state, callee, arguments, parameters);
                if keeper {
                    items.extend(
                        flow.analysis
                            .attached_items(state, arguments, parameter_kinds),
                    );
                    items.sort_unstable();
                    items.dedup();
                }
                (items, block.terminator.span)
            }
        };
        let items: Vec<Item> = flow
            .analysis
            .fitting(items, body.return_type())
            .into_iter()
            .map(|item| borrows.item(item))
            .collect();
        errors.extend(checker.check_write(&items, span));
    }
    errors
}

fn operand_span(operand: &Operand, otherwise: Span) -> Span {
    match operand {
        Operand::Copy(_, span) | Operand::Move(_, span) => *span,
        Operand::Constant(_) => otherwise,
    }
}

/// What a returned item is, when its rule does not allow it.
enum Verdict {
    /// A loan of memory the body owns (`E5008`).
    Dangling(LoanId),
    /// An input, or a loan rooted at one, the result is not tied to (`E5009`), with the local
    /// it derives from and the loan, if any, that borrows it.
    Untied {
        local: LocalId,
        reason: UntiedReason,
        loan: Option<LoanId>,
    },
}

struct Returns<'a> {
    file: &'a MirFile,
    body: &'a Body,
    borrows: &'a BorrowSet,
    rule: Rule,
    /// The positions of the arguments the result is tied to.
    tied: Vec<usize>,
}

impl Returns<'_> {
    /// The error of a write of the return place carrying `items` at `span`, if any.
    fn check_write(&self, items: &[Item], span: Span) -> Option<BorrowError> {
        let verdicts: Vec<Verdict> = items.iter().filter_map(|item| self.judge(*item)).collect();
        let dangling = verdicts
            .iter()
            .filter_map(|verdict| match verdict {
                Verdict::Dangling(loan) => Some(*loan),
                Verdict::Untied { .. } => None,
            })
            .min();
        if let Some(loan) = dangling {
            return Some(self.dangling(loan, span));
        }
        verdicts
            .into_iter()
            .filter_map(|verdict| match verdict {
                Verdict::Untied {
                    local,
                    reason,
                    loan,
                } => Some((local, reason, loan)),
                Verdict::Dangling(_) => None,
            })
            .min_by_key(|(local, _, _)| *local)
            .map(|(local, reason, loan)| self.untied(local, reason, loan, span))
    }

    fn is_closure(&self) -> bool {
        matches!(self.body.owner, BodyOwner::Closure { .. })
    }

    /// Whether `local` is a capture local that the closure owns.
    fn is_owned_capture(&self, local: LocalId) -> bool {
        let index = local.index().checked_sub(self.body.argument_count + 1);
        index.is_some_and(|index| {
            self.body.capture_modes.get(index) == Some(&prism_mir::CaptureMode::Owned)
        })
    }

    /// Whether `local` is an argument at a position the result is tied to.
    fn is_tied(&self, local: LocalId) -> bool {
        self.body.locals[local].kind == LocalKind::Argument
            && self.tied.contains(&(local.index() - 1))
    }

    /// What `item`, carried by a returned value, is, when its rule does not allow it.
    fn judge(&self, item: Item) -> Option<Verdict> {
        match item {
            Item::Loan(loan) => {
                let place = &self.borrows.loans[loan].place;
                let root = Place::local(place.local);
                let kind = self.body.locals[place.local].kind;
                if let Some(reason) = self.borrowed_default_argument(place) {
                    return Some(Verdict::Untied {
                        local: place.local,
                        reason,
                        loan: Some(loan),
                    });
                }
                if places::extends_through_reference(self.body, &self.file.types, place, &root) {
                    return None;
                }
                // A capture the closure owns is memory of its environment, which may be dropped
                // while the result is live (D10 of `escaping-closures`).
                if kind == LocalKind::Capture && self.is_owned_capture(place.local) {
                    return Some(Verdict::Dangling(loan));
                }
                if kind == LocalKind::Capture && self.is_closure() {
                    return (self.rule != Rule::FunctionValue).then_some(Verdict::Untied {
                        local: place.local,
                        reason: UntiedReason::Capture,
                        loan: Some(loan),
                    });
                }
                Some(Verdict::Dangling(loan))
            }
            Item::Input(local) => {
                let kind = self.body.locals[local].kind;
                if kind == LocalKind::Capture {
                    return (self.rule != Rule::FunctionValue).then_some(Verdict::Untied {
                        local,
                        reason: UntiedReason::Capture,
                        loan: None,
                    });
                }
                if self.is_tied(local) {
                    return None;
                }
                Some(Verdict::Untied {
                    local,
                    reason: self.untied_reason(local),
                    loan: None,
                })
            }
        }
    }

    /// In a default value's generator, the reason a loan of `place` borrows an earlier
    /// argument's own temporary, which the call takes, rather than what it refers to (D10):
    /// `place` is the dereference of an earlier argument `&P`, or a part of it, not through a
    /// further reference.
    fn borrowed_default_argument(&self, place: &Place) -> Option<UntiedReason> {
        let BodyOwner::Default { name, .. } = &self.body.owner else {
            return None;
        };
        let declaration = &self.body.locals[place.local];
        let is_earlier_argument = declaration.kind == LocalKind::Argument
            && matches!(declaration.ty, Type::Reference { mutable: false, .. });
        if !is_earlier_argument || place.projections.first() != Some(&Projection::Deref) {
            return None;
        }
        let referent = Place::local(place.local).project(Projection::Deref);
        if places::extends_through_reference(self.body, &self.file.types, place, &referent) {
            return None;
        }
        Some(UntiedReason::DefaultByValue {
            default: name.clone(),
        })
    }

    /// Why the argument `local` is not one the result is tied to.
    fn untied_reason(&self, local: LocalId) -> UntiedReason {
        let ty = &self.body.locals[local].ty;
        match &self.body.owner {
            BodyOwner::Default { name, .. } => {
                let referent = match ty {
                    Type::Reference { inner, .. } => inner.as_ref(),
                    other => other,
                };
                if places::contains_function(referent) {
                    UntiedReason::FunctionParameter
                } else if places::is_input_borrow(referent) {
                    UntiedReason::DefaultMutable {
                        default: name.clone(),
                    }
                } else {
                    UntiedReason::DefaultByValue {
                        default: name.clone(),
                    }
                }
            }
            BodyOwner::Function(path) if self.rule == Rule::Receiver => {
                if places::contains_function(ty) {
                    UntiedReason::FunctionParameter
                } else {
                    UntiedReason::NotReceiver {
                        method: path.name.clone(),
                    }
                }
            }
            _ => UntiedReason::FunctionParameter,
        }
    }

    /// The primary label of a returned value at `span`, by the body's owner.
    fn primary(&self, span: Span) -> Label {
        let message = match self.body.owner {
            BodyOwner::Function(_)
            | BodyOwner::Closure { .. }
            | BodyOwner::Restart { .. }
            | BodyOwner::RootContent { .. } => "returned here",
            BodyOwner::Value(_) => "assigned to a top-level value here",
            BodyOwner::Default { .. } => "returned as a default value here",
        };
        Label {
            span,
            message: message.to_string(),
        }
    }

    /// `E5008` for the returned `loan` of memory the body owns.
    fn dangling(&self, loan: LoanId, span: Span) -> BorrowError {
        let loan = &self.borrows.loans[loan];
        let declaration = &self.body.locals[loan.place.local];
        let name = places::place_name(self.body, &self.file.types, &loan.place);
        let borrowed = match (&declaration.kind, &declaration.name) {
            _ if loan.kind == LoanKind::Borrow(BorrowOrigin::StateGuard) => Borrowed::StateValue,
            (LocalKind::Argument, Some(_)) => Borrowed::Parameter(name.clone().unwrap_or_default()),
            (LocalKind::User, Some(_)) => Borrowed::Local(name.clone().unwrap_or_default()),
            (LocalKind::Capture, Some(capture)) => Borrowed::OwnedCapture(capture.clone()),
            _ => Borrowed::Temporary,
        };
        let mut related = Vec::new();
        if loan.span != span {
            related.push(Label {
                span: loan.span,
                message: match &borrowed {
                    Borrowed::Temporary => "a temporary value is borrowed here".to_string(),
                    Borrowed::StateValue => {
                        "the value of a state is borrowed here, holding the state's flag"
                            .to_string()
                    }
                    Borrowed::Local(name)
                    | Borrowed::Parameter(name)
                    | Borrowed::OwnedCapture(name) => {
                        format!("'{}' is borrowed here", name)
                    }
                },
            });
        }
        sort_labels(&mut related);
        let mut notes = Vec::new();
        if let (Rule::NoInput, BodyOwner::Function(path)) = (self.rule, &self.body.owner) {
            notes.push(format!(
                "'{}' takes no borrowed input, so it can only return borrows of top-level values (elision rule 4)",
                path.name
            ));
        }
        if borrowed == Borrowed::StateValue {
            notes.push(
                "outside composable contexts, a borrow of a state's value holds the state's flag until its statement or the block of its 'val' ends, and the flag ends when the body returns"
                    .to_string(),
            );
        }
        let help = match (&borrowed, &self.body.owner) {
            (Borrowed::StateValue, _) => {
                "return a copy or a clone of what the caller needs, or borrow the value where it is used"
                    .to_string()
            }
            // A `consuming fun` receiver: a `fun` one is a shared reference, tied by rule 2.
            (Borrowed::Parameter(_), _) if declaration.name.as_deref() == Some("this") => {
                "declare the method with 'fun' instead of 'consuming fun', so that the receiver is a shared reference".to_string()
            }
            // A reference parameter itself, borrowed: what it refers to may be returned.
            (Borrowed::Parameter(_), _) if places::is_input_borrow(&declaration.ty) => {
                "return the value itself instead of a borrow of it".to_string()
            }
            (Borrowed::Parameter(_), _) => {
                let parameter = declaration.name.clone().unwrap_or_default();
                format!(
                    "take '{}' as a reference parameter ('&{}') so that the result can borrow the caller's value",
                    parameter, declaration.ty
                )
            }
            (_, BodyOwner::Value(_)) => {
                "declare the value as a top-level 'val' of its own and borrow that".to_string()
            }
            _ => "return the value itself instead of a borrow of it".to_string(),
        };
        BorrowError {
            kind: BorrowErrorKind::DanglingBorrowReturn {
                borrowed,
                rule: self.rule,
            },
            primary: self.primary(span),
            related,
            notes,
            helps: vec![help],
        }
    }

    /// `E5009` for a returned borrow derived from `local`, for `reason`.
    fn untied(
        &self,
        local: LocalId,
        reason: UntiedReason,
        loan: Option<LoanId>,
        span: Span,
    ) -> BorrowError {
        let declaration = &self.body.locals[local];
        let parameter = declaration.name.clone();
        let quoted = places::quoted(&parameter);
        let name = parameter.clone().unwrap_or_default();
        let mut related = vec![Label {
            span: declaration.span,
            message: match reason {
                UntiedReason::Capture => format!("{} is captured here", quoted),
                _ if name == "this" => "'this' is the receiver here".to_string(),
                _ => format!("{} is a parameter here", quoted),
            },
        }];
        if let Some(loan) = loan {
            let loan = &self.borrows.loans[loan];
            if loan.span != span {
                let borrowed = places::place_name(self.body, &self.file.types, &loan.place);
                related.push(Label {
                    span: loan.span,
                    message: format!("{} is borrowed here", places::quoted(&borrowed)),
                });
            }
        }
        sort_labels(&mut related);
        let (note, help) = match &reason {
            UntiedReason::NotReceiver { .. } => (
                "a method's returned borrow is tied to its receiver only (elision rule 2), so callers keep only the receiver borrowed".to_string(),
                Some("return a borrow derived from the receiver, or declare a function taking the receiver as a reference parameter, whose result is tied to every reference parameter (elision rule 3)".to_string()),
            ),
            UntiedReason::FunctionParameter => (
                "a function-typed parameter is not a borrowed input, so callers do not keep borrowed what the function passed for it borrows".to_string(),
                None,
            ),
            UntiedReason::Capture => (
                "the closure's type takes a borrowed input, so its returned borrow is tied to its reference parameters only (elision rules 1 and 3) and callers do not keep what it captures borrowed".to_string(),
                Some(format!("pass {} to the closure as a reference parameter instead", quoted)),
            ),
            UntiedReason::DefaultByValue { .. } => {
                let help = if name == "this" {
                    "declare the method with 'fun' instead of 'consuming fun', so that the receiver is a shared reference".to_string()
                } else {
                    let ty = match &declaration.ty {
                        Type::Reference { inner, .. } => inner.to_string(),
                        other => other.to_string(),
                    };
                    format!("declare {} as a reference parameter ('&{}')", quoted, ty)
                };
                (
                    format!(
                        "the call passes {} itself, so a default can only borrow what a shared reference parameter refers to",
                        quoted
                    ),
                    Some(help),
                )
            }
            UntiedReason::DefaultMutable { default } => (
                format!(
                    "the call passes {} as a mutable borrow beside the default, which would alias it",
                    quoted
                ),
                Some(format!(
                    "pass '{}' explicitly at each call, or give it a default that does not borrow {}",
                    default, quoted
                )),
            ),
        };
        BorrowError {
            kind: BorrowErrorKind::UntiedBorrowReturn { parameter, reason },
            primary: self.primary(span),
            related,
            notes: vec![note],
            helps: help.into_iter().collect(),
        }
    }
}
