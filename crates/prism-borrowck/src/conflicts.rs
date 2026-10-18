//! The access check (D6 and D13 of `borrowck-exclusivity`): every access to a place must not
//! conflict with a live loan of an overlapping place.
//!
//! The check visits a body's locations in reverse postorder. Each location's effects are
//! events in evaluation order: accesses to places and uses of loan-carrying locals. A loan is in
//! scope for an access when a local that may hold it (by the loan flow, before the location) is
//! live after the location, or is used by a later event of the location; a call uses all its
//! operands when it is made, after evaluating them, and writes its destination afterwards. An
//! access conflicts with a loan in scope of an overlapping place by D6's table:
//!
//! | Access | Conflicts with | Code |
//! |---|---|---|
//! | read, shared borrow | a mutable loan | `E5003` |
//! | mutable borrow | any loan | `E5003` |
//! | move | any loan | `E5005` |
//! | write | any loan, except of a place reached from the written one through a reference | `E5004` |
//! | drop, end of storage | any loan, except of a place reached through a reference | `E5006` |
//!
//! At each `Return`, every argument whose type needs no drop ends (an access of kind
//! `StorageDead`, labelled at its declaration), before the caller's uses of the returned
//! locals, which are labelled at the `&mut R` parameter they are seen through; an argument that
//! needs a drop ends at its elaborated drop (D11 and D12 of `borrowck-elision`).
//!
//! A drop that the next statement's assignment of the same place follows is the drop of the
//! assigned place's old value: the assignment is checked as the write. Each access reports the
//! earliest conflicting loan by creation, and a dropped value is reported once per loan, and
//! not at all for the capture loan of a closure that escapes, which the escape check reports;
//! other accesses that conflict with a loan reported as dropped are not reported. Of the accesses
//! at one span that conflict with one loan, only the last is reported (the borrow after the tag
//! read of an optional reborrow).

use crate::borrows::{BorrowSet, Item, LoanId, LoanKind};
use crate::captures::CaptureModes;
use crate::check::{clone_help, sort_labels};
use crate::dataflow::{BitSet, Results};
use crate::dominators::Dominators;
use crate::error::{AccessKind, BorrowError, BorrowErrorKind, Label};
use crate::liveness::{returned_locals, Liveness};
use crate::loan_flow::LoanFlow;
use crate::location::{locations, Location};
use crate::places;
use prism_ast::Span;
use prism_mir::{
    Body, BodyOwner, BorrowOrigin, Callee, LocalId, LocalKind, MirFile, Operand, Place, Rvalue,
    StatementKind, TerminatorKind,
};
use std::collections::{HashSet, VecDeque};

/// One effect of a location, in evaluation order.
#[derive(Debug, Clone)]
pub(crate) enum Event {
    /// An access to a place, at the span of the source that makes it.
    Access {
        place: Place,
        kind: AccessKind,
        span: Span,
    },
    /// A use of the value of a local, which keeps the loans it holds live. `by_caller` marks a
    /// `Return`'s use of an argument that the caller sees afterwards (a `&mut R` parameter),
    /// whose span is the parameter's declaration (D12 of `borrowck-elision`).
    Use {
        local: LocalId,
        span: Span,
        by_caller: bool,
    },
}

impl Event {
    /// A use of `local` at `span` within the body.
    fn using(local: LocalId, span: Span) -> Self {
        Event::Use {
            local,
            span,
            by_caller: false,
        }
    }
}

