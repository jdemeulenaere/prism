//! The escape check (D12 and D13 of `borrowck-exclusivity`, D6 of `escaping-closures`): a value
//! of function type that may hold a borrow must not escape.
//!
//! A value whose type contains a function type escapes when it is written to the return place
//! (of a function, a closure, a default value's generator, or a top-level `val`'s
//! initializer), used as an operand of an aggregate (a struct field or an enum payload),
//! written to a place with a projection (a field, or through a reference) or written to a
//! capture local, passed for an `escaping` parameter, or passed for a parameter whose declared
//! type has a type parameter outside a function type, since the generic body may store, return
//! or keep it (D7 of `generics`), which includes boxing it with `Heap(...)` (D11 of
//! `heap-boxes`). It is rejected as `EscapingClosure` (`E5007`) when, by the loan flow, it may
//! hold a loan, or the input of an argument or capture that carries loans: an `escaping`
//! argument has none (D4 of `escaping-closures`). A closure that owns its captures holds what
//! its captures' operands held, so one owning only values that hold no borrow passes, as does a
//! closure without captures. The type checker makes every closure at such a position own its
//! captures (D1), so this check is the safety net for borrows they still hold.

use crate::borrows::{BorrowSet, Item, LoanId, LoanKind};
use crate::dataflow::Results;
use crate::error::{BorrowError, BorrowErrorKind, EscapingHolder, Label};
use crate::loan_flow::{Holdings, LoanFlow};
use crate::location::locations;
use crate::places;
use prism_ast::Span;
use prism_mir::{
    AggregateKind, Body, BodyOwner, Callee, Capture, LocalId, LocalKind, MirFile, Operand, Place,
    Projection, Rvalue, StatementKind, TerminatorKind,
};
use prism_typed_tree::{BuiltinFunction, Type};
use std::collections::HashSet;

