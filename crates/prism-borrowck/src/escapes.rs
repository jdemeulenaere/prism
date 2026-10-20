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
//!
//! A closure given to an `attached` parameter holds the loans of its weak captures, and a value
//! of an `attached` parameter its input: both may be passed to `attached` parameters, and
//! anywhere else they escape is `AttachedFunctionEscapes` (`E5010`, D9 and D24 of
//! `owned-state`). A keeper (`Composition`) holding loans may be moved between locals and
//! returned under the elision rules, never stored where loans are not tracked: `E5010` for the
//! weak captures of its content, `E5007` for other borrows.

use crate::borrows::{BorrowSet, Item, LoanId, LoanKind};
use crate::dataflow::Results;
use crate::error::{AttachedHolder, BorrowError, BorrowErrorKind, EscapingHolder, Label};
use crate::loan_flow::{Holdings, LoanFlow};
use crate::location::locations;
use crate::places;
use prism_ast::Span;
use prism_mir::{
    AggregateKind, Body, BodyId, BodyOwner, Callee, Capture, LocalId, LocalKind, MirFile, Operand,
    Place, Projection, Rvalue, StatementKind, TerminatorKind,
};
use prism_typed_tree::{BuiltinFunction, Type, Widening, WideningReason};
use std::collections::{HashMap, HashSet};

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
        closures: temporary_closures(body),
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
                parameter_kinds,
                ..
            } = &terminator.kind
            else {
                continue;
            };
            // A box is an owned container that can be returned or stored anywhere, like a
            // struct (D11 of `heap-boxes`); an `escaping` parameter may be kept by the callee; a
            // generic slot's body sees a value of a type parameter.
            let position = |index: usize, parameter: &Type| -> Option<&'static str> {
                Some(
                    if *callee == Callee::Builtin(BuiltinFunction::HeapAllocate) {
                        "stored in a box here"
                    } else if parameter_kinds
                        .get(index)
                        .is_some_and(|kind| kind.is_escaping())
                    {
                        "passed to an escaping parameter here"
                    } else if parameter_kinds
                        .get(index)
                        .is_some_and(|kind| kind.is_attached())
                    {
                        "passed to an attached parameter here"
                    } else if matches!(callee, Callee::State(_))
                        && parameter.mentions_parameter_outside_functions()
                    {
                        // A state keeps the value it is given (D18 of `state-handles`).
                        "stored in a state here"
                    } else if parameter.mentions_parameter_outside_functions() {
                        "passed to a generic parameter here"
                    } else {
                        return None;
                    },
                )
            };
            // The arguments that may escape: those of function or keeper type at an escaping
            // position. The loan flow's state is sought only when there is one, or an `attached`
            // argument that a keeper may keep, so a body without such calls replays nothing.
            let candidates: Vec<(usize, &'static str, &Place, Type)> = arguments
                .iter()
                .zip(parameters)
                .enumerate()
                .filter_map(|(index, (operand, parameter))| {
                    let position = position(index, parameter)?;
                    let place = operand.place()?;
                    let ty = escape_candidate_type(body, file, place)?;
                    Some((index, position, place, ty))
                })
                .collect();
            if candidates.is_empty() && !parameter_kinds.iter().any(|kind| kind.is_attached()) {
                continue;
            }
            let state = cursor.seek(location);
            for (index, position, place, ty) in candidates {
                let operand = &arguments[index];
                let attached = parameter_kinds
                    .get(index)
                    .is_some_and(|kind| kind.is_attached());
                let items = flow.analysis.holds(state, place.local);
                let value = Value::Local(place.local);
                // What an attached parameter keeps stays borrowed in this body, by what keeps
                // it (D24 of `owned-state`): its end is still checked.
                let escaping = if attached {
                    checker.partition_attached(state, &items, &value).1
                } else {
                    items.clone()
                };
                escaped.extend(checker.loans(&escaping));
                errors.extend(checker.escape(
                    state,
                    &items,
                    &value,
                    &ty,
                    position,
                    attached,
                    operand_span(operand, terminator.span),
                ));
            }
            errors.extend(checker.untracked_keepers(
                state,
                arguments,
                parameters,
                parameter_kinds,
                terminator.span,
            ));
            continue;
        };
        let StatementKind::Assign(destination, rvalue) = &statement.kind else {
            continue;
        };
        // The loan flow's state is sought only for a statement that may make a value escape:
        // an aggregate of a function or keeper operand, or a write of one where it escapes.
        let aggregate_escape = match rvalue.as_ref() {
            Rvalue::Aggregate(_, operands) => operands.iter().any(|operand| {
                operand
                    .place()
                    .is_some_and(|place| escape_candidate_type(body, file, place).is_some())
            }),
            _ => false,
        };
        if !aggregate_escape
            && (checker.position(destination).is_none()
                || escape_candidate_type(body, file, destination).is_none())
        {
            continue;
        }
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
                let ty = place.ty(body, &file.types);
                if !places::contains_function(&ty) && !places::contains_keeper(&ty) {
                    continue;
                }
                let items = flow.analysis.holds(state, place.local);
                escaped.extend(checker.loans(&items));
                let value = Value::Local(place.local);
                errors.extend(checker.escape(
                    state,
                    &items,
                    &value,
                    &ty,
                    position,
                    false,
                    operand_span(operand, statement.span),
                ));
            }
        }
        let ty = destination.ty(body, &file.types);
        let keeper = places::contains_keeper(&ty);
        if !places::contains_function(&ty) && !keeper {
            continue;
        }
        let Some(position) = checker.position(destination) else {
            continue;
        };
        // A keeper's returned borrows follow the elision rules (`returns`).
        if keeper && *destination == Place::return_place() {
            continue;
        }
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
            Rvalue::Closure { body, captures } => (Value::Closure(*body, captures), statement.span),
            _ => (Value::Other, statement.span),
        };
        escaped.extend(checker.loans(&items));
        errors.extend(checker.escape(state, &items, &value, &ty, position, false, span));
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
    /// A closure built at the escape, of this body, from these captures.
    Closure(BodyId, &'a [Capture]),
    /// The value of a local.
    Local(LocalId),
    Other,
}

