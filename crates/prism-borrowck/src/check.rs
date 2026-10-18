//! The use check (D6 to D8 and D13 of `borrowck-moves`): every use of a place must find it
//! initialized on every path reaching it.
//!
//! The check visits a body's locations in reverse postorder and replays each location's effects
//! in evaluation order over the states of the two initialization analyses. At a use, the events
//! of the used path, its ancestors and its descendants that reach it decide the error: a move
//! makes it a use after move (`E5001`), a storage start alone a read of an uninitialized value
//! (`E5002`). A third solution of the reaching events, over every edge but the back edges, tells
//! a move made in a previous iteration of a loop from one made earlier in the same iteration.

use crate::analyses::{MaybeInitialized, ReachingUninitializations};
use crate::dataflow::{solve_with, BitSet, Results};
use crate::dominators::Dominators;
use crate::error::{BorrowError, BorrowErrorKind, Label};
use crate::location::{locations, Location};
use crate::move_paths::{Effect, Event, MoveData, MovePathId};
use crate::uses::UseKind;
use prism_ast::{Span, TypeMarker};
use prism_mir::{Body, MirFile, Place, Projection, Rvalue, StatementKind};
use prism_typed_tree::{ArenaId, OwnershipReason, Type, TypeDef};
use std::collections::HashSet;

/// The errors of `body` of `file`, whose move data is `data` and initialization analyses `maybe`
/// and `reaching`, in visiting order.
pub(crate) fn check_body(
    file: &MirFile,
    body: &Body,
    data: &MoveData,
    maybe: &Results<MaybeInitialized>,
    reaching: &Results<ReachingUninitializations>,
) -> Vec<BorrowError> {
    let dominators = Dominators::compute(body);
    let acyclic = solve_with(body, ReachingUninitializations { data }, |from, to| {
        !dominators.is_back_edge(from, to)
    });
    let mut maybe_cursor = maybe.cursor(body);
    let mut reaching_cursor = reaching.cursor(body);
    let mut acyclic_cursor = acyclic.cursor(body);
    let checker = Checker { file, body, data };
    let mut reported: HashSet<(MovePathId, Vec<Event>)> = HashSet::new();
    let mut errors = Vec::new();
    for location in locations(body) {
        // Only a use is checked, so a location without uses (a storage marker, a constant's
        // assignment) needs no state. The cursors' states are brought past the location in
        // place, effect by effect.
        let effects = data.effects(location);
        if !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Use { .. }))
        {
            continue;
        }
        let maybe_state = maybe_cursor.seek_to_apply(location);
        let reaching_state = reaching_cursor.seek_to_apply(location);
        let acyclic_state = acyclic_cursor.seek_to_apply(location);
        for effect in effects {
            if let Effect::Use { path, kind, span } = effect {
                let events = reaching.analysis.reaching(reaching_state, *path);
                if !events.is_empty() {
                    let reported_path = checker.reported_path(*path, &events);
                    if reported.insert((reported_path, events.clone())) {
                        errors.push(checker.error(
                            reported_path,
                            *kind,
                            *span,
                            &events,
                            maybe_state.contains(reported_path.index()),
                            acyclic_state,
                        ));
                    }
                }
            }
            maybe.analysis.apply_effect(maybe_state, effect);
            reaching.analysis.apply_effect(reaching_state, effect);
            acyclic.analysis.apply_effect(acyclic_state, effect);
        }
    }
    errors
}

struct Checker<'a> {
    file: &'a MirFile,
    body: &'a Body,
    data: &'a MoveData,
}

