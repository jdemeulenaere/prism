//! The initialization analyses of one body (D4 of `borrowck-moves`), both forward over
//! [`MoveData`]:
//!
//! - [`MaybeInitialized`] (bits: move paths): whether a path may hold a value, because some path
//!   from the entry initializes it without moving it since.
//! - [`ReachingUninitializations`] (bits: events): the moves and storage starts that reach a
//!   point without an initialization of their path since. A path is possibly uninitialized
//!   exactly where an event of it, an ancestor or a descendant reaches.
//!
//! A path is definitely initialized where the first has it and the second has none of its
//! events. Effects within one location apply in evaluation order, so that the use check can
//! replay them one by one.

use crate::dataflow::{Analysis, BitSet};
use crate::location::Location;
use crate::move_paths::{Effect, Event, MoveData, MovePathId};
use prism_mir::{Statement, Terminator};
use prism_typed_tree::ArenaId;

/// The paths that may hold a value.
pub struct MaybeInitialized<'a> {
    pub data: &'a MoveData,
}

impl MaybeInitialized<'_> {
    /// Applies one effect to `state`.
    pub fn apply_effect(&self, state: &mut BitSet, effect: &Effect) {
        let data = self.data;
        match effect {
            Effect::Use { .. } => {}
            Effect::Init(init) => {
                for path in data.descendants(data.inits[*init].path) {
                    state.insert(path.index());
                }
            }
            Effect::Move(out) => {
                for path in data.descendants(data.moves[*out].path) {
                    state.remove(path.index());
                }
            }
            Effect::StorageLive(start) => {
                let root = data.root(data.storage_starts[*start].local);
                for path in data.descendants(root) {
                    state.remove(path.index());
                }
            }
            Effect::StorageDead(local) => {
                for path in data.descendants(data.root(*local)) {
                    state.remove(path.index());
                }
            }
        }
    }

    fn apply_location(&self, state: &mut BitSet, location: Location) {
        for effect in self.data.effects(location) {
            self.apply_effect(state, effect);
        }
    }
}

impl Analysis for MaybeInitialized<'_> {
    fn domain_size(&self) -> usize {
        self.data.paths.len()
    }

    fn initialize_boundary(&self, state: &mut BitSet) {
        for init in self.data.entry_inits() {
            for path in self.data.descendants(init.path) {
                state.insert(path.index());
            }
        }
    }

    fn apply_statement(&self, state: &mut BitSet, _: &Statement, location: Location) {
        self.apply_location(state, location);
    }

    fn apply_terminator(&self, state: &mut BitSet, _: &Terminator, location: Location) {
        self.apply_location(state, location);
    }
}

/// The moves and storage starts that reach a point without a later initialization of their
/// path.
pub struct ReachingUninitializations<'a> {
    pub data: &'a MoveData,
}

impl ReachingUninitializations<'_> {
    /// Removes the events of `path` and of every path below it.
    fn kill_tree(&self, state: &mut BitSet, path: MovePathId) {
        for path in self.data.descendants(path) {
            for event in self.data.events_of(path) {
                state.remove(self.data.event_index(*event));
            }
        }
    }

    /// Applies one effect to `state`.
    pub fn apply_effect(&self, state: &mut BitSet, effect: &Effect) {
        let data = self.data;
        match effect {
            Effect::Use { .. } => {}
            Effect::Move(out) => {
                state.insert(data.event_index(Event::Move(*out)));
            }
            Effect::Init(init) => self.kill_tree(state, data.inits[*init].path),
            Effect::StorageLive(start) => {
                self.kill_tree(state, data.root(data.storage_starts[*start].local));
                state.insert(data.event_index(Event::StorageStart(*start)));
            }
            Effect::StorageDead(local) => self.kill_tree(state, data.root(*local)),
        }
    }

    fn apply_location(&self, state: &mut BitSet, location: Location) {
        for effect in self.data.effects(location) {
            self.apply_effect(state, effect);
        }
    }

    /// The events in `state` of `path`, its ancestors and its descendants, in event order.
    pub fn reaching(&self, state: &BitSet, path: MovePathId) -> Vec<Event> {
        let data = self.data;
        let mut family = data.ancestors(path);
        family.extend(data.descendants(path));
        let mut events: Vec<Event> = family
            .into_iter()
            .flat_map(|path| data.events_of(path).iter().copied())
            .filter(|event| state.contains(data.event_index(*event)))
            .collect();
        events.sort();
        events.dedup();
        events
    }
}

