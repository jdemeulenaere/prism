//! The protocol operations of a pass (D5, D6, D8 and D9 of `compose-runtime`).
//!
//! A [`Composer`] is the identifier of the pass in progress (D3): every operation compares it
//! with the thread's active pass, and reaches the composition through that pass. The composer
//! keeps a stack of open groups ([`Frame`]). Starting a group matches it against the next
//! unvisited old child of the open group, or, after the first miss under that parent, through a
//! key map built once per parent per pass, so starts cost amortized O(1). Ending a group
//! commits its new children, removes the unvisited old ones and forgets its slots after the
//! cursor; everything removed or forgotten is released at the end of the pass (D7).

use crate::applier::NodeHandle;
use crate::group::{Group, GroupKind};
use crate::handle::Handle;
use crate::slot::{Slot, SlotType};
use crate::thread::{with_state, ActivePass, Frame, FrameKind, GroupKey, KeyMap, Release, State};
use prism_runtime::FunctionValue;

/// A misuse of the composition protocol, with its detail: the runtime error
/// `composition misuse: <detail>` when an entry point meets it (D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Misuse(pub &'static str);

/// The result of a protocol operation.
pub type ComposeResult<T> = Result<T, Misuse>;

pub const OUTSIDE_PASS: Misuse = Misuse("composer used outside its pass");
pub const STALE_COMPOSITION: Misuse = Misuse("stale composition handle");
pub const DURING_PASS: Misuse = Misuse("composition operation during a pass");
pub const NO_SLOT: Misuse = Misuse("no slot to read");
pub const SLOT_OUTSIDE_GROUP: Misuse = Misuse("slot outside a group");
pub const NOT_STARTED: Misuse = Misuse("end of a group that was not started");
pub const WRONG_KIND: Misuse = Misuse("group ended with the wrong kind");
pub const LEFT_OPEN: Misuse = Misuse("group left open");

/// No position.
const NONE: u32 = u32::MAX;

/// Change bits of a composable's parameter (D4 of `compose-transform`): the caller knows
/// nothing of the argument.
pub const UNCERTAIN: i64 = 0;
/// The argument is the same as on the call's last composition.
pub const SAME: i64 = 1;
/// The argument differs from the one of the call's last composition.
pub const DIFFERENT: i64 = 2;
/// Added by [`Composer::keep`] to its result when the kept value changed.
pub const KEEP_CHANGED: i64 = 4;

/// The composer of a pass: the pass's identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Composer(pub u64);

/// The active pass, when `composer` is its composer.
fn check(state: &State, composer: Composer) -> ComposeResult<ActivePass> {
    match state.active {
        Some(pass) if pass.identifier == composer.0 && !state.frames.is_empty() => Ok(pass),
        _ => Err(OUTSIDE_PASS),
    }
}

impl Composer {
    /// Starts a restart, replaceable or movable group (`data_key` is ignored for the others).
    pub fn start_group(self, kind: GroupKind, key: i64, data_key: i64) -> ComposeResult<()> {
        assert!(
            kind != GroupKind::Node,
            "node groups start with start_node_group"
        );
        with_state(|state| start(state, self, kind, key, data_key, 0)).map(|_| ())
    }

    /// Starts a node group of `kind` and gives its node: the one it had when it was matched
    /// with the same kind, or a new one.
    pub fn start_node_group(self, key: i64, kind: i64) -> ComposeResult<NodeHandle> {
        with_state(|state| start(state, self, GroupKind::Node, key, 0, kind))
            .map(|node| node.expect("a node group has a node"))
    }

    /// Ends the innermost open group, a replaceable, movable or node group.
    pub fn end_group(self) -> ComposeResult<()> {
        with_state(|state| {
            check(state, self)?;
            let frame = *state.frames.last().unwrap();
            match frame.kind {
                FrameKind::Group(GroupKind::Restart) => Err(WRONG_KIND),
                FrameKind::Group(_) => {
                    finish_frame(state);
                    Ok(())
                }
                FrameKind::Root | FrameKind::Window => Err(NOT_STARTED),
            }
        })
    }

