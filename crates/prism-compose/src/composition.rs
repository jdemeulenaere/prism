//! Compositions and their passes (D4, D7, D8 and D11 of `compose-runtime`).
//!
//! A composition owns a root restart group, whose restart function is its content, an applier
//! and a queue of invalid scopes. A pass recomposes the invalid scopes in tree order as of its
//! start, parents first, each at most once; then it ends, and only then releases what it
//! removed, forgot or replaced, so every address and value handed out during the pass stays
//! valid until it ends (D7). Releases run program code (drop entries), each called with no
//! borrow of the runtime's state held, so a drop entry may dispose another composition.

use crate::applier::Applier;
use crate::composer::{
    finish_frame, invalidate_in, ComposeResult, Composer, DURING_PASS, LEFT_OPEN, STALE_COMPOSITION,
};
use crate::group::{Group, GroupKind};
use crate::handle::Handle;
use crate::slot::Slot;
use crate::state::{CreationOwner, States};
use crate::thread::{
    with_state, ActivePass, CompositionRecord, Frame, FrameKind, OrderEntry, Release, ReleaseWork,
    State,
};
use prism_runtime::FunctionValue;
use std::sync::atomic::AtomicUsize;

/// The lifecycle operations of compositions, which are named by their [`Handle`]s.
pub struct Composition;

impl Composition {
    /// Creates a composition that owns `content` and `applier`, and runs its first pass,
    /// which composes the content as the body of the root restart group.
    pub fn start(applier: Box<dyn Applier>, content: FunctionValue) -> ComposeResult<Handle> {
        let handle = with_state(|state| {
            if state.active.is_some() {
                return Err(DURING_PASS);
            }
            let handle = state.compositions.insert(CompositionRecord {
                root: Handle(0),
                applier,
                queue: Vec::new(),
                disposed: false,
                cells: Vec::new(),
                effects: Vec::new(),
            });
            let mut root = Group::new(GroupKind::Restart, 0, 0, handle);
            root.restart = Some(content);
            root.invalid = true;
            let root = state.groups.insert(root);
            let record = state.compositions.get_mut(handle).unwrap();
            record.root = root;
            record.queue.push(root);
            state.live_compositions += 1;
            Ok(handle)
        })?;
        run_pass(handle, false)?;
        Ok(handle)
    }

    /// Replaces the composition's content, invalidates its root scope and runs a pass. The old
    /// content is released at the end of that pass.
    pub fn set_content(composition: Handle, content: FunctionValue) -> ComposeResult<()> {
        with_state(|state| {
            let record = live_record(state, composition)?;
            let root = record.root;
            let old = state.groups.get_mut(root).unwrap().restart.replace(content);
            if let Some(old) = old {
                state.releases.push_back(Release::Function(old));
            }
            invalidate_in(state, root);
            Ok(())
        })?;
        run_pass(composition, false).map(|_| ())
    }

    /// Runs a pass when a scope of the composition is invalid, giving `true`; otherwise runs
    /// nothing and gives `false`.
    pub fn recompose(composition: Handle) -> ComposeResult<bool> {
        run_pass(composition, false)
    }

    /// Runs a pass as [`Composition::recompose`] does, in which no restart group skips:
    /// `compose-hot-reload`'s forced pass (D15).
    pub fn recompose_forced(composition: Handle) -> ComposeResult<bool> {
        run_pass(composition, true)
    }

    /// Removes every group of the composition and releases them as the end of a pass does,
    /// then releases its root content and its applier, and makes its handle stale.
    pub fn dispose(composition: Handle) -> ComposeResult<()> {
        with_state(|state| {
            let record = live_record(state, composition)?;
            record.disposed = true;
            let root = record.root;
            state.groups.get_mut(root).unwrap().removed = true;
            state.releases.push_back(Release::Group(root));
            state.live_compositions -= 1;
            Ok(())
        })?;
        process_releases();
        // Then the cells its handlers and side effects created, and the effects no pass ran
        // (D12 and D13 of `state-handles`).
        let (cells, effects) = with_state(|state| {
            let record = state.compositions.get_mut(composition).unwrap();
            (
                std::mem::take(&mut record.cells),
                std::mem::take(&mut record.effects),
            )
        });
        States::release(&cells);
        for effect in effects {
            // SAFETY: a queued effect is a live function value the composition owns.
            unsafe { effect.release() };
        }
        // The applier is dropped outside the state.
        let record = with_state(|state| state.compositions.remove(composition));
        drop(record);
        Ok(())
    }

