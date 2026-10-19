//! What a hot reload recomposes (D3 to D8 of `compose-hot-reload`).
//!
//! The program thread calls [`reload_applied`] once a reload's slots are written, through the
//! runtime's applied-reload hook. It names the composable bodies the reload affects by their
//! body keys, the keys `compose-transform` gives their own groups, and the slots its splits
//! retired. [`invalidate_reloaded`] walks every live composition of the thread once, in
//! pre-order with an explicit stack that carries the nearest enclosing restart scope whose
//! restart function is current, the root counting as one:
//!
//! - a group whose key is an affected body's invalidates that scope, or itself when it is a
//!   current restart group, so a non-skippable composable, a content lambda or a caller of an
//!   edited helper recomposes from the scope above it (D4);
//! - a restart group whose restart function names a retired slot is marked to reset its slots
//!   and is never a scope: its invalidation goes to the scope above it, whose new code calls the
//!   composable's new version, and when that version starts the group again it forgets the
//!   old slots and keeps its children (D8).
//!
//! The invalidated scopes then recompose in an ordinary pass: no pass is forced (D6). A reload
//! applied during a pass, at a function entry of a plain composition's body, is queued and
//! handled once the pass's releases are done, so the walk never sees a tree being updated (D3).
//! The walk is linear in the live groups, and a pass without a reload does no work for it.

use crate::group::GroupKind;
use crate::handle::Handle;
use crate::thread::{with_state, State};
use prism_runtime::reload::AppliedReload;
use std::collections::HashSet;

/// The applied-reload hook (`prism_runtime::reload::set_applied_hook`), which
/// `prism_codegen::Program::start` installs: invalidates the scopes `applied` affects at once,
/// or after the pass when one is running.
pub fn reload_applied(applied: &AppliedReload) {
    let queued = with_state(|state| {
        if state.active.is_some() {
            state.queued_reloads.push((
                applied.composable_bodies.clone(),
                applied.retired_slots.clone(),
            ));
            true
        } else {
            false
        }
    });
    if !queued {
        let bodies: HashSet<u64> = applied.composable_bodies.iter().copied().collect();
        let retired: HashSet<usize> = applied.retired_slots.iter().copied().collect();
        invalidate_reloaded(&bodies, &retired);
    }
}

/// Handles the reloads applied during the pass that just ended, after its releases.
pub(crate) fn handle_queued_reloads() {
    let queued = with_state(|state| {
        if state.queued_reloads.is_empty() || state.active.is_some() {
            None
        } else {
            Some(std::mem::take(&mut state.queued_reloads))
        }
    });
    for (bodies, retired) in queued.into_iter().flatten() {
        let bodies: HashSet<u64> = bodies.into_iter().collect();
        let retired: HashSet<usize> = retired.into_iter().collect();
        invalidate_reloaded(&bodies, &retired);
    }
}

/// Invalidates, in every live composition of the thread, the nearest current restart scope
/// above each group whose key is in `bodies`, and marks the restart groups whose restart
/// functions name a slot of `retired` to reset their slots, moving their invalidation to the
/// scope above them. Gives the number of scopes it invalidated, and records it for
/// [`take_invalidated_scopes`]. One walk, linear in the live groups.
pub fn invalidate_reloaded(bodies: &HashSet<u64>, retired: &HashSet<usize>) -> usize {
    let count = with_state(|state| {
        let mut targets = std::mem::take(&mut state.reload_targets);
        let mut stack = std::mem::take(&mut state.reload_walk);
        targets.clear();
        stack.clear();
        for root in live_roots(state) {
            stack.push((root, root));
            while let Some((group, scope)) = stack.pop() {
                let Some(record) = state.groups.get(group) else {
                    continue;
                };
                let mut current = scope;
                if record.kind == GroupKind::Restart && record.parent.is_some() {
                    // SAFETY: a kept restart function is a live function value, whose
                    // descriptor is static data of the program.
                    let slot = record
                        .restart
                        .as_ref()
                        .map(|restart| unsafe { restart.slot() } as usize);
                    if slot.is_some_and(|slot| retired.contains(&slot)) {
                        targets.push(scope);
                        state.groups.get_mut(group).unwrap().reset_slots = true;
                    } else {
                        current = group;
                    }
                }
                let record = state.groups.get(group).unwrap();
                if bodies.contains(&(record.key as u64)) {
                    targets.push(current);
                }
                for &child in record.children.iter().rev() {
                    stack.push((child, current));
                }
            }
        }
        let mut count = 0;
        for &target in &targets {
            if mark_invalid(state, target) {
                count += 1;
            }
        }
        state.reload_targets = targets;
        state.reload_walk = stack;
        state.invalidated_by_reloads += count;
        count
    });
    count
}

/// The roots of the thread's live compositions.
fn live_roots(state: &State) -> Vec<Handle> {
    state
        .compositions
        .iter()
        .filter(|(_, record)| !record.disposed)
        .map(|(_, record)| record.root)
        .collect()
}

/// Invalidates `scope` unless it is invalid already, giving whether it did.
fn mark_invalid(state: &mut State, scope: Handle) -> bool {
    match state.groups.get(scope) {
        Some(group) if !group.invalid && !group.removed => {
            crate::composer::invalidate_in(state, scope);
            true
        }
        _ => false,
    }
}

/// The number of scopes reloads have invalidated on this thread since the last call, for
/// `--timings` (`timing: reload pass <ms> (<n> scopes)`, D13 of `compose-hot-reload`).
pub fn take_invalidated_scopes() -> usize {
    with_state(|state| std::mem::take(&mut state.invalidated_by_reloads))
}