    /// Ends the innermost open group, a restart group, which keeps `restart` until it next
    /// ends; the restart function it replaces is released at the end of the pass.
    pub fn end_restart_group(self, restart: FunctionValue) -> ComposeResult<()> {
        with_state(|state| {
            check(state, self)?;
            let frame = *state.frames.last().unwrap();
            match frame.kind {
                FrameKind::Group(GroupKind::Restart) => {
                    let group = state.groups.get_mut(frame.group).unwrap();
                    if let Some(old) = group.restart.replace(restart) {
                        state.releases.push_back(Release::Function(old));
                    }
                    finish_frame(state);
                    Ok(())
                }
                FrameKind::Group(_) => Err(WRONG_KIND),
                FrameKind::Root | FrameKind::Window => Err(NOT_STARTED),
            }
        })
    }

    /// Whether the innermost open group is a restart group that existed before the pass and
    /// is not invalid.
    pub fn skipping(self) -> ComposeResult<bool> {
        with_state(|state| {
            check(state, self)?;
            let frame = state.frames.last().unwrap();
            Ok(frame.kind == FrameKind::Group(GroupKind::Restart) && frame.skippable)
        })
    }

    /// Keeps the innermost open group's remaining children, slots and nodes as they are.
    pub fn skip_to_group_end(self) -> ComposeResult<()> {
        with_state(|state| {
            let pass = check(state, self)?;
            let top = state.frames.len() - 1;
            if state.frames[top].kind == FrameKind::Window {
                return Err(NOT_STARTED);
            }
            skip_rest(state, top, pass);
            Ok(())
        })
    }

    /// The current scope: the innermost restart group open, or the composition's root.
    pub fn current_scope(self) -> ComposeResult<Handle> {
        with_state(|state| {
            check(state, self)?;
            Ok(state.frames.last().unwrap().scope)
        })
    }

    /// The address of the value of the slot at the cursor when it has `slot_type`, moving
    /// the cursor past it; null otherwise. The address stays valid until the pass ends.
    ///
    /// # Safety
    ///
    /// `slot_type` must point at a valid slot type record.
    pub unsafe fn slot(self, slot_type: *const SlotType) -> ComposeResult<*const u8> {
        with_state(|state| {
            let (group, cursor) = slot_position(state, self)?;
            let slots = &state.groups.get(group).unwrap().slots;
            match slots
                .get(cursor)
                .and_then(|slot| slot.typed_match(slot_type))
            {
                Some(value) => {
                    advance(state);
                    Ok(value as *const u8)
                }
                None => Ok(std::ptr::null()),
            }
        })
    }

    /// Moves the value at `value` into the slot at the cursor, forgetting a slot of another
    /// type there, and moves the cursor past it.
    ///
    /// # Safety
    ///
    /// `slot_type` must be a valid record and `value` a value of it that the caller gives up.
    pub unsafe fn set_slot(self, slot_type: *const SlotType, value: *mut u8) -> ComposeResult<()> {
        with_state(|state| {
            slot_position(state, self)?;
            store(state, slot_type, value);
            Ok(())
        })
    }

    /// Compares the value at `value` with the slot at the cursor: `false` when the slot has
    /// its type and the type's equality entry finds them equal, after which the new value is
    /// released; otherwise the new value is stored there, forgetting any old slot, and `true`.
    ///
    /// # Safety
    ///
    /// `slot_type` must be a valid record and `value` a value of it that the caller gives up.
    pub unsafe fn changed(self, slot_type: *const SlotType, value: *mut u8) -> ComposeResult<bool> {
        let record = &*slot_type;
        // Find the old value; the equality entry is program code, called outside the state.
        let old = with_state(|state| -> ComposeResult<Option<*mut u8>> {
            let (group, cursor) = slot_position(state, self)?;
            let slots = &state.groups.get(group).unwrap().slots;
            let old = slots
                .get(cursor)
                .and_then(|slot| slot.typed_match(slot_type));
            match (old, record.equals) {
                (Some(old), Some(_)) => Ok(Some(old)),
                _ => {
                    store(state, slot_type, value);
                    Ok(None)
                }
            }
        })?;
        let Some(old) = old else {
            return Ok(true);
        };
        let equal = (record.equals.unwrap())(old, value) != 0;
        let stored = with_state(|state| -> ComposeResult<bool> {
            let (group, cursor) = slot_position(state, self)?;
            let slots = &state.groups.get(group).unwrap().slots;
            let same = slots
                .get(cursor)
                .and_then(|slot| slot.typed_match(slot_type))
                == Some(old);
            if equal && same {
                advance(state);
                Ok(false)
            } else {
                store(state, slot_type, value);
                Ok(true)
            }
        })?;
        if !stored {
            if let Some(drop) = record.drop {
                drop(value);
            }
            return Ok(false);
        }
        Ok(true)
    }

