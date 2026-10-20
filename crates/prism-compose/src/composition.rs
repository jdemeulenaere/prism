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
use crate::state::States;
use crate::thread::{
    with_state, ActivePass, CompositionRecord, Frame, FrameKind, OrderEntry, Release, ReleaseWork,
    State,
};
use prism_runtime::FunctionValue;

/// The force bit, bit 0 of a composable's first change word: the root content's body runs
/// whatever its arguments (D4 of `compose-library-in-prism`).
const FORCE: u64 = 1;

/// The root content of a composition: word content, `(Int) -> Unit`, which the runtime calls
/// with the composer (the protocol module's `startComposition`), or composable content,
/// `compose () -> Unit`, which it calls with the composer and the force bit, as Compose's
/// `invokeComposable` does (D4 of `compose-library-in-prism`).
#[derive(Debug)]
pub enum RootContent {
    Word(FunctionValue),
    Composable(FunctionValue),
}

impl RootContent {
    /// Whether it is composable content.
    pub fn is_composable(&self) -> bool {
        matches!(self, RootContent::Composable(_))
    }

    /// The function value.
    pub fn into_function(self) -> FunctionValue {
        match self {
            RootContent::Word(function) | RootContent::Composable(function) => function,
        }
    }
}

impl From<FunctionValue> for RootContent {
    /// Word content.
    fn from(function: FunctionValue) -> Self {
        RootContent::Word(function)
    }
}

/// The lifecycle operations of compositions, which are named by their [`Handle`]s.
pub struct Composition;

