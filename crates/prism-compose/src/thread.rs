//! The per-thread state of the composition runtime (D4 of `compose-runtime`).
//!
//! Each thread has its own compositions, groups, pass counter and active pass, in a
//! `thread_local!`. Prism runs compositions on one program thread, and Rust tests run on many
//! threads, each with its own state. A thread runs at most one pass at a time, so the buffers a
//! pass works in (the composer's frames, the scratch children, the key maps, the ordering
//! buffer) belong to the thread and are reused by every pass of every composition on it.
//!
//! **The re-entrancy rule.** Every operation borrows the state through [`with_state`], does its
//! work and lets go of it before it calls program code: content and restart functions, and the
//! drop and equality entries of slot types and closures. No reference into the state is ever
//! live while program code runs, even when that code calls back into the runtime, so the pass
//! driver keeps handles and indexes, never references, across those calls. A nested borrow is
//! a bug of this crate, which the `RefCell` turns into a Rust panic rather than aliasing.
//!
//! When the thread ends, its state is dropped without running any program code: values still
//! held by undisposed compositions are leaked, as the allocation check reports (D13).

use crate::applier::{Applier, NodeHandle};
use crate::group::{Group, GroupKind};
use crate::handle::{Arena, Handle};
use crate::slot::Slot;
use prism_runtime::FunctionValue;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

/// The pass a thread is running: its identifier, which is the composer generated code passes,
/// the composition it composes, and whether it compares every kept argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivePass {
    pub identifier: u64,
    pub composition: Handle,
    /// Set for the pass after a reload: a call site's change bits were computed by code that
    /// may differ from the code that kept the arguments, so `same` and `static` arguments are
    /// compared as uncertain ones (`compose-hot-reload`).
    pub compare_kept_arguments: bool,
}

/// A composition: its root group, its applier and its invalid scopes.
pub struct CompositionRecord {
    pub root: Handle,
    pub applier: Box<dyn Applier>,
    /// The invalid scopes, in the order they were invalidated, for the next pass.
    pub queue: Vec<Handle>,
    /// Set when disposing starts: the handle is stale from then on, while the releases that
    /// need the applier run.
    pub disposed: bool,
    /// The side effects its passes queued, run after each pass (D13 of `state-handles`).
    pub effects: Vec<FunctionValue>,
    /// Whether a reload was applied since its last pass, whose next pass then compares every
    /// kept argument (`compose-hot-reload`).
    pub reloaded: bool,
    /// Whether its root content is composable content, called with the composer and the force
    /// bit, rather than word content, called with the composer alone (D4 of
    /// `compose-library-in-prism`).
    pub composable_root: bool,
}

/// What kind of frame of the composer's stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// The root group of the composition, whose body is the root content.
    Root,
    /// The parent of a scope being recomposed alone, with an old-children window of exactly
    /// the scope's group (D8).
    Window,
    /// A group started by the composer.
    Group(GroupKind),
}

/// One open group of the composer.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub group: Handle,
    pub kind: FrameKind,
    /// The window of the group's old children to match, and the next one to look at.
    pub next_old: u32,
    pub old_start: u32,
    pub old_end: u32,
    /// One past the highest old position matched so far: a match below it is out of order.
    pub high_water: u32,
    /// The key map of the old children, once a start missed the next one (D5).
    pub map: Option<u32>,
    pub map_base: u32,
    pub slot_cursor: u32,
    /// Where this group's new children start in the scratch vector.
    pub new_start: u32,
    /// The current scope: the innermost restart group open, or the root.
    pub scope: Handle,
    /// Whether this restart group existed before the pass and was not invalid.
    pub skippable: bool,
    /// The frame holding the node context this group places its nodes in, and that context's
    /// placement count when this group started.
    pub context: u32,
    pub placements_at_start: u32,
    /// The node context, in a frame that owns one (the root, a window, a node group): the node
    /// that nodes are placed into, the last node placed in this pass, how many placements
    /// happened, and whether every group so far was matched in order. While it is in order,
    /// the nodes placed so far are where they belong and every other node of the context
    /// belongs to an old group not visited yet, which will be removed or matched out of order
    /// later, so a reused node is not placed again (D9). A window resolves its node parent and
    /// previous node lazily, only when it places a node (D8).
    pub node_parent: Option<NodeHandle>,
    pub parent_known: bool,
    pub last_placed: Option<NodeHandle>,
    pub previous_known: bool,
    pub placements: u32,
    pub in_order: bool,
    /// A window's scope, and the scope's last node before its recomposition.
    pub window_scope: Handle,
    pub window_old_last: Option<NodeHandle>,
    /// A node group's old node when its kind changed, released at the group's end, after the
    /// nodes of its removed children.
    pub replaced_node: Option<(Option<NodeHandle>, NodeHandle)>,
}