    /// Keeps the value at `value`, an argument of a skippable composable, in the slot at the
    /// cursor by the two change bits `passed` its caller gave it (D8 of `compose-transform`):
    /// uncertain compares it as [`Composer::changed`] does, different stores it, and same or
    /// static read past the kept slot, storing the value only when there is none, except in the
    /// pass after a reload, which compares them as uncertain ones (the caller's code may have
    /// changed since the value was kept, `compose-hot-reload`). Returns the
    /// bits the body now knows (same, different or static), with [`KEEP_CHANGED`] added when
    /// the value differs from the kept one. The caller gives the value up either way.
    ///
    /// # Safety
    ///
    /// `slot_type` must be a valid record and `value` a value of it that the caller gives up.
    pub unsafe fn keep(
        self,
        slot_type: *const SlotType,
        value: *mut u8,
        passed: i64,
    ) -> ComposeResult<i64> {
        match passed & 3 {
            UNCERTAIN => Ok(if self.changed(slot_type, value)? {
                DIFFERENT | KEEP_CHANGED
            } else {
                SAME
            }),
            DIFFERENT => {
                self.set_slot(slot_type, value)?;
                Ok(DIFFERENT | KEEP_CHANGED)
            }
            known => {
                // In the pass after a reload, the call site that computed the bits may be new
                // code passing another value than the one kept, so the value is compared.
                let kept = with_state(|state| -> ComposeResult<Option<bool>> {
                    if check(state, self)?.compare_kept_arguments {
                        return Ok(None);
                    }
                    let (group, cursor) = slot_position(state, self)?;
                    let slots = &state.groups.get(group).unwrap().slots;
                    let present = slots
                        .get(cursor)
                        .and_then(|slot| slot.typed_match(slot_type))
                        .is_some();
                    if present {
                        advance(state);
                    } else {
                        store(state, slot_type, value);
                    }
                    Ok(Some(present))
                })?;
                match kept {
                    None if self.changed(slot_type, value)? => Ok(DIFFERENT | KEEP_CHANGED),
                    None => Ok(known),
                    Some(true) => {
                        if let Some(drop) = (*slot_type).drop {
                            drop(value);
                        }
                        Ok(known)
                    }
                    Some(false) => Ok(known),
                }
            }
        }
    }

    /// During a restart, before the restart function starts a group, the address of the value
    /// of slot `index` of the restarted scope's group when that slot has `slot_type`: how a
    /// restart function reads the arguments its composable kept (D10 of `compose-transform`).
    /// Otherwise, and outside a restart, the misuse `no slot to read`. The address stays valid
    /// until the pass ends.
    ///
    /// # Safety
    ///
    /// `slot_type` must point at a valid slot type record.
    pub unsafe fn kept_slot(
        self,
        index: usize,
        slot_type: *const SlotType,
    ) -> ComposeResult<*const u8> {
        with_state(|state| {
            check(state, self)?;
            let frame = state.frames.last().unwrap();
            if frame.kind != FrameKind::Window {
                return Err(NO_SLOT);
            }
            let slots = &state.groups.get(frame.window_scope).unwrap().slots;
            slots
                .get(index)
                .and_then(|slot| slot.typed_match(slot_type))
                .map(|value| value as *const u8)
                .ok_or(NO_SLOT)
        })
    }

    /// Whether the slot at the cursor is a word slot of `tag`.
    pub fn has_word(self, tag: i64) -> ComposeResult<bool> {
        with_state(|state| {
            let (group, cursor) = slot_position(state, self)?;
            let slots = &state.groups.get(group).unwrap().slots;
            Ok(matches!(slots.get(cursor), Some(Slot::Word { tag: own, .. }) if *own == tag))
        })
    }