impl Checker<'_> {
    /// The path an error names: the path of the first reaching event, or the used path when the
    /// event is below it (a whole-value use of a partly moved value).
    fn reported_path(&self, used: MovePathId, events: &[Event]) -> MovePathId {
        let path = self.data.event_path(events[0]);
        if self.data.ancestors(path).contains(&used) {
            used
        } else {
            path
        }
    }

    /// For a move made by building a closure that owns the value, the closure's span and the
    /// reason it owns its captures.
    fn closure_move(&self, event: Event) -> Option<(Span, Option<OwnershipReason>)> {
        let location = self.data.event_location(event);
        let statement = self.body.blocks[location.block]
            .statements
            .get(location.statement)?;
        match &statement.kind {
            StatementKind::Assign(_, rvalue) => match rvalue.as_ref() {
                Rvalue::Closure { body, .. } => Some((
                    statement.span,
                    self.file.bodies[*body].ownership_reason.clone(),
                )),
                _ => None,
            },
            _ => None,
        }
    }

    fn error(
        &self,
        path: MovePathId,
        kind: UseKind,
        span: Span,
        events: &[Event],
        possibly: bool,
        acyclic: &BitSet,
    ) -> BorrowError {
        let place = &self.data.paths[path].place;
        let name = self.name(place);
        let ty = place.ty(self.body, &self.file.types);
        let moves: Vec<Event> = events
            .iter()
            .copied()
            .filter(|event| matches!(event, Event::Move(_)))
            .collect();
        if moves.is_empty() {
            let mut related: Vec<Label> = events
                .iter()
                .map(|event| Label {
                    span: self.data.event_span(*event),
                    message: format!("the declaration of '{}' is skipped here", name),
                })
                .collect();
            sort_labels(&mut related);
            let message = if possibly {
                format!(
                    "'{}' used here, but it is not initialized on every path",
                    name
                )
            } else {
                format!("'{}' used here before it is initialized", name)
            };
            return BorrowError {
                kind: BorrowErrorKind::UseOfUninitialized {
                    name: name.clone(),
                    possibly,
                },
                primary: Label { span, message },
                related,
                notes: Vec::new(),
                helps: vec![format!("declare '{}' before the jump that skips it", name)],
            };
        }
        let mut in_previous_iteration = false;
        let mut reasons = Vec::new();
        let mut related: Vec<Label> = moves
            .iter()
            .map(|event| {
                let previous = !acyclic.contains(self.data.event_index(*event));
                in_previous_iteration |= previous;
                // A closure that owns its captures moved the value when it was built: the label
                // is at the closure, with why it owns them (D15 of `escaping-closures`).
                if let Some((closure_span, reason)) = self.closure_move(*event) {
                    reasons.extend(reason);
                    return Label {
                        span: closure_span,
                        message: if previous {
                            "value moved into closure here, in previous iteration of loop"
                                .to_string()
                        } else {
                            "value moved into closure here".to_string()
                        },
                    };
                }
                Label {
                    span: self.data.event_span(*event),
                    message: if previous {
                        "value moved here, in previous iteration of loop".to_string()
                    } else {
                        "value moved here".to_string()
                    },
                }
            })
            .collect();
        for reason in reasons {
            let label = Label {
                span: reason.span,
                message: reason.label(),
            };
            if !related.contains(&label) {
                related.push(label);
            }
        }
        sort_labels(&mut related);
        let primary = Label {
            span,
            message: match kind {
                UseKind::Used => "value used here after move",
                UseKind::Borrowed => "value borrowed here after move",
                UseKind::Captured => "value captured here after move",
                UseKind::Assigned => "value assigned here after move",
            }
            .to_string(),
        };
        let mut helps = Vec::new();
        // A value a pattern took apart can be matched through a borrow instead, which leaves it
        // whole (D16 of `payload-patterns`).
        let taken_apart = moves.iter().any(|event| {
            let location = self.data.event_location(*event);
            self.body.blocks[location.block]
                .statements
                .get(location.statement)
                .is_some_and(|statement| {
                    matches!(
                        statement.kind,
                        StatementKind::Destructure { source, .. } if source == place.local
                    )
                })
        });
        if taken_apart && place.projections.is_empty() {
            helps.push(format!(
                "to keep '{}' whole, match on its borrow instead, as in 'when (&{})' or 'val (...) = &{}', which borrows its parts",
                name, name, name
            ));
        }
        if let Some(help) = self.copy_help(&ty) {
            helps.push(help);
        }
        // A clone written at the closure would still be moved into it, so a value a closure
        // took is cloned into a `val` the closure captures instead.
        let into_closure = moves
            .iter()
            .any(|event| self.closure_move(*event).is_some());
        if into_closure {
            helps.extend(capture_clone_help(self.file, &name, &ty));
        } else {
            helps.extend(clone_help(self.file, &name, &ty));
        }
        let mutable = place.projections.is_empty() && self.body.locals[place.local].mutable;
        if in_previous_iteration && mutable {
            helps.push(format!(
                "assign '{}' a new value before the next iteration",
                name
            ));
        }
        BorrowError {
            kind: BorrowErrorKind::UseAfterMove {
                name: name.clone(),
                possibly,
            },
            primary,
            related,
            notes: vec![format!("'{}' has type '{}', which is not 'Copy'", name, ty)],
            helps,
        }
    }

    /// The help suggesting `: Copy` for a struct or enum (or a nullable of one) without the
    /// marker whose every field or variant payload is `Copy` (D8).
    fn copy_help(&self, ty: &Type) -> Option<String> {
        let types = &self.file.types;
        let id = match ty {
            Type::Nullable(inner) => match inner.as_ref() {
                Type::Struct { id, .. } | Type::Enum { id, .. } => id,
                _ => return None,
            },
            Type::Struct { id, .. } | Type::Enum { id, .. } => id,
            _ => return None,
        };
        if types.marker(id) == Some(TypeMarker::Copy) {
            return None;
        }
        let members: Vec<&Type> = match types.definition(id)? {
            TypeDef::Struct { fields, .. } => fields.iter().map(|(_, _, ty)| ty).collect(),
            TypeDef::Enum { variants, .. } => variants
                .iter()
                .flat_map(|(_, payloads)| payloads.iter())
                .collect(),
        };
        if !members.iter().all(|member| types.is_copy(member)) {
            return None;
        }
        let module = if id.module == self.file.module {
            String::new()
        } else {
            format!(" in '{}'", id.module)
        };
        Some(format!(
            "add ': Copy' to the declaration of '{}'{} to copy its values instead of moving them",
            id.name, module
        ))
    }

    /// The source name of a path's place: its local's name, then its field names.
    fn name(&self, place: &Place) -> String {
        let declaration = &self.body.locals[place.local];
        let mut name = declaration.name.clone().unwrap_or_else(|| {
            panic!(
                "internal error: an unnamed temporary _{} is used after it moved or before it is initialized in {:?}",
                place.local.0, self.body.owner
            )
        });
        let mut ty = declaration.ty.clone();
        for projection in &place.projections {
            if let (Projection::Field(index), Type::Struct { id, .. }) = (projection, &ty) {
                if let Some((field, _, _)) = self
                    .file
                    .types
                    .struct_fields(id)
                    .and_then(|fields| fields.get(*index))
                {
                    name = format!("{}.{}", name, field);
                }
            }
            ty = prism_mir::projection_type(&ty, *projection, &self.file.types)
                .expect("a move path's place is well typed");
        }
        name
    }
}