/// The events of `location` in `body`, in evaluation order.
pub(crate) fn location_events(
    file: &MirFile,
    body: &Body,
    location: Location,
    returned: &[LocalId],
) -> Vec<Event> {
    let mut events = Vec::new();
    let block = &body.blocks[location.block];
    match block.statements.get(location.statement) {
        Some(statement) => {
            let span = statement.span;
            match &statement.kind {
                StatementKind::Assign(destination, rvalue) => {
                    rvalue_events(file, body, rvalue, span, &mut events);
                    destination_events(destination, span, &mut events);
                }
                StatementKind::Read(place) => {
                    access(&mut events, place, AccessKind::Read, span);
                }
                // One whole-value move of the source, then a write of each part (D16 of
                // `payload-patterns`).
                StatementKind::Destructure { source, parts, .. } => {
                    access(&mut events, &Place::local(*source), AccessKind::Move, span);
                    for (_, target) in parts {
                        destination_events(&Place::local(*target), span, &mut events);
                    }
                }
                StatementKind::StorageDead(local) => events.push(Event::Access {
                    place: Place::local(*local),
                    kind: AccessKind::StorageDead,
                    span,
                }),
                StatementKind::StorageLive(_) => {}
            }
        }
        None => {
            let terminator = &block.terminator;
            let span = terminator.span;
            match &terminator.kind {
                TerminatorKind::SwitchInt { value, .. } => {
                    operand_events(file, body, value, &mut events)
                }
                TerminatorKind::Call {
                    callee,
                    arguments,
                    destination,
                    ..
                } => {
                    let mut uses = Vec::new();
                    if let Callee::Value(place, callee_span) = callee {
                        events.push(Event::Access {
                            place: place.clone(),
                            kind: AccessKind::Read,
                            span: *callee_span,
                        });
                        uses.push(Event::using(place.local, *callee_span));
                    }
                    for argument in arguments {
                        let Some(place) = argument.place() else {
                            continue;
                        };
                        events.push(Event::Access {
                            place: place.clone(),
                            kind: operand_access(file, body, argument),
                            span: operand_span(argument),
                        });
                        // A temporary passed to a call is used by the call itself.
                        let use_span = if body.locals[place.local].kind == LocalKind::Temporary {
                            span
                        } else {
                            operand_span(argument)
                        };
                        uses.push(Event::using(place.local, use_span));
                    }
                    events.extend(uses);
                    destination_events(destination, span, &mut events);
                }
                TerminatorKind::Drop { place, target } => {
                    let replaced = body.blocks[*target].statements.first().is_some_and(|next| {
                        matches!(&next.kind, StatementKind::Assign(written, _) if written == place)
                    });
                    if !replaced {
                        events.push(Event::Access {
                            place: place.clone(),
                            kind: AccessKind::Drop,
                            span,
                        });
                    }
                }
                TerminatorKind::Return => {
                    // The arguments end with the body (D11 of `borrowck-elision`): those that
                    // need a drop at their elaborated drop or by being moved, the others here,
                    // before the caller sees what the returned locals hold.
                    for local in body.arguments() {
                        let declaration = &body.locals[local];
                        if !prism_mir::needs_drop(&declaration.ty, &file.types) {
                            events.push(Event::Access {
                                place: Place::local(local),
                                kind: AccessKind::StorageDead,
                                span: declaration.span,
                            });
                        }
                    }
                    for local in returned {
                        let declaration = &body.locals[*local];
                        let by_caller = declaration.kind == LocalKind::Argument;
                        events.push(Event::Use {
                            local: *local,
                            span: if by_caller { declaration.span } else { span },
                            by_caller,
                        });
                    }
                }
                TerminatorKind::Goto(_) | TerminatorKind::Unreachable => {}
            }
        }
    }
    events
}

/// An access to `place`, then the use of its root local.
fn access(events: &mut Vec<Event>, place: &Place, kind: AccessKind, span: Span) {
    events.push(Event::Access {
        place: place.clone(),
        kind,
        span,
    });
    events.push(Event::using(place.local, span));
}

/// How `operand` accesses its place: a move of a `Copy` value is a read.
fn operand_access(file: &MirFile, body: &Body, operand: &Operand) -> AccessKind {
    match operand {
        Operand::Move(place, _) if !file.types.is_copy(&place.ty(body, &file.types)) => {
            AccessKind::Move
        }
        _ => AccessKind::Read,
    }
}

fn operand_span(operand: &Operand) -> Span {
    match operand {
        Operand::Copy(_, span) | Operand::Move(_, span) => *span,
        Operand::Constant(_) => Span::dummy(),
    }
}

fn operand_events(file: &MirFile, body: &Body, operand: &Operand, events: &mut Vec<Event>) {
    if let Some(place) = operand.place() {
        access(
            events,
            place,
            operand_access(file, body, operand),
            operand_span(operand),
        );
    }
}