    /// The value of the word slot at the cursor, moving the cursor past it.
    pub fn word(self) -> ComposeResult<i64> {
        with_state(|state| {
            let (group, cursor) = slot_position(state, self)?;
            let slots = &state.groups.get(group).unwrap().slots;
            match slots.get(cursor) {
                Some(Slot::Word { value, .. }) => {
                    let value = *value;
                    advance(state);
                    Ok(value)
                }
                _ => Err(NO_SLOT),
            }
        })
    }

    /// Writes a word slot of `tag` holding `value` at the cursor, forgetting a slot of another
    /// type there, and moves the cursor past it.
    pub fn set_word(self, tag: i64, value: i64) -> ComposeResult<()> {
        with_state(|state| {
            slot_position(state, self)?;
            store_word(state, tag, value);
            Ok(())
        })
    }

    /// Compares `value` with the word slot of `tag` at the cursor: `false` when it holds
    /// `value`; otherwise writes it there and gives `true`. The cursor moves past the slot.
    pub fn changed_word(self, tag: i64, value: i64) -> ComposeResult<bool> {
        with_state(|state| {
            let (group, cursor) = slot_position(state, self)?;
            let slots = &state.groups.get(group).unwrap().slots;
            if matches!(slots.get(cursor), Some(Slot::Word { tag: own, value: old }) if *own == tag && *old == value)
            {
                advance(state);
                return Ok(false);
            }
            store_word(state, tag, value);
            Ok(true)
        })
    }
}

impl Composer {
    /// Opens the window of a `remember` calculation that runs because its slot is missing or a
    /// key changed (D11 of `state-handles`): the cells created until
    /// [`Composer::end_remember`] belong to the slot the calculation's result is stored in.
    pub fn begin_remember(self) -> ComposeResult<()> {
        with_state(|state| {
            check(state, self)?;
            state
                .creation_owners
                .push(crate::state::CreationOwner::Remember(Vec::new()));
            Ok(())
        })
    }

    /// Closes the window of [`Composer::begin_remember`], after the result's slot was written:
    /// attaches the cells created since to the slot before the cursor, or is the misuse `no slot
    /// to read` when there is none.
    pub fn end_remember(self) -> ComposeResult<()> {
        with_state(|state| {
            let (group, cursor) = slot_position(state, self)?;
            let cells = match state.creation_owners.pop() {
                Some(crate::state::CreationOwner::Remember(cells)) => cells,
                Some(other) => {
                    state.creation_owners.push(other);
                    return Err(NO_SLOT);
                }
                None => return Err(NO_SLOT),
            };
            if cells.is_empty() {
                return Ok(());
            }
            let slots = &mut state.groups.get_mut(group).unwrap().slots;
            match cursor.checked_sub(1).and_then(|index| slots.get_mut(index)) {
                Some(Slot::Typed {
                    cells: attached, ..
                }) => {
                    attached.extend(cells);
                    Ok(())
                }
                _ => Err(NO_SLOT),
            }
        })
    }

    /// Queues the side effect `effect`, a `() -> Unit` function value the composition takes
    /// over, to run after the pass (D13 of `state-handles`).
    pub fn side_effect(self, effect: FunctionValue) -> ComposeResult<()> {
        with_state(|state| {
            let pass = check(state, self)?;
            let record = state.compositions.get_mut(pass.composition).unwrap();
            record.effects.push(effect);
            Ok(())
        })
    }
}

/// Invalidates the restart scope `scope` in O(1): it is recomposed by the next pass of its
/// composition. A handle that names no live scope is ignored. A scope invalidated during a
/// pass goes to the next pass (D4).
pub fn invalidate(scope: Handle) {
    with_state(|state| invalidate_in(state, scope))
}

pub(crate) fn invalidate_in(state: &mut State, scope: Handle) {
    let Some(group) = state.groups.get_mut(scope) else {
        return;
    };
    if group.kind != GroupKind::Restart || group.removed {
        return;
    }
    if state.active.is_some() {
        if !group.pending {
            group.pending = true;
            state.deferred.push(scope);
        }
        return;
    }
    if group.invalid {
        return;
    }
    let composition = group.composition;
    match state.compositions.get_mut(composition) {
        Some(record) if !record.disposed => {
            group.invalid = true;
            record.queue.push(scope);
        }
        _ => {}
    }
}