/// The key of a group among its siblings: kind, key and data key.
pub type GroupKey = (GroupKind, i64, i64);

/// The map from keys to the queue of a parent's unvisited old children, in order (D5): a head
/// position per key and a chain of next positions.
#[derive(Debug, Default)]
pub struct KeyMap {
    pub heads: HashMap<GroupKey, u32>,
    pub next: Vec<u32>,
}

/// Something removed, forgotten or replaced in a pass, released at its end (D7).
#[derive(Debug)]
pub enum Release {
    Group(Handle),
    Slot(Slot),
    Function(FunctionValue),
    Node {
        composition: Handle,
        parent: Option<NodeHandle>,
        node: NodeHandle,
    },
}

/// A step of the post-order walk that releases a removed subtree.
#[derive(Debug, Clone, Copy)]
pub enum ReleaseWork {
    Enter { group: Handle, top: bool },
    Finish { group: Handle, top: bool },
}

/// One invalid scope of a pass, with its path of `index_in_parent` values in the ordering
/// buffer.
#[derive(Debug, Clone, Copy)]
pub struct OrderEntry {
    pub start: u32,
    pub length: u32,
    pub group: Handle,
}

/// What one thread's runtime holds.
pub struct State {
    /// The identifier of the last pass started, counting from 1.
    pub pass_counter: u64,
    /// The pass running now, if any.
    pub active: Option<ActivePass>,
    /// Compositions started and not disposed.
    pub live_compositions: usize,
    pub compositions: Arena<CompositionRecord>,
    /// The groups of every composition of the thread, so a scope handle needs no composition.
    pub groups: Arena<Group>,
    pub frames: Vec<Frame>,
    pub scratch: Vec<Handle>,
    pub maps: Vec<KeyMap>,
    pub maps_used: usize,
    pub releases: VecDeque<Release>,
    pub release_work: Vec<ReleaseWork>,
    pub order: Vec<OrderEntry>,
    pub order_paths: Vec<u32>,
    /// A composition's queue swapped out while a pass orders it, so neither reallocates.
    pub spare_queue: Vec<Handle>,
    /// Scopes invalidated during the pass, for the next one.
    pub deferred: Vec<Handle>,
    /// The walk of a subtree whose nodes are placed again.
    pub walk: Vec<Handle>,
    /// The state cells of the thread (D8 of `state-handles`).
    pub cells: Arena<crate::state::Cell>,
    /// The creation windows open now (D15 of `owned-state`): `remember` calculations and
    /// top-level initializers, which may create cells during a pass.
    pub creation_windows: u32,
    /// The retired cells not freed yet, with the pass counter when each was retired, in order
    /// (D7 of `owned-state`).
    pub retired: VecDeque<(Handle, u64)>,
    /// The pass whose releases run now, 0 outside them.
    pub releasing_pass: u64,
    /// The error the last failed state operation recorded, for its location (D14).
    pub state_error: Option<crate::state::StateError>,
    /// A spare effect queue, swapped with a composition's while its effects run.
    pub spare_effects: Vec<FunctionValue>,
    /// The body keys and retired slots of the reloads applied during the current pass, handled
    /// when it ends (D3 of `compose-hot-reload`).
    pub queued_reloads: Vec<(Vec<u64>, Vec<usize>)>,
    /// The slots retired by the reloads applied during the current pass: a restart group whose
    /// restart function names one forgets its slots when it starts in that pass (D8 of
    /// `compose-hot-reload`). Cleared when the pass ends; a reload retires few slots.
    pub retired_in_pass: Vec<usize>,
    /// The walk of a reload's invalidation and the scopes it found, reused by every reload.
    pub reload_walk: Vec<(Handle, Handle)>,
    pub reload_targets: Vec<Handle>,
    /// The scopes reloads invalidated since `--timings` last asked.
    pub invalidated_by_reloads: usize,
    /// The next version a state cell is given (D3 of `strong-skipping`): one counter per
    /// thread, from 2, so a version names one cell between two writes and is never 0 (no
    /// identity) or 1 (permanent).
    pub next_version: u64,
    /// The recomposition counts of the bodies counted, while counting is on (D14 of
    /// `strong-skipping`).
    pub counting: Option<Box<HashMap<i64, crate::counts::Counts>>>,
}

impl State {
    /// A version no cell of the thread had before (D3 of `strong-skipping`).
    pub fn fresh_version(&mut self) -> u64 {
        let version = self.next_version;
        self.next_version += 1;
        version
    }