    /// Runs `f` as a handler of `composition` (D12 of `state-handles`): the state cells it
    /// creates belong to the composition and are freed when it is disposed. What `ui-toolkit`
    /// dispatches events through, and how side effects run.
    pub fn run_handler<R>(composition: Handle, f: impl FnOnce() -> R) -> R {
        States::push_owner(CreationOwner::Composition(composition));
        let result = f();
        States::pop_owner();
        result
    }

    /// The root scope of the composition.
    pub fn root_scope(composition: Handle) -> ComposeResult<Handle> {
        with_state(|state| {
            let record = state.compositions.get(composition);
            match record {
                Some(record) if !record.disposed => Ok(record.root),
                _ => Err(STALE_COMPOSITION),
            }
        })
    }

    /// Invalidates every restart scope of the composition whose restart function's descriptor
    /// names a code slot that `predicate` accepts, and gives how many: what `compose-hot-reload`
    /// calls with the slots of reloaded units (D15). `predicate` runs outside the runtime's
    /// state.
    pub fn invalidate_where(
        composition: Handle,
        predicate: impl Fn(*const AtomicUsize) -> bool,
    ) -> ComposeResult<usize> {
        let scopes = with_state(
            |state| -> ComposeResult<Vec<(Handle, *const AtomicUsize)>> {
                let root = live_record(state, composition)?.root;
                let mut scopes = Vec::new();
                let mut walk = vec![root];
                while let Some(group) = walk.pop() {
                    let record = state.groups.get(group).unwrap();
                    if let Some(restart) = &record.restart {
                        // SAFETY: a kept restart function is a live function value.
                        scopes.push((group, unsafe { restart.slot() }));
                    }
                    walk.extend_from_slice(&record.children);
                }
                Ok(scopes)
            },
        )?;
        let mut count = 0;
        for (scope, slot) in scopes {
            if predicate(slot) {
                with_state(|state| invalidate_in(state, scope));
                count += 1;
            }
        }
        Ok(count)
    }

    /// Runs `f` with the composition's applier, outside any pass.
    pub fn with_applier<R>(
        composition: Handle,
        f: impl FnOnce(&mut dyn Applier) -> R,
    ) -> ComposeResult<R> {
        with_state(|state| {
            let record = live_record(state, composition)?;
            Ok(f(record.applier.as_mut()))
        })
    }
}

/// The record of a live composition, outside any pass.
fn live_record(state: &mut State, composition: Handle) -> ComposeResult<&mut CompositionRecord> {
    if state.active.is_some() {
        return Err(DURING_PASS);
    }
    match state.compositions.get_mut(composition) {
        Some(record) if !record.disposed => Ok(record),
        _ => Err(STALE_COMPOSITION),
    }
}

/// A bitwise copy of a function value kept by the runtime, to call it outside the state; the
/// runtime keeps owning it.
fn borrowed(function: &FunctionValue) -> FunctionValue {
    FunctionValue {
        descriptor: function.descriptor,
        environment: function.environment,
    }
}

/// One pass over the composition's invalid scopes, or none when there is none.
fn run_pass(composition: Handle, forced: bool) -> ComposeResult<bool> {
    let identifier = with_state(|state| -> ComposeResult<Option<u64>> {
        live_record(state, composition)?;
        if !order_scopes(state, composition) {
            return Ok(None);
        }
        state.pass_counter += 1;
        state.active = Some(ActivePass {
            identifier: state.pass_counter,
            composition,
            forced,
        });
        Ok(Some(state.pass_counter))
    })?;
    let Some(identifier) = identifier else {
        return Ok(false);
    };
    let count = with_state(|state| state.order.len());
    for index in 0..count {
        let Some(function) = with_state(|state| open_scope(state, index)) else {
            continue;
        };
        // SAFETY: the runtime keeps the function value alive until the end of the pass at
        // least, since a replaced one is released only then.
        unsafe { function.call_with_word(identifier) };
        with_state(|state| {
            if state.frames.len() != 1 {
                abort_pass(state);
                return Err(LEFT_OPEN);
            }
            finish_frame(state);
            Ok(())
        })?;
    }
    with_state(end_pass);
    process_releases();
    run_effects(composition);
    Ok(true)
}