fn rvalue_events(
    file: &MirFile,
    body: &Body,
    rvalue: &Rvalue,
    span: Span,
    events: &mut Vec<Event>,
) {
    match rvalue {
        Rvalue::Use(operand)
        | Rvalue::Unary(_, operand)
        | Rvalue::WrapNullable(operand)
        | Rvalue::Subtype(operand) => operand_events(file, body, operand, events),
        Rvalue::Binary(_, left, right) => {
            operand_events(file, body, left, events);
            operand_events(file, body, right, events);
        }
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                operand_events(file, body, operand, events);
            }
        }
        Rvalue::Borrow { mutable, place, .. } => {
            let kind = if *mutable {
                AccessKind::MutableBorrow
            } else {
                AccessKind::SharedBorrow
            };
            access(events, place, kind, span);
        }
        Rvalue::Discriminant(place) => access(events, place, AccessKind::Read, span),
        Rvalue::Closure {
            body: closure,
            captures,
        } => {
            let modes = &file.bodies[*closure].capture_modes;
            for (index, place) in captures.iter().enumerate() {
                let kind = match modes.get(index) {
                    Some(prism_mir::CaptureMode::Mutable) => AccessKind::MutableBorrow,
                    _ => AccessKind::SharedBorrow,
                };
                access(events, place, kind, span);
            }
        }
    }
}

/// The write of `destination`: a write through or into a place uses its root local first.
fn destination_events(destination: &Place, span: Span, events: &mut Vec<Event>) {
    if !destination.projections.is_empty() {
        events.push(Event::using(destination.local, span));
    }
    events.push(Event::Access {
        place: destination.clone(),
        kind: AccessKind::Write,
        span,
    });
}

/// The conflicts of `body` of `file`.
pub(crate) fn check_conflicts(
    file: &MirFile,
    body: &Body,
    captures: &CaptureModes,
    borrows: &BorrowSet,
    liveness: &Results<Liveness>,
    flow: &Results<LoanFlow>,
    escaped: &HashSet<LoanId>,
) -> Vec<BorrowError> {
    let checker = Checker {
        file,
        body,
        captures,
        borrows,
        flow,
        returned: returned_locals(body, borrows),
        dominators: Dominators::compute(body),
    };
    let mut live_cursor = liveness.cursor(body);
    let mut flow_cursor = flow.cursor(body);
    let mut reported_drops: HashSet<LoanId> = HashSet::new();
    let mut errors = Vec::new();
    for location in locations(body) {
        let live_after = live_cursor.seek(location).clone();
        let state = flow_cursor.seek(location).clone();
        let events = location_events(file, body, location, &checker.returned);
        for (index, event) in events.iter().enumerate() {
            let Event::Access { place, kind, span } = event else {
                continue;
            };
            let in_scope = live_after_event(&events[index + 1..], borrows, |local| {
                liveness.analysis.is_live(&live_after, local)
            });
            let mut loans: Vec<LoanId> = in_scope
                .iter()
                .flat_map(|local| flow.analysis.holds(&state, *local))
                .filter_map(|item| match item {
                    Item::Loan(loan) => Some(loan),
                    Item::Input(_) => None,
                })
                .filter(|loan| checker.conflicts(place, *kind, *loan))
                .collect();
            loans.sort();
            let Some(&loan) = loans.first() else {
                continue;
            };
            // A value that ends while an escaping closure borrows it is reported as the escape.
            if matches!(kind, AccessKind::Drop | AccessKind::StorageDead)
                && (escaped.contains(&loan) || !reported_drops.insert(loan))
            {
                continue;
            }
            let error = checker.error(location, index, &events, place, *kind, *span, loan);
            // An access that conflicts with the loan an earlier one at the same span conflicts
            // with is the same mistake, seen more precisely: the borrow after the tag read of an
            // optional reborrow (`both(m, m)` for `m: (&mut T)?`).
            errors.retain(
                |(earlier, _, reported): &(LoanId, AccessKind, BorrowError)| {
                    *earlier != loan || reported.primary.span != *span
                },
            );
            errors.push((loan, *kind, error));
        }
    }
    // A value that ends while borrowed is reported once: an access that conflicts with the
    // same loan later, such as the value's next iteration in a loop, is a consequence of it.
    errors
        .into_iter()
        .filter(|(loan, kind, _)| {
            matches!(kind, AccessKind::Drop | AccessKind::StorageDead)
                || !reported_drops.contains(loan)
        })
        .map(|(_, _, error)| error)
        .collect()
}