/// The open group whose slots an operation reads, and its cursor.
fn slot_position(state: &State, composer: Composer) -> ComposeResult<(Handle, usize)> {
    check(state, composer)?;
    let frame = state.frames.last().unwrap();
    if frame.kind == FrameKind::Window {
        return Err(SLOT_OUTSIDE_GROUP);
    }
    Ok((frame.group, frame.slot_cursor as usize))
}

/// Moves the slot cursor of the innermost open group past one slot.
fn advance(state: &mut State) {
    state.frames.last_mut().unwrap().slot_cursor += 1;
}

/// Puts a value of `slot_type` at the cursor of the innermost open group and moves past it.
/// A slot of the same type whose values need no drop is overwritten in place; any other slot
/// there is forgotten, to be released at the end of the pass, so every address given out
/// during the pass stays valid.
///
/// # Safety
///
/// As for [`Composer::set_slot`].
unsafe fn store(state: &mut State, slot_type: *const SlotType, value: *mut u8) {
    let frame = *state.frames.last().unwrap();
    let cursor = frame.slot_cursor as usize;
    let slots = &mut state.groups.get_mut(frame.group).unwrap().slots;
    match slots.get_mut(cursor) {
        Some(slot) => {
            // A slot holding state cells is replaced, so its cells are released with it (D12
            // of `state-handles`).
            let in_place = slot.cells().is_empty() && (*slot_type).drop.is_none();
            if let (Some(old), true) = (slot.typed_match(slot_type), in_place) {
                std::ptr::copy_nonoverlapping(value, old, (*slot_type).size);
            } else {
                let old = std::mem::replace(slot, Slot::typed(slot_type, value));
                state.releases.push_back(Release::Slot(old));
            }
        }
        None => slots.push(Slot::typed(slot_type, value)),
    }
    advance(state);
}

/// Puts a word slot at the cursor of the innermost open group and moves past it.
fn store_word(state: &mut State, tag: i64, value: i64) {
    let frame = *state.frames.last().unwrap();
    let cursor = frame.slot_cursor as usize;
    let slots = &mut state.groups.get_mut(frame.group).unwrap().slots;
    match slots.get_mut(cursor) {
        Some(slot @ Slot::Word { .. }) => *slot = Slot::Word { tag, value },
        Some(slot) => {
            let old = std::mem::replace(slot, Slot::Word { tag, value });
            state.releases.push_back(Release::Slot(old));
        }
        None => slots.push(Slot::Word { tag, value }),
    }
    advance(state);
}