/// The help suggesting to clone the value `name` of type `ty` where it moves, for a `Clone`
/// type.
pub(crate) fn clone_help(file: &MirFile, name: &str, ty: &Type) -> Option<String> {
    if !file.types.is_clone(ty) {
        return None;
    }
    let clone = match ty {
        Type::Nullable(_) => format!("{}?.clone()", name),
        _ => format!("{}.clone()", name),
    };
    Some(format!("clone it where it moves: '{}'", clone))
}

/// The help suggesting to clone the value `name` of type `ty`, which a closure owning its
/// captures moves, into a `val` that the closure captures instead, for a `Clone` type (D15 of
/// `escaping-closures`).
pub(crate) fn capture_clone_help(file: &MirFile, name: &str, ty: &Type) -> Option<String> {
    if !file.types.is_clone(ty) {
        return None;
    }
    let fresh = if name == "m" { "k" } else { "m" };
    let clone = match ty {
        Type::Nullable(_) => format!("{}?.clone()", name),
        _ => format!("{}.clone()", name),
    };
    Some(format!(
        "clone it into a 'val' before the closure and capture that instead: 'val {} = {}'",
        fresh, clone
    ))
}

/// Whether the statement at `location` of `body` builds a closure, which moves the captures
/// it owns.
pub(crate) fn builds_closure(body: &Body, location: Location) -> bool {
    body.blocks[location.block]
        .statements
        .get(location.statement)
        .is_some_and(|statement| {
            matches!(
                &statement.kind,
                StatementKind::Assign(_, rvalue) if matches!(rvalue.as_ref(), Rvalue::Closure { .. })
            )
        })
}

/// Sorts labels in source order.
pub(crate) fn sort_labels(labels: &mut [Label]) {
    labels.sort_by_key(|label| (label.span.line, label.span.column, label.span.start));
}