/// The escapes of `body` of `file`, with the loans of the escaping values, whose end the
/// conflict check need not report again.
pub(crate) fn check_escapes(
    file: &MirFile,
    body: &Body,
    borrows: &BorrowSet,
    flow: &Results<LoanFlow>,
) -> (Vec<BorrowError>, HashSet<LoanId>) {
    let mut escaped = HashSet::new();
    let mut cursor = flow.cursor(body);
    let mut errors = Vec::new();
    let checker = Escapes {
        file,
        body,
        borrows,
        flow: &flow.analysis,
    };
    for location in locations(body) {
        let Some(statement) = body.blocks[location.block]
            .statements
            .get(location.statement)
        else {
            let terminator = &body.blocks[location.block].terminator;
            let TerminatorKind::Call {
                callee,
                arguments,
                parameters,
                escaping,
                ..
            } = &terminator.kind
            else {
                continue;
            };
            let state = cursor.seek(location);
            for (index, (operand, parameter)) in arguments.iter().zip(parameters).enumerate() {
                // A box is an owned container that can be returned or stored anywhere, like a
                // struct (D11 of `heap-boxes`); an `escaping` parameter may be kept by the
                // callee; a generic slot's body sees a value of a type parameter.
                let position = if *callee == Callee::Builtin(BuiltinFunction::HeapAllocate) {
                    "stored in a box here"
                } else if escaping.get(index).copied().unwrap_or(false) {
                    "passed to an escaping parameter here"
                } else if matches!(callee, Callee::State(_))
                    && parameter.mentions_parameter_outside_functions()
                {
                    // A state keeps the value it is given (D18 of `state-handles`).
                    "stored in a state here"
                } else if parameter.mentions_parameter_outside_functions() {
                    "passed to a generic parameter here"
                } else {
                    continue;
                };
                let Some(place) = operand.place() else {
                    continue;
                };
                if !places::contains_function(&place.ty(body, &file.types)) {
                    continue;
                }
                let items = flow.analysis.holds(state, place.local);
                escaped.extend(checker.loans(&items));
                let value = Value::Local(place.local);
                errors.extend(checker.error(
                    state,
                    &items,
                    &value,
                    position,
                    operand_span(operand, terminator.span),
                ));
            }
            continue;
        };
        let StatementKind::Assign(destination, rvalue) = &statement.kind else {
            continue;
        };
        let state = cursor.seek(location);
        if let Rvalue::Aggregate(kind, operands) = rvalue.as_ref() {
            let position = match kind {
                AggregateKind::Struct { .. } => "stored in a field here",
                AggregateKind::Variant { .. } => "stored in a variant here",
            };
            for operand in operands {
                let Some(place) = operand.place() else {
                    continue;
                };
                if !places::contains_function(&place.ty(body, &file.types)) {
                    continue;
                }
                let items = flow.analysis.holds(state, place.local);
                escaped.extend(checker.loans(&items));
                let value = Value::Local(place.local);
                errors.extend(checker.error(
                    state,
                    &items,
                    &value,
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
            .rvalue_items(state, rvalue, location)
            .into_iter()
            .map(|item| borrows.item(item))
            .collect();
        let (value, span) = match rvalue.as_ref() {
            Rvalue::Use(operand) | Rvalue::WrapNullable(operand) | Rvalue::Subtype(operand) => (
                operand
                    .place()
                    .map_or(Value::Other, |place| Value::Local(place.local)),
                operand_span(operand, statement.span),
            ),
            Rvalue::Closure { captures, .. } => (Value::Closure(captures), statement.span),
            _ => (Value::Other, statement.span),
        };
        escaped.extend(checker.loans(&items));
        errors.extend(checker.error(state, &items, &value, position, span));
    }
    (errors, escaped)
}

fn operand_span(operand: &Operand, otherwise: Span) -> Span {
    match operand {
        Operand::Copy(_, span) | Operand::Move(_, span) => *span,
        Operand::Constant(_) => otherwise,
    }
}

/// The escaping value, as far as the diagnostic needs it.
enum Value<'a> {
    /// A closure built at the escape, from these captures.
    Closure(&'a [Capture]),
    /// The value of a local.
    Local(LocalId),
    Other,
}

struct Escapes<'a> {
    file: &'a MirFile,
    body: &'a Body,
    borrows: &'a BorrowSet,
    flow: &'a LoanFlow<'a>,
}

impl Escapes<'_> {
    /// The loans among `items`.
    fn loans(&self, items: &[Item]) -> Vec<LoanId> {
        items
            .iter()
            .filter_map(|item| match item {
                Item::Loan(loan) => Some(*loan),
                Item::Input(_) => None,
            })
            .collect()
    }

    /// What writing a function value to `destination` does, when it is an escape.
    fn position(&self, destination: &Place) -> Option<&'static str> {
        if *destination == Place::return_place() {
            return Some(match self.body.owner {
                BodyOwner::Function(_)
                | BodyOwner::Closure { .. }
                | BodyOwner::Restart { .. }
                | BodyOwner::RootContent { .. } => "returned here",
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

    /// The captures of the closure `value` is: one built at the escape, or the closure a
    /// temporary was assigned once.
    fn closure_captures<'v>(&'v self, value: &'v Value) -> Option<&'v [Capture]> {
        match value {
            Value::Closure(captures) => Some(captures),
            Value::Local(local) if self.body.locals[*local].kind == LocalKind::Temporary => {
                self.body.blocks.iter().find_map(|(_, block)| {
                    block
                        .statements
                        .iter()
                        .find_map(|statement| match &statement.kind {
                            StatementKind::Assign(place, rvalue)
                                if place.as_local() == Some(*local) =>
                            {
                                match rvalue.as_ref() {
                                    Rvalue::Closure { captures, .. } => Some(captures.as_slice()),
                                    _ => None,
                                }
                            }
                            _ => None,
                        })
                })
            }
            _ => None,
        }
    }

    /// The owned capture of the closure `value` whose operand held `item`, with its name and
    /// the span of its first use in the closure.
    fn owning_capture(
        &self,
        state: &Holdings,
        value: &Value,
        item: Item,
    ) -> Option<(Option<String>, Span)> {
        let captures = self.closure_captures(value)?;
        let index = self.borrows.item_index(item);
        captures.iter().find_map(|capture| match capture {
            Capture::Owned(Operand::Copy(place, span) | Operand::Move(place, span)) => self
                .flow
                .capture_items(state, capture)
                .contains(&index)
                .then(|| (self.body.locals[place.local].name.clone(), *span)),
            _ => None,
        })
    }

    /// The error for a value holding `items` that escapes at `span`, when it may hold a
    /// borrow: a loan, or the input of a local carrying loans.
    fn error(
        &self,
        state: &Holdings,
        items: &[Item],
        value: &Value,
        position: &str,
        span: Span,
    ) -> Option<BorrowError> {
        // Inputs first, then loans, each in order: a parameter is the likelier cause to name.
        let item = items
            .iter()
            .copied()
            .find(|item| matches!(item, Item::Input(_)))
            .or_else(|| items.first().copied())?;
        let primary = Label {
            span,
            message: position.to_string(),
        };
        let is_closure = self.closure_captures(value).is_some();
        let (holder, related, notes, helps) = match item {
            Item::Input(local) => {
                let declaration = &self.body.locals[local];
                let name = declaration.name.clone();
                let quoted = places::quoted(&name);
                let captured = self.owning_capture(state, value, item);
                // The capture that holds the input: the input itself, or a function value that
                // holds it, such as a closure that captured it (`{ h() }` for `h = { r }`).
                let captured_label = |(capture, at): (Option<String>, Span)| Label {
                    span: at,
                    message: if capture == name {
                        format!("{} is captured here", quoted)
                    } else {
                        format!(
                            "{}, which holds {}, is captured here",
                            places::quoted(&capture),
                            quoted
                        )
                    },
                };
                let through_holder = captured
                    .as_ref()
                    .is_some_and(|(capture, _)| *capture != name);
                if places::contains_function(&declaration.ty) {
                    let mut related = Vec::new();
                    if let Some(captured) = captured {
                        related.push(captured_label(captured));
                    }
                    related.push(Label {
                        span: declaration.span,
                        message: format!("{} is a parameter here", quoted),
                    });
                    let mut notes = vec![
                        "an unmarked function-typed parameter may borrow its caller's variables"
                            .to_string(),
                    ];
                    if is_closure {
                        notes.insert(
                            0,
                            "a closure that escapes owns what it captures".to_string(),
                        );
                    }
                    let helps = match (&name, declaration.kind) {
                        (Some(name), LocalKind::Argument) => vec![format!(
                            "declare it as '{}: escaping {}' if the function may keep it",
                            name,
                            escaping_type(&declaration.ty)
                        )],
                        _ => Vec::new(),
                    };
                    let holder = if is_closure {
                        EscapingHolder::ClosureOwningParameter(name)
                    } else {
                        EscapingHolder::Parameter(name)
                    };
                    (holder, related, notes, helps)
                } else {
                    let mut related = Vec::new();
                    if let Some(captured) = captured {
                        related.push(captured_label(captured));
                    }
                    if related.is_empty() || through_holder {
                        related.push(Label {
                            span: declaration.span,
                            message: format!("{} is a parameter here", quoted),
                        });
                    }
                    (
                        EscapingHolder::ClosureOwningReference(name),
                        related,
                        vec![
                            "a closure that escapes owns what it captures, and a reference cannot be owned"
                                .to_string(),
                        ],
                        vec![format!(
                            "pass the value {} refers to instead of a reference",
                            quoted
                        )],
                    )
                }
            }
            Item::Loan(loan) => {
                let loan_data = &self.borrows.loans[loan];
                match loan_data.kind {
                    LoanKind::Capture { closure, index } => {
                        let closure_body = &self.file.bodies[closure];
                        let capture = &closure_body.locals
                            [LocalId((closure_body.argument_count + 1 + index) as u32)];
                        let name = capture.name.clone();
                        let quoted = places::quoted(&name);
                        let related = vec![Label {
                            span: capture.span,
                            message: format!("{} is captured here", quoted),
                        }];
                        match self.owning_capture(state, value, item) {
                            Some((holder, _)) => {
                                let holder_quoted = places::quoted(&holder);
                                (
                                    EscapingHolder::ClosureOwningBorrower {
                                        holder,
                                        capture: name,
                                    },
                                    related,
                                    vec![format!(
                                        "{} borrows {}, so a closure that escapes cannot own it",
                                        holder_quoted, quoted
                                    )],
                                    vec![format!(
                                        "write the closure of {} where it escapes, so that it owns {}",
                                        holder_quoted, quoted
                                    )],
                                )
                            }
                            None => (
                                EscapingHolder::ClosureBorrowing(name),
                                related,
                                vec![
                                    "a closure that borrows what it captures can only be used while that is in scope"
                                        .to_string(),
                                ],
                                vec![format!(
                                    "pass {} to the closure as a parameter instead of capturing it",
                                    quoted
                                )],
                            ),
                        }
                    }
                    LoanKind::Borrow(_) => {
                        let borrowed =
                            places::place_name(self.body, &self.file.types, &loan_data.place);
                        let borrowed_quoted = places::quoted(&borrowed);
                        let mut related = Vec::new();
                        let holder = match self.owning_capture(state, value, item) {
                            Some((holder, at)) => {
                                related.push(Label {
                                    span: at,
                                    message: format!(
                                        "{} is captured here",
                                        places::quoted(&holder)
                                    ),
                                });
                                EscapingHolder::ClosureOwningReference(holder)
                            }
                            None => EscapingHolder::Borrow(borrowed),
                        };
                        related.push(Label {
                            span: loan_data.span,
                            message: format!("{} is borrowed here", borrowed_quoted),
                        });
                        (
                            holder,
                            related,
                            vec![
                                "a value that escapes cannot hold a borrow, since it may outlive what it borrows"
                                    .to_string(),
                            ],
                            vec![format!(
                                "pass the value of {} instead of a reference to it",
                                borrowed_quoted
                            )],
                        )
                    }
                }
            }
        };
        Some(BorrowError {
            kind: BorrowErrorKind::EscapingClosure { holder },
            primary,
            related,
            notes,
            helps,
        })
    }
}

/// The type a parameter of type `ty` is declared with when it is `escaping`: a function type,
/// or a nullable one, in parentheses.
fn escaping_type(ty: &Type) -> String {
    ty.to_string()
}