impl Composition {
    /// Creates a composition that owns `content` and `applier`, and runs its first pass,
    /// which composes the content as the body of the root restart group.
    pub fn start(
        applier: Box<dyn Applier>,
        content: impl Into<RootContent>,
    ) -> ComposeResult<Handle> {
        let content = content.into();
        let composable_root = content.is_composable();
        let content = content.into_function();
        crate::state::mark_in_use();
        let handle = with_state(|state| {
            if state.active.is_some() {
                return Err(DURING_PASS);
            }
            let handle = state.compositions.insert(CompositionRecord {
                root: Handle(0),
                applier,
                queue: Vec::new(),
                disposed: false,
                effects: Vec::new(),
                reloaded: false,
                composable_root,
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
        run_pass(handle)?;
        Ok(handle)
    }

    /// Replaces the composition's content, invalidates its root scope and runs a pass. The old
    /// content is released at the end of that pass.
    pub fn set_content(composition: Handle, content: impl Into<RootContent>) -> ComposeResult<()> {
        let content = content.into();
        let composable_root = content.is_composable();
        let content = content.into_function();
        with_state(|state| {
            let record = live_record(state, composition)?;
            record.composable_root = composable_root;
            let root = record.root;
            let old = state.groups.get_mut(root).unwrap().restart.replace(content);
            if let Some(old) = old {
                state.releases.push_back(Release::Function(old));
            }
            invalidate_in(state, root);
            Ok(())
        })?;
        run_pass(composition).map(|_| ())
    }

    /// Runs a pass when a scope of the composition is invalid, giving `true`; otherwise runs
    /// nothing and gives `false`.
    pub fn recompose(composition: Handle) -> ComposeResult<bool> {
        run_pass(composition)
    }

    /// Whether a scope of the composition was invalidated since its last pass, so that
    /// [`Composition::recompose`] may run one; it touches no queue.
    pub fn has_invalid_scopes(composition: Handle) -> ComposeResult<bool> {
        with_state(|state| Ok(!live_record(state, composition)?.queue.is_empty()))
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
        // Then the effects no pass ran (D13 of `state-handles`).
        let effects = with_state(|state| {
            let record = state.compositions.get_mut(composition).unwrap();
            std::mem::take(&mut record.effects)
        });
        for effect in effects {
            // SAFETY: a queued effect is a live function value the composition owns.
            unsafe { effect.release() };
        }
        // The applier is dropped outside the state.
        let record = with_state(|state| state.compositions.remove(composition));
        drop(record);
        // With no composition left on the thread, no node or handler can name a retired cell
        // any more, so the retired cells are freed (D7 of `owned-state`).
        if with_state(|state| state.live_compositions) == 0 {
            States::free_all_retired();
        }
        Ok(())
    }

    /// Runs `f` as a handler of `composition`, outside any pass: what `ui-toolkit` dispatches
    /// events through, and how side effects run. A state it creates belongs to whatever holds
    /// its state object (D16 of `owned-state`).
    pub fn run_handler<R>(composition: Handle, f: impl FnOnce() -> R) -> R {
        let _ = composition;
        f()
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
fn run_pass(composition: Handle) -> ComposeResult<bool> {
    let identifier = with_state(|state| -> ComposeResult<Option<u64>> {
        live_record(state, composition)?;
        if !order_scopes(state, composition) {
            return Ok(None);
        }
        state.pass_counter += 1;
        let record = state.compositions.get_mut(composition).unwrap();
        let compare_kept_arguments = std::mem::take(&mut record.reloaded);
        state.active = Some(ActivePass {
            identifier: state.pass_counter,
            composition,
            compare_kept_arguments,
        });
        Ok(Some(state.pass_counter))
    })?;
    let Some(identifier) = identifier else {
        return Ok(false);
    };
    let count = with_state(|state| state.order.len());
    for index in 0..count {
        let Some((function, composable)) = with_state(|state| open_scope(state, index)) else {
            continue;
        };
        // SAFETY: the runtime keeps the function value alive until the end of the pass at
        // least, since a replaced one is released only then.
        unsafe {
            if composable {
                function.call_composable(identifier, FORCE)
            } else {
                function.call_with_word(identifier)
            }
        };
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
    with_state(|state| state.releasing_pass = identifier);
    process_releases();
    with_state(|state| state.releasing_pass = 0);
    // The cells retired before the pass started, whose readers and holders it recomposed or
    // removed, are freed after its releases and before its side effects (D7 of `owned-state`).
    States::free_retired_before(identifier);
    // Reloads applied during the pass invalidate their scopes for the next one (D3 of
    // `compose-hot-reload`).
    crate::reload::handle_queued_reloads();
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
/// The flag says that the function is composable root content.
fn open_scope(state: &mut State, index: usize) -> Option<(FunctionValue, bool)> {
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
    let mut composable = false;
    match group.parent {
        None => {
            group.invalid = false;
            group.visited = pass.identifier;
            let record = state.compositions.get(pass.composition).unwrap();
            composable = record.composable_root;
            frame.node_parent = Some(record.applier.root());
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
    Some((function, composable))
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
    state.retired_in_pass.clear();
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
            },
            ReleaseStep::Function(function) => unsafe { function.release() },
        }
    }
}

/// The next step of the releases: the walk of a removed subtree first (children last first,
/// then the group's node with the handlers it owns, its restart function and its slots last
/// first: D8 of `owned-state`), then the release list in order.
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
                // The node first, so no node or handler names a value its slots own once they
                // are dropped (R1 of `owned-state`).
                if let Some(node) = record.node.take() {
                    let parent_node = record.parent_node;
                    let composition = record.composition;
                    if let Some(composition) = state.compositions.get_mut(composition) {
                        if let (true, Some(parent)) = (top, parent_node) {
                            composition.applier.remove(parent, node);
                        }
                        composition.applier.release(node);
                    }
                    state.release_work.push(work);
                    return ReleaseStep::Continue;
                }
                if let Some(function) = record.restart.take() {
                    state.release_work.push(work);
                    return ReleaseStep::Function(function);
                }
                if let Some(slot) = record.slots.pop() {
                    state.release_work.push(work);
                    return ReleaseStep::Slot(slot);
                }
                state.groups.remove(group);
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