/// Runs the side effects the pass queued, in order, each as a handler of the composition and
/// then released (D13 of `state-handles`). The queue is swapped with a spare one, so a pass
/// that queues effects reuses both vectors.
fn run_effects(composition: Handle) {
    let effects = with_state(|state| {
        let record = state.compositions.get_mut(composition)?;
        if record.effects.is_empty() {
            return None;
        }
        let spare = std::mem::take(&mut state.spare_effects);
        Some(std::mem::replace(&mut record.effects, spare))
    });
    let Some(mut effects) = effects else {
        return;
    };
    for effect in effects.drain(..) {
        // SAFETY: a queued effect is a live `() -> Unit` function value the queue owns,
        // released once after it runs.
        Composition::run_handler(composition, || unsafe { effect.call() });
        unsafe { effect.release() };
    }
    with_state(|state| {
        if state.spare_effects.capacity() < effects.capacity() {
            state.spare_effects = effects;
        }
    });
}

/// Orders the composition's invalid scopes in tree order into the pass's buffer: each scope's
/// path of `index_in_parent` values from the root, compared lexicographically, so parents
/// sort before their descendants. O(k log k · d). Gives whether there is any.
fn order_scopes(state: &mut State, composition: Handle) -> bool {
    let record = state.compositions.get_mut(composition).unwrap();
    std::mem::swap(&mut record.queue, &mut state.spare_queue);
    state.order.clear();
    state.order_paths.clear();
    for index in 0..state.spare_queue.len() {
        let scope = state.spare_queue[index];
        match state.groups.get(scope) {
            Some(group) if group.invalid && !group.removed => {}
            _ => continue,
        }
        let start = state.order_paths.len();
        let mut group = scope;
        while let Some(record) = state.groups.get(group) {
            let Some(parent) = record.parent else {
                break;
            };
            state.order_paths.push(record.index_in_parent);
            group = parent;
        }
        state.order_paths[start..].reverse();
        state.order.push(OrderEntry {
            start: start as u32,
            length: (state.order_paths.len() - start) as u32,
            group: scope,
        });
    }
    state.spare_queue.clear();
    let paths = &state.order_paths;
    let path =
        |entry: &OrderEntry| &paths[entry.start as usize..(entry.start + entry.length) as usize];
    state.order.sort_unstable_by(|a, b| path(a).cmp(path(b)));
    !state.order.is_empty()
}

/// Opens the frame that recomposes the pass's `index`-th scope, unless its group was removed
/// or already composed in this pass, and gives the function to call: the root content for
/// the root, or the scope's restart function, with a window of exactly the scope's group in
/// its parent (D8).
fn open_scope(state: &mut State, index: usize) -> Option<FunctionValue> {
    let pass = state.active.unwrap();
    let scope = state.order[index].group;
    if !state.groups.get(scope)?.invalid || inside_removed_group(state, scope) {
        return None;
    }
    let group = state.groups.get_mut(scope)?;
    let function = borrowed(group.restart.as_ref()?);
    let mut frame = Frame {
        group: scope,
        kind: FrameKind::Root,
        next_old: 0,
        old_start: 0,
        old_end: group.children.len() as u32,
        high_water: 0,
        map: None,
        map_base: 0,
        slot_cursor: 0,
        new_start: state.scratch.len() as u32,
        scope,
        skippable: false,
        context: state.frames.len() as u32,
        placements_at_start: 0,
        node_parent: None,
        parent_known: true,
        last_placed: None,
        previous_known: true,
        placements: 0,
        in_order: true,
        window_scope: scope,
        window_old_last: None,
        replaced_node: None,
    };
    match group.parent {
        None => {
            group.invalid = false;
            group.visited = pass.identifier;
            let applier = &state.compositions.get(pass.composition).unwrap().applier;
            frame.node_parent = Some(applier.root());
        }
        Some(parent) => {
            // A parent that skipped in this pass kept the scope's group and marked it visited
            // without composing it. The window must still match it, or a different first start
            // removes it, so it starts this frame unvisited.
            group.visited = 0;
            let position = group.index_in_parent;
            frame.window_old_last = group.last_node;
            frame.kind = FrameKind::Window;
            frame.group = parent;
            frame.next_old = position;
            frame.high_water = position;
            frame.old_start = position;
            frame.old_end = position + 1;
            frame.parent_known = false;
            frame.previous_known = false;
            // The current scope before the restart function starts its group: the nearest
            // restart group enclosing the scope.
            let mut enclosing = parent;
            while let Some(record) = state.groups.get(enclosing) {
                match record.parent {
                    Some(above) if record.kind != GroupKind::Restart => enclosing = above,
                    _ => break,
                }
            }
            frame.scope = enclosing;
        }
    }
    state.frames.push(frame);
    Some(function)
}