/// Starts a group under the innermost open one: reuses the first unvisited old child with the
/// same kind and key (and data key for a movable group), or creates one, and opens it. A node
/// group gets its node, of `node_kind`, and places it in its node context.
fn start(
    state: &mut State,
    composer: Composer,
    kind: GroupKind,
    key: i64,
    data_key: i64,
    node_kind: i64,
) -> ComposeResult<Option<NodeHandle>> {
    let pass = check(state, composer)?;
    let top = state.frames.len() - 1;
    let data_key = if kind == GroupKind::Movable {
        data_key
    } else {
        0
    };
    let parent = state.frames[top].group;
    let found = find_match(state, top, (kind, key, data_key), pass.identifier);
    let (group, reused) = match found {
        Some(group) => (group, true),
        None => {
            let composition = state.groups.get(parent).unwrap().composition;
            let mut record = Group::new(kind, key, data_key, composition);
            record.parent = Some(parent);
            record.created = pass.identifier;
            record.visited = pass.identifier;
            (state.groups.insert(record), false)
        }
    };
    state.scratch.push(group);
    let parent_frame = state.frames[top];
    let mut frame = Frame {
        group,
        kind: FrameKind::Group(kind),
        next_old: 0,
        old_start: 0,
        old_end: 0,
        high_water: 0,
        map: None,
        map_base: 0,
        slot_cursor: 0,
        new_start: state.scratch.len() as u32,
        scope: parent_frame.scope,
        skippable: false,
        context: parent_frame.context,
        placements_at_start: state.frames[parent_frame.context as usize].placements,
        node_parent: None,
        parent_known: true,
        last_placed: None,
        previous_known: true,
        placements: 0,
        in_order: true,
        window_scope: group,
        window_old_last: None,
        replaced_node: None,
    };
    let record = state.groups.get_mut(group).unwrap();
    frame.old_end = record.children.len() as u32;
    let mut node = None;
    match kind {
        GroupKind::Restart => {
            frame.scope = group;
            // A reload applied during this pass has not walked the tree yet: a restart group
            // whose restart function it retired is reset as the walk would mark it.
            if reused && !state.retired_in_pass.is_empty() {
                // SAFETY: a kept restart function is a live function value, whose descriptor
                // is static data of the program.
                let slot = record
                    .restart
                    .as_ref()
                    .map(|restart| unsafe { restart.slot() } as usize);
                if slot.is_some_and(|slot| state.retired_in_pass.contains(&slot)) {
                    record.reset_slots = true;
                }
            }
            frame.skippable = reused && !record.invalid && !record.reset_slots;
            record.invalid = false;
            // A scope whose restart function a reload retired starts afresh: its kept arguments
            // and remembered values were laid out by the old code. Its children keep their
            // state (D8 of `compose-hot-reload`).
            if std::mem::take(&mut record.reset_slots) {
                let slots = std::mem::take(&mut record.slots);
                state.releases.extend(slots.into_iter().map(Release::Slot));
            }
        }
        GroupKind::Node => {
            let (existing, existing_kind, old_parent) =
                (record.node, record.node_kind, record.parent_node);
            let created = match existing {
                Some(existing) if existing_kind == node_kind => existing,
                _ => {
                    let applier = &mut state
                        .compositions
                        .get_mut(pass.composition)
                        .unwrap()
                        .applier;
                    let created = applier.create(node_kind);
                    if let Some(existing) = existing {
                        frame.replaced_node = Some((old_parent, existing));
                    }
                    let record = state.groups.get_mut(group).unwrap();
                    record.node = Some(created);
                    record.node_kind = node_kind;
                    // A new node holds none of the nodes of the group's old children yet.
                    frame.in_order = false;
                    created
                }
            };
            let context = parent_frame.context as usize;
            let in_place = reused && existing == Some(created) && state.frames[context].in_order;
            if in_place {
                let frame = &mut state.frames[context];
                frame.last_placed = Some(created);
                frame.previous_known = true;
                frame.placements += 1;
            } else {
                let parent_node = place(state, context, created, pass.composition);
                state.groups.get_mut(group).unwrap().parent_node = Some(parent_node);
            }
            let record = state.groups.get_mut(group).unwrap();
            record.last_node = Some(created);
            frame.context = state.frames.len() as u32;
            frame.node_parent = Some(created);
            node = Some(created);
        }
        GroupKind::Replaceable | GroupKind::Movable => {}
    }
    state.frames.push(frame);
    Ok(node)
}

/// The unvisited old child of the open group at `top` that matches `key`, marked visited.
fn find_match(state: &mut State, top: usize, key: GroupKey, pass: u64) -> Option<Handle> {
    let frame = state.frames[top];
    let parent = state.groups.get(frame.group).unwrap();
    if frame.map.is_none() {
        let mut position = frame.next_old;
        while position < frame.old_end {
            let child = parent.children[position as usize];
            let record = state.groups.get(child).unwrap();
            if record.visited == pass {
                position += 1;
                continue;
            }
            if record.matches(key.0, key.1, key.2) {
                state.frames[top].next_old = position + 1;
                state.frames[top].high_water = position + 1;
                state.groups.get_mut(child).unwrap().visited = pass;
                return Some(child);
            }
            break;
        }
        state.frames[top].next_old = position;
        if position >= frame.old_end {
            return None;
        }
        build_map(state, top, pass);
    }
    let frame = state.frames[top];
    let map = &mut state.maps[frame.map.unwrap() as usize];
    loop {
        let head = *map.heads.get(&key)?;
        let next = map.next[(head - frame.map_base) as usize];
        if next == NONE {
            map.heads.remove(&key);
        } else {
            map.heads.insert(key, next);
        }
        let child = state.groups.get(frame.group).unwrap().children[head as usize];
        let record = state.groups.get_mut(child).unwrap();
        if record.visited != pass {
            record.visited = pass;
            matched_at(state, top, head);
            return Some(child);
        }
    }
}

/// Records a match at old position `position` of the open group at `top`: below the highest
/// one matched so far, the group moved before a sibling already placed, so the node context is
/// out of order and its nodes have to be placed again. Skipping unvisited siblings keeps it in
/// order: they will be removed or matched out of order later.
fn matched_at(state: &mut State, top: usize, position: u32) {
    let frame = &mut state.frames[top];
    if position < frame.high_water {
        let context = frame.context as usize;
        state.frames[context].in_order = false;
    } else {
        frame.high_water = position + 1;
    }
}