struct Escapes<'a> {
    file: &'a MirFile,
    body: &'a Body,
    borrows: &'a BorrowSet,
    flow: &'a LoanFlow<'a>,
    /// The closure each temporary is first assigned, so finding a value's closure takes
    /// constant time at each of a body's escapes.
    closures: HashMap<LocalId, (BodyId, &'a [Capture])>,
}

/// The closure each temporary of `body` is first assigned, in block and statement order.
fn temporary_closures(body: &Body) -> HashMap<LocalId, (BodyId, &[Capture])> {
    let mut closures = HashMap::new();
    for (_, block) in body.blocks.iter() {
        for statement in &block.statements {
            let StatementKind::Assign(place, rvalue) = &statement.kind else {
                continue;
            };
            let (
                Some(local),
                Rvalue::Closure {
                    body: closure,
                    captures,
                },
            ) = (place.as_local(), rvalue.as_ref())
            else {
                continue;
            };
            if body.locals[local].kind == LocalKind::Temporary {
                closures
                    .entry(local)
                    .or_insert((*closure, captures.as_slice()));
            }
        }
    }
    closures
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
    fn closure_captures<'v>(&'v self, value: &'v Value) -> Option<(BodyId, &'v [Capture])> {
        match value {
            Value::Closure(body, captures) => Some((*body, captures)),
            Value::Local(local) => self.closures.get(local).copied(),
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
        self.owning_capture_index(state, value, item)
            .map(|(name, span, _)| (name, span))
    }

    /// [`Self::owning_capture`], with the path the closure's body widened the capture from
    /// (D10 of `path-captures`).
    fn owning_capture_index(
        &self,
        state: &Holdings,
        value: &Value,
        item: Item,
    ) -> Option<(Option<String>, Span, Option<&Widening>)> {
        let (body, captures) = self.closure_captures(value)?;
        let index = self.borrows.item_index(item);
        captures
            .iter()
            .enumerate()
            .find_map(|(position, capture)| match capture {
                // A moved input (a `&mut` reference) no longer holds its items where the closure
                // escapes, so it is matched by its local.
                Capture::Owned(Operand::Copy(place, span) | Operand::Move(place, span)) => {
                    (self.flow.capture_items(state, capture).contains(&index)
                        || item == Item::Input(place.local))
                    .then(|| {
                        let widening = self
                            .file
                            .bodies
                            .get(body)
                            .and_then(|closure| closure.capture_widenings.get(position))
                            .and_then(Option::as_ref);
                        (self.body.locals[place.local].name.clone(), *span, widening)
                    })
                }
                _ => None,
            })
    }

    /// The help and note of `E5007` for a closure owning the reference `name`, of type `ty`,
    /// which its body widened from a path it could not copy (D10 of `path-captures`): a copy or
    /// clone of the path into a `val`, and why the path was not copied.
    fn widened_help(
        &self,
        name: &str,
        ty: &Type,
        widening: &Widening,
    ) -> Option<(String, Option<String>)> {
        let types = &self.file.types;
        let path = prism_typed_tree::path_text(name, ty, &widening.projections, types);
        let path_type = prism_typed_tree::path_type(ty, &widening.projections, types)?;
        let expression = path.trim_start_matches('*');
        let field = expression.rsplit('.').next().filter(|field| *field != name);
        let fresh = field.unwrap_or(if name == "m" { "k" } else { "m" });
        // A state is captured weakly by a closure the composition keeps, and through an
        // explicit weak reference by any other (D10 and D11 of `owned-state`).
        if types.has_capture_function(&path_type) {
            let base = field.unwrap_or(expression);
            let base = base.rsplit('.').next().unwrap_or(base);
            let help = format!(
                "make a weak reference before the closure, 'val {}Ref = {}.weak()', and use '{}Ref.get()' inside it",
                base, expression, base
            );
            let note = match widening.reason {
                WideningReason::VariableField | WideningReason::MutableReference => Some(
                    "a closure captures a state read through a reference weakly only when every field on the way is a 'val' and no reference on the way is mutable"
                        .to_string(),
                ),
                WideningReason::NotCopy | WideningReason::VariableBinding => None,
            };
            return Some((help, note));
        }
        let help = if types.is_copy(&path_type) {
            format!(
                "copy '{}' into a 'val' before the closure and capture that instead: 'val {} = {}'",
                path, fresh, expression
            )
        } else if types.is_clone(&path_type) {
            format!(
                "clone '{}' into a 'val' before the closure and capture that instead: 'val {} = {}.clone()'",
                path, fresh, expression
            )
        } else {
            return None;
        };
        let note = match widening.reason {
            WideningReason::VariableField => Some(
                "a closure copies a field read through a reference only when the field is a 'val'"
                    .to_string(),
            ),
            WideningReason::MutableReference => Some(format!(
                "'{}' is a mutable reference, whose referent may be replaced",
                name
            )),
            WideningReason::NotCopy | WideningReason::VariableBinding => None,
        };
        Some((help, note))
    }

    /// Whether `item` is attached rather than a borrow (D24 of `owned-state`): a loan of a weak
    /// capture, or the input of an `attached` parameter.
    fn is_attached_item(&self, item: &Item) -> bool {
        match item {
            Item::Loan(loan) => matches!(
                self.borrows.loans[*loan].kind,
                LoanKind::Capture { weak: true, .. }
            ),
            Item::Input(local) => self.body.locals[*local].parameter_kind.is_attached(),
        }
    }

    /// `items`, held by `value`, split into what is attached to the composition (weak captures,
    /// `attached` parameters, and what a closure holds through its weak captures alone, such as
    /// the input of the reference a captured path goes through, `*count`) and the rest.
    fn partition_attached(
        &self,
        state: &Holdings,
        items: &[Item],
        value: &Value,
    ) -> (Vec<Item>, Vec<Item>) {
        // Only a closure with weak captures holding something else needs its captures read.
        let through_weak: Vec<Item> = match self.closure_captures(value) {
            Some((_, captures))
                if items.iter().any(|item| !self.is_attached_item(item))
                    && captures
                        .iter()
                        .any(|capture| matches!(capture, Capture::Weak(_))) =>
            {
                let items_of = |weak: bool| -> Vec<Item> {
                    captures
                        .iter()
                        .filter(|capture| matches!(capture, Capture::Weak(_)) == weak)
                        .flat_map(|capture| self.flow.capture_items(state, capture))
                        .map(|item| self.borrows.item(item))
                        .collect()
                };
                let others = items_of(false);
                items_of(true)
                    .into_iter()
                    .filter(|item| !others.contains(item))
                    .collect()
            }
            _ => Vec::new(),
        };
        items
            .iter()
            .copied()
            .partition(|item| self.is_attached_item(item) || through_weak.contains(item))
    }

    /// The error for a value of type `ty` holding `items` that escapes at `span` (`position`):
    /// `E5007` for a borrow it may hold, else, unless the position is an `attached` parameter,
    /// `E5010` for what is attached to the composition.
    #[allow(clippy::too_many_arguments)]
    fn escape(
        &self,
        state: &Holdings,
        items: &[Item],
        value: &Value,
        ty: &Type,
        position: &str,
        attached_position: bool,
        span: Span,
    ) -> Option<BorrowError> {
        let (attached, borrowed) = self.partition_attached(state, items, value);
        if !borrowed.is_empty() {
            return self.error(state, &borrowed, value, position, span);
        }
        if attached_position {
            return None;
        }
        let item = *attached.first()?;
        Some(self.attached_error(item, ty, position, span))
    }

    /// `E5010` for each keeper given by reference to a call that keeps what its `attached`
    /// arguments hold (`app.c.setContent { ... }`), when the keeper is held where loans are not
    /// tracked: in a field of a struct, an element, a box's or a state's contents (D24 of
    /// `owned-state`). A keeper in a local, or reached through a reference parameter, is
    /// tracked by the loan flow.
    fn untracked_keepers(
        &self,
        state: &Holdings,
        arguments: &[Operand],
        parameters: &[Type],
        parameter_kinds: &[prism_mir::ParameterKind],
        span: Span,
    ) -> Option<BorrowError> {
        let attached: Vec<Item> = arguments
            .iter()
            .zip(parameter_kinds)
            .filter(|(_, kind)| kind.is_attached())
            .filter_map(|(operand, _)| operand.place())
            .flat_map(|place| {
                let items = self.flow.holds(state, place.local);
                self.partition_attached(state, &items, &Value::Local(place.local))
                    .0
            })
            .collect();
        let item = *attached.first()?;
        for (operand, parameter) in arguments.iter().zip(parameters) {
            let Some(keeper) = referent(parameter).filter(places::contains_keeper) else {
                continue;
            };
            let Some(place) = operand.place() else {
                continue;
            };
            let untracked = self
                .flow
                .holds(state, place.local)
                .iter()
                .any(|item| match item {
                    Item::Loan(loan) => !self.tracks_loans(&self.borrows.loans[*loan].place),
                    Item::Input(_) => false,
                });
            if untracked {
                return Some(self.attached_error(
                    item,
                    &keeper,
                    "given to a composition held in a field, an element, a box or a state here",
                    operand_span(operand, span),
                ));
            }
        }
        None
    }

    /// Whether the loan flow tracks what `place` holds: every place on the way to it, from its
    /// local, carries loans.
    fn tracks_loans(&self, place: &Place) -> bool {
        let mut prefix = Place::local(place.local);
        for projection in &place.projections {
            if !places::carries_loans(&prefix.ty(self.body, &self.file.types)) {
                return false;
            }
            prefix.projections.push(*projection);
        }
        places::carries_loans(&prefix.ty(self.body, &self.file.types))
    }

    /// `E5010` for a value of type `ty` holding the attached `item` that escapes at `span`.
    fn attached_error(&self, item: Item, ty: &Type, position: &str, span: Span) -> BorrowError {
        let primary = Label {
            span,
            message: position.to_string(),
        };
        let (holder, related, notes) = match item {
            Item::Input(local) => {
                let declaration = &self.body.locals[local];
                let name = declaration.name.clone();
                let related = vec![Label {
                    span: declaration.span,
                    message: format!("{} is an attached parameter here", places::quoted(&name)),
                }];
                let holder = if places::contains_keeper(ty) {
                    AttachedHolder::Composition(name)
                } else {
                    AttachedHolder::Parameter(name)
                };
                (
                    holder,
                    related,
                    vec!["an attached function value is kept only by the composition, which releases it with the group that passed it".to_string()],
                )
            }
            Item::Loan(loan) => {
                let loan_data = &self.borrows.loans[loan];
                let LoanKind::Capture { closure, index, .. } = loan_data.kind else {
                    unreachable!("an attached loan is a weak capture")
                };
                let closure_body = &self.file.bodies[closure];
                let capture =
                    &closure_body.locals[LocalId((closure_body.argument_count + 1 + index) as u32)];
                let name = capture.name.clone();
                let related = vec![Label {
                    span: capture.span,
                    message: format!("{} is captured weakly here", places::quoted(&name)),
                }];
                if places::contains_keeper(ty) {
                    (
                        AttachedHolder::Composition(name.clone()),
                        related,
                        vec![format!(
                            "the composition's content captures {} weakly, so the composition keeps it borrowed while it is used",
                            places::quoted(&name)
                        )],
                    )
                } else {
                    (
                        AttachedHolder::Closure(name),
                        related,
                        vec!["a closure given to an attached parameter captures states weakly, and only the composition may keep it".to_string()],
                    )
                }
            }
        };
        let helps = match &holder {
            AttachedHolder::Composition(_) => vec![
                "keep the composition in a local of the scope that owns the state, or remember the state in the content".to_string(),
            ],
            _ => vec![
                "declare the receiving parameter 'attached', or call the function value here"
                    .to_string(),
            ],
        };
        BorrowError {
            kind: BorrowErrorKind::AttachedFunctionEscapes { holder },
            primary,
            related,
            notes,
            helps,
        }
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
                let captured_widened = self.owning_capture_index(state, value, item);
                let widening = captured_widened
                    .as_ref()
                    .and_then(|(_, _, widening)| *widening);
                let captured = captured_widened.map(|(capture, at, _)| (capture, at));
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
                    let mut notes = vec![
                        "a closure that escapes owns what it captures, and a reference cannot be owned"
                            .to_string(),
                    ];
                    let mut helps = vec![format!(
                        "pass the value {} refers to instead of a reference",
                        quoted
                    )];
                    // The closure reads a path of the reference it could not copy: copy or
                    // clone that path instead (D10 of `path-captures`).
                    let widened = match (&name, widening) {
                        (Some(name), Some(widening)) if !through_holder => {
                            self.widened_help(name, &declaration.ty, widening)
                        }
                        _ => None,
                    };
                    if let Some((help, note)) = widened {
                        helps = vec![help];
                        notes.extend(note);
                    }
                    (
                        EscapingHolder::ClosureOwningReference(name),
                        related,
                        notes,
                        helps,
                    )
                }
            }
            Item::Loan(loan) => {
                let loan_data = &self.borrows.loans[loan];
                match loan_data.kind {
                    LoanKind::Capture { closure, index, .. } => {
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

/// The type of `place` when a value of it may escape: it contains a function type or a
/// keeper. A whole local's type is tested in place, without copying it.
fn escape_candidate_type(body: &Body, file: &MirFile, place: &Place) -> Option<Type> {
    let escapes = |ty: &Type| places::contains_function(ty) || places::contains_keeper(ty);
    if place.projections.is_empty() {
        let ty = &body.locals[place.local].ty;
        return escapes(ty).then(|| ty.clone());
    }
    let ty = place.ty(body, &file.types);
    escapes(&ty).then_some(ty)
}

/// What a reference type refers to, under nullables: `R` for `&R` or `(&R)?`.
fn referent(ty: &Type) -> Option<Type> {
    match ty {
        Type::Reference { inner, .. } => Some(inner.as_ref().clone()),
        Type::Nullable(inner) => referent(inner),
        _ => None,
    }
}