/// Whether `group` or one of its ancestors was removed in this pass. Only the top group of a
/// removed subtree is marked until the subtree is released, so a scope below it is found by
/// walking up: O(depth), as computing the scope's path for the ordering is.
fn inside_removed_group(state: &State, group: Handle) -> bool {
    let mut current = Some(group);
    while let Some(handle) = current {
        let Some(record) = state.groups.get(handle) else {
            return true;
        };
        if record.removed {
            return true;
        }
        current = record.parent;
    }
    false
}

/// Ends the active pass: the scopes invalidated during it go to their compositions' queues.
fn end_pass(state: &mut State) {
    state.active = None;
    for index in 0..state.deferred.len() {
        let scope = state.deferred[index];
        if let Some(group) = state.groups.get_mut(scope) {
            group.pending = false;
        }
        invalidate_in(state, scope);
    }
    state.deferred.clear();
}

/// Ends a pass that met a misuse its driver reports: the open frames are dropped and the pass
/// ends. The composition stays memory-safe; what the pass changed is kept as it is.
fn abort_pass(state: &mut State) {
    state.frames.clear();
    state.scratch.clear();
    state.maps_used = 0;
    end_pass(state);
}

/// What releasing one thing asks of the program.
enum ReleaseStep {
    Done,
    Continue,
    Slot(Slot),
    Function(FunctionValue),
}

/// Releases everything waiting for release, one step at a time, calling each drop entry with
/// no borrow of the state held.
pub(crate) fn process_releases() {
    loop {
        match with_state(next_release) {
            ReleaseStep::Done => break,
            ReleaseStep::Continue => {}
            // SAFETY: a released slot or function value was owned by the runtime and is
            // released once.
            ReleaseStep::Slot(slot) => unsafe {
                if let Some((drop, value)) = slot.drop_entry() {
                    drop(value);
                }
                slot.free_block();
                // A slot's state cells are released after its value (D12 of `state-handles`).
                if !slot.cells().is_empty() {
                    States::release(slot.cells());
                }
            },
            ReleaseStep::Function(function) => unsafe { function.release() },
        }
    }
}

/// The next step of the releases: the walk of a removed subtree first (children last first,
/// then the group's slots last first, its restart function and its node), then the release
/// list in order.
fn next_release(state: &mut State) -> ReleaseStep {
    if let Some(work) = state.release_work.pop() {
        match work {
            ReleaseWork::Enter { group, top } => {
                let Some(record) = state.groups.get_mut(group) else {
                    return ReleaseStep::Continue;
                };
                let below = top && record.kind != GroupKind::Node;
                let children = std::mem::take(&mut record.children);
                state.release_work.push(ReleaseWork::Finish { group, top });
                for child in children {
                    state.release_work.push(ReleaseWork::Enter {
                        group: child,
                        top: below,
                    });
                }
                return ReleaseStep::Continue;
            }
            ReleaseWork::Finish { group, top } => {
                let Some(record) = state.groups.get_mut(group) else {
                    return ReleaseStep::Continue;
                };
                if let Some(slot) = record.slots.pop() {
                    state.release_work.push(work);
                    return ReleaseStep::Slot(slot);
                }
                if let Some(function) = record.restart.take() {
                    state.release_work.push(work);
                    return ReleaseStep::Function(function);
                }
                let record = state.groups.remove(group).unwrap();
                if let Some(node) = record.node {
                    if let Some(composition) = state.compositions.get_mut(record.composition) {
                        if let (true, Some(parent)) = (top, record.parent_node) {
                            composition.applier.remove(parent, node);
                        }
                        composition.applier.release(node);
                    }
                }
                return ReleaseStep::Continue;
            }
        }
    }
    match state.releases.pop_front() {
        None => ReleaseStep::Done,
        Some(Release::Group(group)) => {
            state
                .release_work
                .push(ReleaseWork::Enter { group, top: true });
            ReleaseStep::Continue
        }
        Some(Release::Slot(slot)) => ReleaseStep::Slot(slot),
        Some(Release::Function(function)) => ReleaseStep::Function(function),
        Some(Release::Node {
            composition,
            parent,
            node,
        }) => {
            if let Some(record) = state.compositions.get_mut(composition) {
                if let Some(parent) = parent {
                    record.applier.remove(parent, node);
                }
                record.applier.release(node);
            }
            ReleaseStep::Continue
        }
    }
}

/// The composer of the pass running now, for tests that drive the protocol from Rust.
pub fn active_composer() -> Option<Composer> {
    with_state(|state| state.active.map(|pass| Composer(pass.identifier)))
}