    const fn new() -> State {
        State {
            pass_counter: 0,
            active: None,
            live_compositions: 0,
            compositions: Arena::new(),
            groups: Arena::new(),
            frames: Vec::new(),
            scratch: Vec::new(),
            maps: Vec::new(),
            maps_used: 0,
            releases: VecDeque::new(),
            release_work: Vec::new(),
            order: Vec::new(),
            order_paths: Vec::new(),
            spare_queue: Vec::new(),
            deferred: Vec::new(),
            walk: Vec::new(),
            cells: Arena::new(),
            creation_windows: 0,
            retired: VecDeque::new(),
            releasing_pass: 0,
            state_error: None,
            spare_effects: Vec::new(),
            queued_reloads: Vec::new(),
            retired_in_pass: Vec::new(),
            reload_walk: Vec::new(),
            reload_targets: Vec::new(),
            invalidated_by_reloads: 0,
            next_version: 2,
            counting: None,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State::new()) };
}

/// Runs `f` with this thread's state. `f` must not call program code (the re-entrancy rule).
pub fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|state| f(&mut state.borrow_mut()))
}

/// The line the allocation check writes when compositions of this thread were started and
/// never disposed (D13): `allocation check: compositions still live at exit: <n>`. Installed
/// with `prism_runtime::set_finish_report`, which calls it on the program's thread.
pub fn live_compositions_report() -> Option<String> {
    let live = with_state(|state| state.live_compositions);
    (live > 0).then(|| {
        format!(
            "allocation check: compositions still live at exit: {}",
            live
        )
    })
}

/// The lines the allocation check writes for this thread's groups, slots and state cells still
/// live when `main` returns, after the composition report (D6 of `reactive-app-verification`):
/// `allocation check: groups still live at exit: <n>`, then `slots` and `state cells` in the
/// same form, one line per non-zero count, in this order. `None` when a composition is still
/// live, since what it holds is expected to be live then, or when nothing is. Groups and cells
/// are their arenas' live entries; slots are those of the live groups and those waiting in the
/// release queue. It walks the groups once, at exit, so counting costs nothing before.
pub fn live_objects_report() -> Option<String> {
    let (groups, slots, cells) = with_state(|state| {
        if state.live_compositions > 0 {
            return (0, 0, 0);
        }
        let held: usize = state
            .groups
            .iter()
            .map(|(_, group)| group.slots.len())
            .sum();
        let pending = state
            .releases
            .iter()
            .filter(|release| matches!(release, Release::Slot(_)))
            .count();
        (state.groups.len(), held + pending, state.cells.len())
    });
    let lines: Vec<String> = [("groups", groups), ("slots", slots), ("state cells", cells)]
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(name, count)| format!("allocation check: {} still live at exit: {}", name, count))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The current scope of the pass running on this thread, if one is: the innermost restart
/// group open, or the composition's root (compose, "The Current Scope"). For `state-handles`,
/// which registers it as a reader.
pub fn active_scope() -> Option<Handle> {
    with_state(|state| {
        state.active?;
        state.frames.last().map(|frame| frame.scope)
    })
}

/// The composition of the pass running on this thread, if one is: what `prism-ui` checks its
/// setters against, so that toolkit composables run only in the running app's composition (D10
/// of `ui-toolkit`).
pub fn active_composition() -> Option<Handle> {
    with_state(|state| state.active.map(|pass| pass.composition))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_thread_has_its_own_state() {
        with_state(|state| state.live_compositions = 2);
        assert_eq!(
            live_compositions_report().as_deref(),
            Some("allocation check: compositions still live at exit: 2")
        );
        let other = std::thread::spawn(|| {
            assert_eq!(live_compositions_report(), None);
            with_state(|state| {
                state.pass_counter += 1;
                state.pass_counter
            })
        })
        .join()
        .unwrap();
        assert_eq!(other, 1);
        with_state(|state| {
            assert_eq!(state.pass_counter, 0, "the other thread's pass is its own");
            state.live_compositions = 0;
        });
        assert_eq!(live_compositions_report(), None);
        assert_eq!(active_scope(), None);
    }

    /// execution: "A leaked group, cell and node are reported", for this crate's part: each
    /// leak gives its line, in the fixed order, and a live composition hides them.
    #[test]
    fn leaked_groups_slots_and_cells_are_reported_in_order() {
        std::thread::spawn(|| {
            assert_eq!(live_objects_report(), None);
            crate::test_support::leak_cell();
            assert_eq!(
                live_objects_report().as_deref(),
                Some("allocation check: state cells still live at exit: 1")
            );
            crate::test_support::leak_slot();
            crate::test_support::leak_group();
            crate::test_support::leak_group();
            assert_eq!(
                live_objects_report().as_deref(),
                Some(
                    "allocation check: groups still live at exit: 2\n\
                     allocation check: slots still live at exit: 1\n\
                     allocation check: state cells still live at exit: 1"
                )
            );
            with_state(|state| state.live_compositions = 1);
            assert_eq!(live_objects_report(), None, "a live composition hides them");
            with_state(|state| state.live_compositions = 0);
        })
        .join()
        .unwrap();
    }
}