/// The loan-carrying locals live just after an event, given the events that follow it in its
/// location and the locals live after the location: liveness applied backward through those
/// events, so that a local the location wholly reassigns does not keep its old loans in scope.
fn live_after_event(
    later: &[Event],
    borrows: &BorrowSet,
    live_after_location: impl Fn(LocalId) -> bool,
) -> Vec<LocalId> {
    let mut live: Vec<bool> = borrows
        .carriers
        .iter()
        .map(|local| live_after_location(*local))
        .collect();
    for event in later.iter().rev() {
        match event {
            Event::Use { local, .. } => {
                if let Some(bit) = borrows.carrier(*local) {
                    live[bit] = true;
                }
            }
            Event::Access {
                place,
                kind: AccessKind::Write | AccessKind::StorageDead,
                ..
            } if place.projections.is_empty() => {
                if let Some(bit) = borrows.carrier(place.local) {
                    live[bit] = false;
                }
            }
            Event::Access { .. } => {}
        }
    }
    borrows
        .carriers
        .iter()
        .zip(live)
        .filter_map(|(local, live)| live.then_some(*local))
        .collect()
}

struct Checker<'a> {
    file: &'a MirFile,
    body: &'a Body,
    captures: &'a CaptureModes,
    borrows: &'a BorrowSet,
    flow: &'a Results<LoanFlow<'a>>,
    returned: Vec<LocalId>,
    dominators: Dominators,
}