/// Builds the key map of the unvisited old children of the open group at `top`, from its
/// next old child on: linear in them, once per parent per pass.
fn build_map(state: &mut State, top: usize, pass: u64) {
    let index = state.maps_used;
    if index == state.maps.len() {
        state.maps.push(KeyMap::default());
    }
    state.maps_used += 1;
    let frame = state.frames[top];
    let map = &mut state.maps[index];
    map.heads.clear();
    map.next.clear();
    map.next
        .resize((frame.old_end - frame.next_old) as usize, NONE);
    let children = &state.groups.get(frame.group).unwrap().children;
    for position in (frame.next_old..frame.old_end).rev() {
        let record = state.groups.get(children[position as usize]).unwrap();
        if record.visited == pass {
            continue;
        }
        let key = (record.kind, record.key, record.data_key);
        let previous = map.heads.insert(key, position).unwrap_or(NONE);
        map.next[(position - frame.next_old) as usize] = previous;
    }
    state.frames[top].map = Some(index as u32);
    state.frames[top].map_base = frame.next_old;
}

/// Places `node` in the node context of the frame at `context`, after the last node placed
/// there, and gives the node it was placed into.
fn place(state: &mut State, context: usize, node: NodeHandle, composition: Handle) -> NodeHandle {
    resolve_context(state, context, composition);
    let frame = &mut state.frames[context];
    let parent = frame.node_parent.unwrap();
    let previous = frame.last_placed;
    frame.last_placed = Some(node);
    frame.placements += 1;
    let applier = &mut state.compositions.get_mut(composition).unwrap().applier;
    applier.place(parent, previous, node);
    parent
}

/// Resolves a window's node parent and previous node (D8): the node of the nearest node group
/// enclosing the scope, or the applier's root, and the last node, in group order, before the
/// scope within it. O(depth · siblings), only when the recomposed scope places a node.
fn resolve_context(state: &mut State, context: usize, composition: Handle) {
    let frame = state.frames[context];
    if frame.parent_known && frame.previous_known {
        return;
    }
    let mut child = frame.window_scope;
    let mut previous = None;
    let mut found = frame.previous_known;
    let node_parent;
    loop {
        let record = state.groups.get(child).unwrap();
        let parent = record.parent.expect("a window's scope is not the root");
        let parent_record = state.groups.get(parent).unwrap();
        if !found {
            let before = &parent_record.children[..record.index_in_parent as usize];
            for sibling in before.iter().rev() {
                if let Some(node) = state.groups.get(*sibling).unwrap().last_node {
                    previous = Some(node);
                    found = true;
                    break;
                }
            }
        }
        if parent_record.kind == GroupKind::Node {
            node_parent = parent_record.node;
            break;
        }
        if parent_record.parent.is_none() {
            node_parent = Some(state.compositions.get(composition).unwrap().applier.root());
            break;
        }
        child = parent;
    }
    let frame = &mut state.frames[context];
    frame.node_parent = node_parent;
    frame.parent_known = true;
    if !frame.previous_known {
        frame.last_placed = previous;
        frame.previous_known = true;
    }
}

/// Keeps the remaining old children of the open group at `top`, in order, and its slots, and
/// moves the node context past their nodes, placing them again when an earlier group moved.
fn skip_rest(state: &mut State, top: usize, pass: ActivePass) {
    let frame = state.frames[top];
    let context = frame.context as usize;
    for position in frame.old_start..frame.old_end {
        let child = state.groups.get(frame.group).unwrap().children[position as usize];
        let record = state.groups.get_mut(child).unwrap();
        if record.visited == pass.identifier {
            continue;
        }
        record.visited = pass.identifier;
        let last_node = record.last_node;
        state.scratch.push(child);
        matched_at(state, top, position);
        if last_node.is_none() {
            continue;
        }
        if state.frames[context].in_order {
            let frame = &mut state.frames[context];
            frame.last_placed = last_node;
            frame.previous_known = true;
            frame.placements += 1;
        } else {
            place_subtree(state, context, child, pass.composition);
        }
    }
    let slots = state.groups.get(frame.group).unwrap().slots.len() as u32;
    let frame = &mut state.frames[top];
    frame.slot_cursor = slots;
    frame.next_old = frame.old_end;
}

