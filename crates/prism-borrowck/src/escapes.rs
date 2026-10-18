//! The escape check (D12 and D13 of `borrowck-exclusivity`): a function value that may be a
//! closure with captures, or a function-typed parameter, must not escape.
//!
//! A value whose type contains a function type escapes when it is written to the return place
//! (of a function, a closure, a default value's generator, or a top-level `val`'s
//! initializer), used as an operand of an aggregate (a struct field or an enum payload),
//! written to a place with a projection (a field, or through a reference) or written to a
//! capture local. It is rejected as `EscapingClosure` (`E5007`) when, by the loan flow, it may
//! hold a closure capture's loan or a function input. Passing a closure to a call is not an
//! escape, and a closure without captures holds neither.

use crate::borrows::{BorrowSet, Item, LoanId, LoanKind};
use crate::dataflow::Results;
use crate::error::{BorrowError, BorrowErrorKind, Label};
use crate::loan_flow::LoanFlow;
use crate::location::locations;
use crate::places;
use prism_ast::Span;
use prism_mir::{
    AggregateKind, Body, BodyOwner, LocalId, LocalKind, MirFile, Operand, Place, Projection,
    Rvalue, StatementKind,
};
use std::collections::HashSet;

/// The escapes of `body` of `file`, with the capture loans of the escaping values, whose end
/// the conflict check need not report again.
pub(crate) fn check_escapes(
    file: &MirFile,
    body: &Body,
    borrows: &BorrowSet,
    flow: &Results<LoanFlow>,
) -> (Vec<BorrowError>, HashSet<LoanId>) {
    let mut escaped = HashSet::new();
    let mut cursor = flow.cursor(body);
    let mut errors = Vec::new();
    for location in locations(body) {
        let Some(statement) = body.blocks[location.block]
            .statements
            .get(location.statement)
        else {
            continue;
        };
        let StatementKind::Assign(destination, rvalue) = &statement.kind else {
            continue;
        };
        let state = cursor.seek(location).clone();
        let checker = Escapes {
            file,
            body,
            borrows,
        };
        if let Rvalue::Aggregate(kind, operands) = rvalue.as_ref() {
            let position = match kind {
                AggregateKind::Struct(_) => "stored in a field here",
                AggregateKind::Variant { .. } => "stored in a variant here",
            };
            for operand in operands {
                let Some(place) = operand.place() else {
                    continue;
                };
                if !places::contains_function(&place.ty(body, &file.types)) {
                    continue;
                }
                let items = flow.analysis.holds(&state, place.local);
                escaped.extend(checker.capture_loans(&items));
                errors.extend(checker.error(
                    &items,
                    position,
                    operand_span(operand, statement.span),
                ));
            }
        }
        let ty = destination.ty(body, &file.types);
        if !places::contains_function(&ty) {
            continue;
        }
        let Some(position) = checker.position(destination) else {
            continue;
        };
        let items: Vec<Item> = flow
            .analysis
            .rvalue_items(&state, rvalue, location)
            .into_iter()
            .map(|item| borrows.item(item))
            .collect();
        let span = match rvalue.as_ref() {
            Rvalue::Use(operand) | Rvalue::WrapNullable(operand) | Rvalue::Subtype(operand) => {
                operand_span(operand, statement.span)
            }
            _ => statement.span,
        };
        escaped.extend(checker.capture_loans(&items));
        errors.extend(checker.error(&items, position, span));
    }
    (errors, escaped)
}

fn operand_span(operand: &Operand, otherwise: Span) -> Span {
    match operand {
        Operand::Copy(_, span) | Operand::Move(_, span) => *span,
        Operand::Constant(_) => otherwise,
    }
}

struct Escapes<'a> {
    file: &'a MirFile,
    body: &'a Body,
    borrows: &'a BorrowSet,
}

impl Escapes<'_> {
    /// The closure captures' loans among `items`.
    fn capture_loans(&self, items: &[Item]) -> Vec<LoanId> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Loan(loan)
                    if matches!(self.borrows.loans[*loan].kind, LoanKind::Capture { .. }) =>
                {
                    Some(*loan)
                }
                _ => None,
            })
            .collect()
    }

    /// What writing a function value to `destination` does, when it is an escape.
    fn position(&self, destination: &Place) -> Option<&'static str> {
        if *destination == Place::return_place() {
            return Some(match self.body.owner {
                BodyOwner::Function(_) | BodyOwner::Closure { .. } => "returned here",
                BodyOwner::Value(_) => "assigned to a top-level value here",
                BodyOwner::Default { .. } => "returned as a default value here",
            });
        }
        if destination.projections.contains(&Projection::Deref) {
            return Some("written through a reference here");
        }
        if !destination.projections.is_empty() {
            return Some("stored in a field here");
        }
        (self.body.locals[destination.local].kind == LocalKind::Capture)
            .then_some("assigned to a captured variable here")
    }

    /// The error for a value holding `items` that escapes at `span`, when one of them is a
    /// closure capture's loan or a function input.
    fn error(&self, items: &[Item], position: &str, span: Span) -> Option<BorrowError> {
        let escaping = items.iter().copied().find(|item| match item {
            Item::Loan(loan) => matches!(self.borrows.loans[*loan].kind, LoanKind::Capture { .. }),
            Item::Input(_) => true,
        })?;
        let primary = Label {
            span,
            message: position.to_string(),
        };
        Some(match escaping {
            Item::Loan(loan) => {
                let LoanKind::Capture { closure, index } = self.borrows.loans[loan].kind else {
                    unreachable!("a capture's loan");
                };
                let closure_body = &self.file.bodies[closure];
                let capture =
                    &closure_body.locals[LocalId((closure_body.argument_count + 1 + index) as u32)];
                let name = capture.name.clone();
                let quoted = places::quoted(&name);
                BorrowError {
                    kind: BorrowErrorKind::EscapingClosure {
                        capture: name,
                        parameter: None,
                    },
                    primary,
                    related: vec![Label {
                        span: capture.span,
                        message: format!("{} is captured here", quoted),
                    }],
                    notes: vec![
                        "a closure borrows what it captures, so it can only be called or passed to functions while that is in scope; escaping closures that capture by move are not designed yet"
                            .to_string(),
                    ],
                    helps: vec![format!(
                        "pass {} to the closure as a parameter instead of capturing it",
                        quoted
                    )],
                }
            }
            Item::Input(local) => {
                let declaration = &self.body.locals[local];
                let name = declaration.name.clone();
                BorrowError {
                    kind: BorrowErrorKind::EscapingClosure {
                        capture: None,
                        parameter: name.clone(),
                    },
                    primary,
                    related: vec![Label {
                        span: declaration.span,
                        message: format!("{} is a parameter here", places::quoted(&name)),
                    }],
                    notes: vec![
                        "a function-typed parameter may be a closure that borrows the caller's variables"
                            .to_string(),
                    ],
                    helps: Vec::new(),
                }
            }
        })
    }
}