impl Checker<'_> {
    /// Whether an access of `kind` to `place` conflicts with `loan`.
    fn conflicts(&self, place: &Place, kind: AccessKind, loan: LoanId) -> bool {
        let loan = &self.borrows.loans[loan];
        if !places::places_overlap(place, &loan.place) {
            return false;
        }
        match kind {
            AccessKind::Read | AccessKind::SharedBorrow => loan.mutable,
            AccessKind::MutableBorrow | AccessKind::Move => true,
            AccessKind::Write | AccessKind::Drop | AccessKind::StorageDead => {
                !places::extends_through_reference(self.body, &self.file.types, &loan.place, place)
            }
        }
    }

    fn name(&self, place: &Place) -> Option<String> {
        places::place_name(self.body, &self.file.types, place)
    }

    #[allow(clippy::too_many_arguments)]
    fn error(
        &self,
        location: Location,
        index: usize,
        events: &[Event],
        place: &Place,
        kind: AccessKind,
        span: Span,
        loan_id: LoanId,
    ) -> BorrowError {
        let loan = &self.borrows.loans[loan_id];
        let place_name = self.name(place);
        let borrowed = self.name(&loan.place);
        let later = self.later_use(location, index, events, loan_id);
        let mut related = Vec::new();
        let mut helps = Vec::new();
        let (kind_of_error, primary) = match kind {
            AccessKind::Drop | AccessKind::StorageDead => {
                let name = place_name.clone();
                let default_slot =
                    loan.mutable && loan.kind == LoanKind::Borrow(BorrowOrigin::Default);
                let ends_with_body = place.projections.is_empty()
                    && self.body.locals[place.local].kind == LocalKind::Argument;
                let (primary, at_end) = match &name {
                    Some(name) if ends_with_body => (
                        "borrowed value does not live long enough".to_string(),
                        format!(
                            "'{}' is dropped when the {} returns, while still borrowed",
                            name,
                            if matches!(self.body.owner, BodyOwner::Closure { .. }) {
                                "closure"
                            } else {
                                "function"
                            }
                        ),
                    ),
                    Some(name) => (
                        "borrowed value does not live long enough".to_string(),
                        format!("'{}' dropped here while still borrowed", name),
                    ),
                    // The slot of an omitted reference argument, which its default may fill.
                    None if default_slot => (
                        "the default value of an omitted reference argument is held in a temporary here"
                            .to_string(),
                        "temporary value is freed at the end of this statement".to_string(),
                    ),
                    None => (
                        "this borrows a temporary value".to_string(),
                        "temporary value is freed at the end of this statement".to_string(),
                    ),
                };
                if name.is_none() {
                    helps.push(if default_slot {
                        "pass the argument explicitly, bound to a local with 'val', so that it lives as long as its block"
                            .to_string()
                    } else {
                        "bind the value to a local with 'val' before borrowing it, so that it lives as long as its block"
                            .to_string()
                    });
                }
                // An argument ends with the body, so it is labelled at its declaration (D12 of
                // `borrowck-elision`).
                let end_span = if ends_with_body {
                    self.body.locals[place.local].span
                } else {
                    span
                };
                related.push(Label {
                    span: end_span,
                    message: at_end,
                });
                if let Some(later) = later {
                    related.push(later.label("borrow later used here"));
                }
                sort_labels(&mut related);
                return BorrowError {
                    kind: BorrowErrorKind::DroppedWhileBorrowed { name },
                    primary: Label {
                        span: loan.span,
                        message: primary,
                    },
                    related,
                    notes: Vec::new(),
                    helps,
                };
            }
            AccessKind::Write => (
                BorrowErrorKind::AssignToBorrowed {
                    place: place_name.clone(),
                    borrowed: borrowed.clone(),
                },
                format!(
                    "{} is assigned here while borrowed",
                    places::quoted(&place_name)
                ),
            ),
            AccessKind::Move => {
                if let Some(name) = &place_name {
                    let ty = place.ty(self.body, &self.file.types);
                    helps.extend(clone_help(self.file, name, &ty));
                }
                (
                    BorrowErrorKind::MoveOutWhileBorrowed {
                        place: place_name.clone(),
                        borrowed: borrowed.clone(),
                    },
                    format!(
                        "{} is moved here while borrowed",
                        places::quoted(&place_name)
                    ),
                )
            }
            AccessKind::Read | AccessKind::SharedBorrow | AccessKind::MutableBorrow => {
                let label = match kind {
                    AccessKind::MutableBorrow if loan.mutable => {
                        "second mutable borrow occurs here".to_string()
                    }
                    AccessKind::MutableBorrow => "mutable borrow occurs here".to_string(),
                    AccessKind::SharedBorrow => "shared borrow occurs here".to_string(),
                    _ => format!("use of borrowed {}", places::quoted(&borrowed)),
                };
                (
                    BorrowErrorKind::BorrowConflict {
                        place: place_name.clone(),
                        borrowed: borrowed.clone(),
                        access: kind,
                        loan_mutable: loan.mutable,
                    },
                    label,
                )
            }
        };
        let conflict = matches!(
            kind,
            AccessKind::Read | AccessKind::SharedBorrow | AccessKind::MutableBorrow
        );
        let both_mutable = kind == AccessKind::MutableBorrow && loan.mutable;
        match loan.kind {
            LoanKind::Capture { closure, index } => {
                let captured = self.file.bodies[closure].locals
                    [LocalId((self.file.bodies[closure].argument_count + 1 + index) as u32)]
                .name
                .clone();
                let captured = places::quoted(&captured);
                related.push(Label {
                    span: loan.span,
                    message: if loan.mutable {
                        format!("closure borrows {} mutably here", captured)
                    } else {
                        format!("closure borrows {} here", captured)
                    },
                });
                if let Some(deciding) = self.captures.deciding_use(closure, index) {
                    related.push(Label {
                        span: deciding,
                        message: format!(
                            "borrow occurs due to this use of {} in the closure",
                            captured
                        ),
                    });
                }
            }
            LoanKind::Borrow(_) => related.push(Label {
                span: loan.span,
                message: if !conflict {
                    format!("{} is borrowed here", places::quoted(&borrowed))
                } else if both_mutable {
                    "first mutable borrow occurs here".to_string()
                } else if loan.mutable {
                    "mutable borrow occurs here".to_string()
                } else {
                    "shared borrow occurs here".to_string()
                },
            }),
        }
        if let Some(later) = later {
            let message = if !conflict {
                "borrow later used here"
            } else if both_mutable {
                "first borrow later used here"
            } else if loan.mutable {
                "mutable borrow later used here"
            } else {
                "shared borrow later used here"
            };
            related.push(later.label(message));
        }
        sort_labels(&mut related);
        BorrowError {
            kind: kind_of_error,
            primary: Label {
                span,
                message: primary,
            },
            related,
            notes: Vec::new(),
            helps,
        }
    }

    /// Whether `local` may hold `loan` in `state`.
    fn holds(&self, state: &BitSet, local: LocalId, loan: LoanId) -> bool {
        self.flow
            .analysis
            .holds_item(state, local, Item::Loan(loan))
    }

    /// The first use after event `index` of `location` of a local that holds `loan` there, by
    /// breadth-first search.
    fn later_use(
        &self,
        location: Location,
        index: usize,
        events: &[Event],
        loan: LoanId,
    ) -> Option<LaterUse> {
        let mut cursor = self.flow.cursor(self.body);
        let state = cursor.seek(location).clone();
        for event in &events[index + 1..] {
            if let Some(found) = self.use_of(&state, event, loan, false) {
                return Some(found);
            }
        }
        for later_iteration in [false, true] {
            if let Some(found) = self.search(location, loan, later_iteration) {
                return Some(found);
            }
        }
        None
    }

    /// `event`, when it is a use of a local that holds `loan` in `state`.
    fn use_of(
        &self,
        state: &BitSet,
        event: &Event,
        loan: LoanId,
        later_iteration: bool,
    ) -> Option<LaterUse> {
        let Event::Use {
            local,
            span,
            by_caller,
        } = event
        else {
            return None;
        };
        self.holds(state, *local, loan).then(|| LaterUse {
            span: *span,
            later_iteration,
            by_caller: by_caller
                .then(|| self.body.locals[*local].name.clone())
                .flatten(),
        })
    }

    /// The breadth-first search of [`Self::later_use`] from the successors of `location`,
    /// following back edges only when `back_edges` is set.
    fn search(&self, location: Location, loan: LoanId, back_edges: bool) -> Option<LaterUse> {
        let mut cursor = self.flow.cursor(self.body);
        let mut visited: HashSet<Location> = HashSet::new();
        let mut queue: VecDeque<Location> = self.successors(location, back_edges).into();
        while let Some(next) = queue.pop_front() {
            if !visited.insert(next) {
                continue;
            }
            let state = cursor.seek(next).clone();
            let events = location_events(self.file, self.body, next, &self.returned);
            for event in &events {
                if let Some(found) = self.use_of(&state, event, loan, back_edges) {
                    return Some(found);
                }
            }
            queue.extend(self.successors(next, back_edges));
        }
        None
    }

    /// The locations that follow `location`.
    fn successors(&self, location: Location, back_edges: bool) -> Vec<Location> {
        if !location.is_terminator(self.body) {
            return vec![Location {
                block: location.block,
                statement: location.statement + 1,
            }];
        }
        self.body.blocks[location.block]
            .terminator
            .kind
            .successors()
            .into_iter()
            .filter(|target| back_edges || !self.dominators.is_back_edge(location.block, *target))
            .map(Location::start)
            .collect()
    }
}

/// A later use of a loan: where, whether it is reached only through a loop's back edge, and,
/// for the caller's use through a parameter after the return, that parameter's name.
struct LaterUse {
    span: Span,
    later_iteration: bool,
    by_caller: Option<String>,
}

impl LaterUse {
    /// Its label, from `message` ("borrow later used here"): "... by the caller through 'slot'"
    /// for the caller's use, ", in later iteration of loop" when reached only around a loop.
    fn label(&self, message: &str) -> Label {
        let message = match &self.by_caller {
            Some(parameter) => format!(
                "{} by the caller through '{}'",
                message.trim_end_matches(" here"),
                parameter
            ),
            None if self.later_iteration => format!("{}, in later iteration of loop", message),
            None => message.to_string(),
        };
        Label {
            span: self.span,
            message,
        }
    }
}