/// Places the top-level nodes of `group`'s subtree again, in order, in the node context at
/// `context`.
fn place_subtree(state: &mut State, context: usize, group: Handle, composition: Handle) {
    let mut walk = std::mem::take(&mut state.walk);
    walk.clear();
    walk.push(group);
    while let Some(group) = walk.pop() {
        let record = state.groups.get(group).unwrap();
        if record.kind == GroupKind::Node {
            if let Some(node) = record.node {
                let parent = place(state, context, node, composition);
                state.groups.get_mut(group).unwrap().parent_node = Some(parent);
            }
            continue;
        }
        let start = walk.len();
        walk.extend_from_slice(&record.children);
        walk[start..].reverse();
    }
    state.walk = walk;
}

/// Ends the innermost open frame: removes its unvisited old children, forgets its slots after
/// the cursor, commits its new children and records its last node.
pub(crate) fn finish_frame(state: &mut State) {
    let frame = state.frames.pop().unwrap();
    let pass = state.active.unwrap().identifier;
    let group = frame.group;
    for position in frame.old_start..frame.old_end {
        let child = state.groups.get(group).unwrap().children[position as usize];
        let record = state.groups.get_mut(child).unwrap();
        if record.visited != pass {
            record.removed = true;
            state.releases.push_back(Release::Group(child));
        }
    }
    if let Some((parent, node)) = frame.replaced_node {
        let composition = state.groups.get(group).unwrap().composition;
        state.releases.push_back(Release::Node {
            composition,
            parent,
            node,
        });
    }
    let new_children = frame.new_start as usize;
    if frame.kind == FrameKind::Window {
        let start = frame.old_start as usize;
        let record = state.groups.get_mut(group).unwrap();
        let same = state.scratch.len() - new_children == 1
            && state.scratch[new_children] == record.children[start];
        if !same {
            record.children.splice(
                start..start + 1,
                state.scratch[new_children..].iter().copied(),
            );
            renumber(state, group, start);
        }
        state.scratch.truncate(new_children);
        let new_last = if frame.placements > 0 {
            frame.last_placed
        } else {
            None
        };
        if new_last != frame.window_old_last || !same {
            propagate_last_node(state, group);
        }
    } else {
        let cursor = frame.slot_cursor as usize;
        let record = state.groups.get_mut(group).unwrap();
        // Forgotten last to first, as a removed group's slots are released.
        for slot in record.slots.drain(cursor.min(record.slots.len())..).rev() {
            state.releases.push_back(Release::Slot(slot));
        }
        record.children.clear();
        record
            .children
            .extend_from_slice(&state.scratch[new_children..]);
        state.scratch.truncate(new_children);
        renumber(state, group, 0);
        if frame.kind != FrameKind::Group(GroupKind::Node) && frame.kind != FrameKind::Root {
            let context = &state.frames[frame.context as usize];
            let last_node = if context.placements != frame.placements_at_start {
                context.last_placed
            } else {
                None
            };
            state.groups.get_mut(group).unwrap().last_node = last_node;
        }
    }
    if frame.map.is_some() {
        state.maps_used -= 1;
    }
}

/// Sets `index_in_parent` of `group`'s children from `start` on.
fn renumber(state: &mut State, group: Handle, start: usize) {
    let count = state.groups.get(group).unwrap().children.len();
    for index in start..count {
        let child = state.groups.get(group).unwrap().children[index];
        state.groups.get_mut(child).unwrap().index_in_parent = index as u32;
    }
}

/// Recomputes the last node of `group` and its ancestors up to the nearest node group or the
/// root, after a recomposed scope changed what it places.
fn propagate_last_node(state: &mut State, mut group: Handle) {
    loop {
        let record = state.groups.get(group).unwrap();
        if record.kind == GroupKind::Node {
            return;
        }
        let Some(parent) = record.parent else {
            return;
        };
        let last_node = record
            .children
            .iter()
            .rev()
            .find_map(|child| state.groups.get(*child).unwrap().last_node);
        state.groups.get_mut(group).unwrap().last_node = last_node;
        group = parent;
    }
}