impl Analysis for ReachingUninitializations<'_> {
    fn domain_size(&self) -> usize {
        self.data.event_count()
    }

    fn initialize_boundary(&self, _: &mut BitSet) {}

    fn apply_statement(&self, state: &mut BitSet, _: &Statement, location: Location) {
        self.apply_location(state, location);
    }

    fn apply_terminator(&self, state: &mut BitSet, _: &Terminator, location: Location) {
        self.apply_location(state, location);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataflow::solve;
    use crate::location::locations;
    use crate::test_bodies::{body, lower};
    use prism_mir::{Body, Callee, LocalId, TerminatorKind};

    /// The location of the call of `callee` in `body`.
    fn call_of(body: &Body, callee: &str) -> Location {
        locations(body)
            .into_iter()
            .find(|location| {
                location.is_terminator(body)
                    && matches!(
                        &body.blocks[location.block].terminator.kind,
                        TerminatorKind::Call { callee: Callee::Function(path), .. }
                            if path.to_string() == format!("main::{}", callee)
                    )
            })
            .unwrap_or_else(|| panic!("no call of {}", callee))
    }

    fn local_named(body: &Body, name: &str) -> LocalId {
        body.locals
            .iter()
            .find(|(_, l)| l.name.as_deref() == Some(name))
            .map(|(id, _)| id)
            .unwrap()
    }

    /// Whether `name` may be initialized before the call of `callee`, and the events of `name`
    /// that reach the call, in function `f` of `source`.
    fn state_before_call(source: &str, name: &str, callee: &str) -> (bool, Vec<Event>) {
        let file = lower(source);
        let body = body(&file, "f");
        let data = MoveData::gather(body, &file.types);
        let location = call_of(body, callee);
        let path = data.root(local_named(body, name));
        let maybe = solve(body, MaybeInitialized { data: &data });
        let reaching = solve(body, ReachingUninitializations { data: &data });
        let initialized = maybe.cursor(body).seek(location).contains(path.index());
        let events = reaching
            .analysis
            .reaching(reaching.cursor(body).seek(location), path);
        (initialized, events)
    }

    const DECLARATIONS: &str = "fun take(s: String) {}\nfun g() {}\n";

    #[test]
    fn a_conditional_move_leaves_the_value_possibly_initialized() {
        let source = format!(
            "{}fun f(c: Bool, s: String) {{ if (c) take(s); g() }}\n",
            DECLARATIONS
        );
        let (initialized, events) = state_before_call(&source, "s", "g");
        assert!(initialized);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::Move(_)));
    }

    #[test]
    fn an_assignment_reinitializes_a_moved_value() {
        let source = format!(
            "{}fun f(a: String, b: String) {{ var s = a; take(s); s = b; g() }}\n",
            DECLARATIONS
        );
        let (initialized, events) = state_before_call(&source, "s", "g");
        assert!(initialized);
        assert_eq!(events, vec![]);
        let source = format!(
            "{}fun f(a: String) {{ var s = a; take(s); g() }}\n",
            DECLARATIONS
        );
        let (initialized, events) = state_before_call(&source, "s", "g");
        assert!(!initialized);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn a_move_in_a_loop_reaches_the_loop_head() {
        let source = format!(
            "{}fun f(c: Bool, s: String) {{ while (c) take(s) }}\n",
            DECLARATIONS
        );
        let file = lower(&source);
        let body = body(&file, "f");
        let data = MoveData::gather(body, &file.types);
        let reaching = solve(body, ReachingUninitializations { data: &data });
        let moved = data
            .moves
            .iter()
            .find(|(_, out)| out.path == data.root(LocalId(2)))
            .map(|(id, _)| id)
            .unwrap();
        // The move reaches its own location around the loop.
        let location = data.moves[moved].location;
        let state = reaching.cursor(body).seek(location).clone();
        assert!(state.contains(data.event_index(Event::Move(moved))));
    }

    #[test]
    fn a_continue_reaches_the_condition_with_a_declaration_skipped() {
        let source = "\
fun next(): Int = 0
fun f(c: Bool) { do { val s = \"a\"; if (c) continue; val x = next() } while (x > 0) }
";
        let file = lower(source);
        let body = body(&file, "f");
        let data = MoveData::gather(body, &file.types);
        let x = local_named(body, "x");
        let path = data.root(x);
        // The condition's read of `x`.
        let read = locations(body)
            .into_iter()
            .find(|location| {
                data.effects(*location)
                    .iter()
                    .any(|effect| matches!(effect, Effect::Use { path: used, .. } if *used == path))
            })
            .unwrap();
        let maybe = solve(body, MaybeInitialized { data: &data });
        let reaching = solve(body, ReachingUninitializations { data: &data });
        assert!(maybe.cursor(body).seek(read).contains(path.index()));
        let events = reaching
            .analysis
            .reaching(reaching.cursor(body).seek(read), path);
        let starts: Vec<_> = data
            .storage_starts
            .iter()
            .filter(|(_, start)| start.local == x)
            .map(|(id, _)| Event::StorageStart(id))
            .collect();
        assert_eq!(starts.len(), 2, "x is marked live on both paths");
        // Only the `continue` path's storage start reaches, unassigned.
        assert_eq!(events.len(), 1);
        assert!(starts.contains(&events[0]));
        assert_eq!(data.event_span(events[0]).column, 43);
    }
}
