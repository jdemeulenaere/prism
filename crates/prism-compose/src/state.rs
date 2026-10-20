//! State cells (D6, D8 to D10 and D14 of `state-handles`, D3, D7 and D16 of `owned-state`).
//!
//! A `State<T>` or `MutableState<T>` is a state object that owns one cell: one word, a
//! generational [`Handle`] of a [`Cell`] in the thread's cell arena. A cell holds its value's
//! slot type record, its value in a block of exactly the record's size and alignment that never
//! moves while the cell lives (so a borrow of it stays valid however the state object moves and
//! however many cells are created), a borrow flag and the scopes that read it. The runtime does
//! not know who owns a cell: its state object does, and dropping the object retires the cell.
//!
//! - **Validation** (D8, D15). Every access validates the handle's index and generation and
//!   compares the record it is given with the cell's, so a handle can never reach a freed cell
//!   or a cell of another layout; a failure is the runtime error `stale state handle`.
//! - **Borrow flags** (D6, guards of `owned-state` D3). A cell is free, shared by guards
//!   (a count), empty while `update` holds its value, or exclusive while an exclusive guard
//!   holds it or a write drops the old value. A conflicting access is `state accessed while
//!   borrowed`.
//! - **Readers** (D9). A read during a pass registers the pass's current scope once per cell:
//!   a vector with a linear membership test, backed by a hash set beyond eight scopes, pruned
//!   of scopes that left their composition before it grows. A write invalidates them and
//!   forgets them.
//! - **Equal writes** (D10). A `value =` write compares the new value with the current one
//!   through the record's equality entry; an equal one is dropped, and nothing is invalidated.
//! - **Creation windows** (D15 of `owned-state`). A count of the windows that allow creating a
//!   cell during a pass: a `remember` calculation, or a top-level value's initializer. A cell
//!   created during a pass outside every window is `state created during composition outside
//!   remember`.
//! - **Retirement** (D7 of `owned-state`). Dropping a state object retires its cell: its
//!   readers are invalidated and forgotten, a weak reference to it upgrades to nothing, and no
//!   read registers a reader again, but its value stays valid, so borrows taken before and weak
//!   handles held by nodes keep working. The cell is freed, its value dropped, at the end of the
//!   first pass on the thread that starts after the retirement, after that pass's releases; at
//!   once when no composition is live on the thread, and when the last one is disposed.
//! - **Frees** (D12 of `state-handles`). Freeing a cell drops its value through its record and
//!   frees its block, outside the thread's state; a cell held by a guard or `update` when its
//!   free is due becomes stale at once and is freed when the last hold ends.
//! - **Versions** (D3 of `strong-skipping`). Each cell holds a 64-bit version from one counter
//!   per thread: a fresh one when it is created, and a new one whenever a write stores a value
//!   (not an equal one), `put` stores `update`'s result, an exclusive guard is taken, or it is
//!   retired. A composable keeps a borrow of a state's contents with the state's version, so a
//!   changed version means the data may have changed. Reading it registers no reader.
//! - **Errors** (D14). An entry that fails records the error's kind in the thread's state and
//!   returns a failure result; generated code then calls `prism_compose_state_error` with the
//!   access's location.

use crate::composer::invalidate_in;
use crate::handle::Handle;
use crate::slot::{allocate_block, free_block, SlotType};
use crate::thread::{with_state, State};
use prism_runtime::PanicKind;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether a composition was started, on any thread of the process. Until then no thread runs
/// a pass, so a top-level initializer may create cells without opening a creation window: a
/// program that does not use compose pays one load per top-level value initialized (D11 of
/// `state-handles`). The thread that sets it reads it back in program order, which is all a
/// thread's own windows need.
static IN_USE: AtomicBool = AtomicBool::new(false);

/// Records that compose is in use: called when a composition starts.
pub(crate) fn mark_in_use() {
    if !IN_USE.load(Ordering::Relaxed) {
        IN_USE.store(true, Ordering::Relaxed);
    }
}

/// How many readers a cell tests linearly before it keeps a hash set of them too.
const LINEAR_READERS: usize = 8;

/// How a cell is held (D6 of `state-handles`, D3 of `owned-state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// Nothing holds the cell.
    Free,
    /// `count` shared guards borrow its value.
    Shared(u32),
    /// `update` moved its value out.
    Empty,
    /// An exclusive guard borrows its value mutably, or a write drops its old value.
    Exclusive,
}

/// The scopes that read a cell during passes, each once (D9).
#[derive(Debug, Default)]
pub struct Readers {
    list: Vec<Handle>,
    set: Option<HashSet<Handle>>,
}

impl Readers {
    /// Whether `scope` is registered.
    fn contains(&self, scope: Handle) -> bool {
        match &self.set {
            Some(set) => set.contains(&scope),
            None => self.list.contains(&scope),
        }
    }

    /// Registers `scope` once; before the vector grows, the scopes that left their
    /// composition (no live group of `groups` has their handle) are removed, so it holds at
    /// most twice the live readers.
    fn register(&mut self, scope: Handle, live: impl Fn(Handle) -> bool) {
        if self.contains(scope) {
            return;
        }
        if self.list.len() == self.list.capacity() && !self.list.is_empty() {
            self.list.retain(|reader| live(*reader));
            if let Some(set) = &mut self.set {
                set.retain(|reader| live(*reader));
            }
        }
        self.list.push(scope);
        match &mut self.set {
            Some(set) => {
                set.insert(scope);
            }
            None if self.list.len() > LINEAR_READERS => {
                self.set = Some(self.list.iter().copied().collect());
            }
            None => {}
        }
    }

    /// The registered scopes, in registration order.
    pub fn scopes(&self) -> &[Handle] {
        &self.list
    }

    /// Forgets every scope, keeping the storage.
    fn clear(&mut self) {
        self.list.clear();
        if let Some(set) = &mut self.set {
            set.clear();
        }
    }
}

/// One state cell.
#[derive(Debug)]
pub struct Cell {
    pub record: *const SlotType,
    /// The block holding the value, of exactly the record's size and alignment.
    pub value: *mut u8,
    pub flag: Flag,
    pub readers: Readers,
    /// Whether its state object was dropped (D7 of `owned-state`): its readers were forgotten,
    /// and a weak reference to it upgrades to nothing; its value stays valid until it is freed.
    pub retired: bool,
    /// Whether its free came due while a guard or `update` held it: it is stale from then on,
    /// and freed when the last hold ends (D12 of `state-handles`).
    pub released: bool,
    /// Its version, renewed by every change of its value (D3 of `strong-skipping`).
    pub version: u64,
}

/// A state error waiting for its location (D14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateError {
    Stale,
    Borrowed,
    WrittenDuringComposition,
    CreatedDuringComposition,
}

impl StateError {
    /// The runtime error it is.
    pub fn kind(self) -> PanicKind {
        match self {
            StateError::Stale => PanicKind::StaleStateHandle,
            StateError::Borrowed => PanicKind::StateBorrowed,
            StateError::WrittenDuringComposition => PanicKind::StateWrittenDuringComposition,
            StateError::CreatedDuringComposition => PanicKind::StateCreatedDuringComposition,
        }
    }
}

/// The result of a state operation: its value, or the error it records.
pub type StateResult<T> = Result<T, StateError>;

/// The cell `handle` names with `record`, if it is live and not released.
fn cell_mut(state: &mut State, handle: Handle, record: *const SlotType) -> StateResult<&mut Cell> {
    match state.cells.get_mut(handle) {
        // SAFETY: records are static data valid while the program runs.
        Some(cell) if !cell.released && unsafe { (*cell.record).matches(&*record) } => Ok(cell),
        _ => Err(StateError::Stale),
    }
}

/// Registers the active pass's current scope as a reader of `handle` (D9), unless the cell is
/// retired: nothing writes it any more that its readers need to see.
fn register_reader(state: &mut State, handle: Handle) {
    if state.active.is_none() {
        return;
    }
    let Some(scope) = state.frames.last().map(|frame| frame.scope) else {
        return;
    };
    let State { cells, groups, .. } = state;
    if let Some(cell) = cells.get_mut(handle) {
        if !cell.retired {
            cell.readers
                .register(scope, |reader| groups.contains(reader));
        }
    }
}

/// Invalidates and forgets the readers of `handle` (D10).
fn invalidate_readers(state: &mut State, handle: Handle) {
    let Some(cell) = state.cells.get_mut(handle) else {
        return;
    };
    let mut readers = std::mem::take(&mut cell.readers);
    for reader in readers.scopes() {
        invalidate_in(state, *reader);
    }
    readers.clear();
    if let Some(cell) = state.cells.get_mut(handle) {
        cell.readers = readers;
    }
}

/// Fails with `state written during composition` while a pass runs (D11).
fn check_writable(state: &State) -> StateResult<()> {
    match state.active {
        Some(_) => Err(StateError::WrittenDuringComposition),
        None => Ok(()),
    }
}

/// Removes the cell `handle` from the arena when no hold is on it, for a free; else marks its
/// free due, for the end of the last hold.
fn take_if_free(state: &mut State, handle: Handle) -> Option<Cell> {
    let cell = state.cells.get_mut(handle)?;
    if cell.flag != Flag::Free {
        cell.released = true;
        return None;
    }
    state.cells.remove(handle)
}

/// Drops the value of a freed cell through its record and frees its block. Program code runs,
/// so no borrow of the state may be held.
///
/// # Safety
///
/// `cell` was removed from the arena, its value is initialized unless `empty`.
unsafe fn destroy(cell: Cell, empty: bool) {
    let record = &*cell.record;
    if !empty {
        if let Some(drop) = record.drop {
            drop(cell.value);
        }
    }
    free_block(record, cell.value);
}

/// The state operations, named by the handles generated code passes.
pub struct States;

impl States {
    /// Creates a cell of `record` holding the value at `value`, moved in, for the state object
    /// that owns it; during a pass, only in a creation window (D15 of `owned-state`).
    ///
    /// # Safety
    ///
    /// `record` must be a valid record and `value` a value of it that the caller gives up.
    pub unsafe fn create(record: *const SlotType, value: *mut u8) -> StateResult<Handle> {
        with_state(|state| {
            if state.creation_windows == 0 && state.active.is_some() {
                return Err(StateError::CreatedDuringComposition);
            }
            let block = allocate_block(&*record);
            std::ptr::copy_nonoverlapping(value, block, (*record).size);
            let version = state.fresh_version();
            Ok(state.cells.insert(Cell {
                record,
                value: block,
                flag: Flag::Free,
                readers: Readers::default(),
                retired: false,
                released: false,
                version,
            }))
        })
    }

    /// Retires the cell of a dropped state object (D7 of `owned-state`): invalidates and
    /// forgets its readers and marks it retired; with no composition live on the thread, frees
    /// it at once (or when the hold on it ends), else records it for the end of the next pass.
    /// A handle of a cell already gone does nothing. Program code may run (the value's drop).
    pub fn retire(handle: Handle) {
        let freed = with_state(|state| {
            let cell = state.cells.get_mut(handle)?;
            if cell.retired {
                return None;
            }
            cell.retired = true;
            let version = state.fresh_version();
            let cell = state.cells.get_mut(handle).unwrap();
            cell.version = version;
            // A reader that ran in the pass whose releases retire the cell read the states it
            // needs then; any other reader showed the retired state (D7 of `owned-state`).
            let releasing = state.releasing_pass;
            let mut readers = std::mem::take(&mut cell.readers);
            for reader in readers.scopes() {
                let current = releasing != 0
                    && state
                        .groups
                        .get(*reader)
                        .is_some_and(|group| group.visited == releasing);
                if !current {
                    invalidate_in(state, *reader);
                }
            }
            readers.clear();
            if let Some(cell) = state.cells.get_mut(handle) {
                cell.readers = readers;
            }
            if state.live_compositions > 0 {
                state.retired.push_back((handle, state.pass_counter));
                return None;
            }
            take_if_free(state, handle)
        });
        if let Some(cell) = freed {
            // SAFETY: the cell was removed from the arena and holds its value.
            unsafe { destroy(cell, false) }
        }
    }

    /// Frees the cells retired before the pass `identifier` started, at its end, after its
    /// releases (D7 of `owned-state`). Retirements are recorded in pass order, so the due ones
    /// are a prefix: the cost is their number.
    pub fn free_retired_before(identifier: u64) {
        loop {
            let freed = with_state(|state| {
                let &(handle, retired_at) = state.retired.front()?;
                if retired_at >= identifier {
                    return None;
                }
                state.retired.pop_front();
                Some(take_if_free(state, handle))
            });
            match freed {
                None => break,
                // SAFETY: the cell was removed from the arena and holds its value.
                Some(Some(cell)) => unsafe { destroy(cell, false) },
                Some(None) => {}
            }
        }
    }

    /// Frees every retired cell: when the last composition of the thread is disposed, and at
    /// exit under the allocation check. A freed value may retire more cells, which are freed
    /// at once when no composition is live.
    pub fn free_all_retired() {
        loop {
            let freed = with_state(|state| {
                let (handle, _) = state.retired.pop_front()?;
                Some(take_if_free(state, handle))
            });
            match freed {
                None => break,
                // SAFETY: the cell was removed from the arena and holds its value.
                Some(Some(cell)) => unsafe { destroy(cell, false) },
                Some(None) => {}
            }
        }
    }

    /// Whether `handle` names a live cell that is not retired: what a weak reference's `get()`
    /// tests (D11 of `owned-state`).
    pub fn upgrade(handle: Handle) -> bool {
        with_state(|state| {
            state
                .cells
                .get(handle)
                .is_some_and(|cell| !cell.retired && !cell.released)
        })
    }

    /// The address of the cell's value, which a read copies out of or a composable context
    /// borrows until its body ends; during a pass the current scope becomes a reader (D9).
    pub fn read(handle: Handle, record: *const SlotType) -> StateResult<*const u8> {
        with_state(|state| {
            let cell = cell_mut(state, handle, record)?;
            if matches!(cell.flag, Flag::Empty | Flag::Exclusive) {
                return Err(StateError::Borrowed);
            }
            let value = cell.value as *const u8;
            register_reader(state, handle);
            Ok(value)
        })
    }

    /// The cell's version (D3 of `strong-skipping`), validating the handle and record as
    /// [`States::read`] does; it registers no reader. During a pass no state is written, so a
    /// version read in a pass holds until the pass ends.
    pub fn version(handle: Handle, record: *const SlotType) -> StateResult<u64> {
        with_state(|state| {
            let cell = cell_mut(state, handle, record)?;
            if matches!(cell.flag, Flag::Empty | Flag::Exclusive) {
                return Err(StateError::Borrowed);
            }
            Ok(cell.version)
        })
    }

    /// Replaces the cell's value with the value at `value`, moved in (D10): an equal value is
    /// dropped and invalidates nothing; otherwise the old value is dropped and the readers are
    /// invalidated.
    ///
    /// # Safety
    ///
    /// `record` must be a valid record and `value` a value of it that the caller gives up.
    pub unsafe fn write(
        handle: Handle,
        record: *const SlotType,
        value: *mut u8,
    ) -> StateResult<()> {
        let (old, equals, drop) = with_state(|state| {
            check_writable(state)?;
            let cell = cell_mut(state, handle, record)?;
            if cell.flag != Flag::Free {
                return Err(StateError::Borrowed);
            }
            let record = &*cell.record;
            Ok((cell.value, record.equals, record.drop))
        })?;
        // The comparison and the drops are program code, run outside the state; the cell is
        // held exclusively meanwhile, so nothing reaches it.
        with_state(|state| state.cells.get_mut(handle).unwrap().flag = Flag::Exclusive);
        if let Some(equals) = equals {
            if equals(old, value) != 0 {
                if let Some(drop) = drop {
                    drop(value);
                }
                Self::end_hold(handle, false);
                return Ok(());
            }
        }
        if let Some(drop) = drop {
            drop(old);
        }
        std::ptr::copy_nonoverlapping(value, old, (*record).size);
        with_state(|state| {
            let version = state.fresh_version();
            if let Some(cell) = state.cells.get_mut(handle) {
                cell.version = version;
            }
        });
        Self::end_hold(handle, true);
        Ok(())
    }

    /// Takes a shared guard on the cell, giving the value's address; during a pass the current
    /// scope becomes a reader.
    pub fn begin_read(handle: Handle, record: *const SlotType) -> StateResult<*const u8> {
        with_state(|state| {
            let cell = cell_mut(state, handle, record)?;
            cell.flag = match cell.flag {
                Flag::Free => Flag::Shared(1),
                Flag::Shared(count) => Flag::Shared(count + 1),
                Flag::Empty | Flag::Exclusive => return Err(StateError::Borrowed),
            };
            let value = cell.value as *const u8;
            register_reader(state, handle);
            Ok(value)
        })
    }

    /// Ends a shared guard of [`States::begin_read`], freeing a cell whose free came due
    /// meanwhile once its last hold ends.
    pub fn end_read(handle: Handle) {
        let freed = with_state(|state| {
            let cell = state.cells.get_mut(handle)?;
            cell.flag = match cell.flag {
                Flag::Shared(count) if count > 1 => Flag::Shared(count - 1),
                _ => Flag::Free,
            };
            if cell.released && cell.flag == Flag::Free {
                return state.cells.remove(handle);
            }
            None
        });
        if let Some(cell) = freed {
            // SAFETY: the cell was removed from the arena and holds its value.
            unsafe { destroy(cell, false) }
        }
    }

    /// Moves the cell's value to `destination` for an `update` transform; the cell holds
    /// nothing until [`States::put`].
    ///
    /// # Safety
    ///
    /// `destination` must be writable memory for a value of the record.
    pub unsafe fn take(
        handle: Handle,
        record: *const SlotType,
        destination: *mut u8,
    ) -> StateResult<()> {
        with_state(|state| {
            check_writable(state)?;
            let cell = cell_mut(state, handle, record)?;
            if cell.flag != Flag::Free {
                return Err(StateError::Borrowed);
            }
            cell.flag = Flag::Empty;
            std::ptr::copy_nonoverlapping(cell.value, destination, (*cell.record).size);
            Ok(())
        })
    }

    /// Moves the value at `value`, the result of an `update` transform, into the cell its
    /// [`States::take`] emptied, and invalidates the readers; into a cell whose free came due
    /// meanwhile, the value is dropped and the cell freed.
    ///
    /// # Safety
    ///
    /// `record` must be a valid record and `value` a value of it that the caller gives up.
    pub unsafe fn put(handle: Handle, record: *const SlotType, value: *mut u8) -> StateResult<()> {
        let freed = with_state(|state| -> StateResult<Option<Cell>> {
            let Some(cell) = state.cells.get_mut(handle) else {
                return Err(StateError::Stale);
            };
            if cell.flag != Flag::Empty || !(*cell.record).matches(&*record) {
                return Err(StateError::Stale);
            }
            cell.flag = Flag::Free;
            if cell.released {
                return Ok(state.cells.remove(handle));
            }
            std::ptr::copy_nonoverlapping(value, cell.value, (*cell.record).size);
            let version = state.fresh_version();
            state.cells.get_mut(handle).unwrap().version = version;
            invalidate_readers(state, handle);
            Ok(None)
        })?;
        if let Some(cell) = freed {
            if let Some(drop) = (*record).drop {
                drop(value);
            }
            destroy(cell, true);
        }
        Ok(())
    }

    /// Takes an exclusive guard on the cell, giving the value's address.
    pub fn begin_edit(handle: Handle, record: *const SlotType) -> StateResult<*mut u8> {
        with_state(|state| {
            check_writable(state)?;
            let cell = cell_mut(state, handle, record)?;
            if cell.flag != Flag::Free {
                return Err(StateError::Borrowed);
            }
            cell.flag = Flag::Exclusive;
            let value = cell.value;
            // What the guard's holder writes cannot be compared, so the version changes now.
            let version = state.fresh_version();
            state.cells.get_mut(handle).unwrap().version = version;
            Ok(value)
        })
    }

    /// Ends the exclusive guard of [`States::begin_edit`] and invalidates the readers, or frees
    /// a cell whose free came due meanwhile.
    pub fn end_edit(handle: Handle) {
        Self::end_hold(handle, true)
    }

    /// Ends an exclusive hold of the cell, invalidating its readers when `changed`, or freeing
    /// it when its free came due meanwhile.
    fn end_hold(handle: Handle, changed: bool) {
        let freed = with_state(|state| {
            let cell = state.cells.get_mut(handle)?;
            cell.flag = Flag::Free;
            if cell.released {
                return state.cells.remove(handle);
            }
            if changed {
                invalidate_readers(state, handle);
            }
            None
        });
        if let Some(cell) = freed {
            // SAFETY: the cell was removed from the arena and holds its value.
            unsafe { destroy(cell, false) }
        }
    }

    /// Opens the program scope of a top-level initializer (D11 of `state-handles`), a creation
    /// window, giving whether it opened one: it does not while compose is unused, since no pass
    /// runs.
    pub fn begin_program() -> bool {
        if !IN_USE.load(Ordering::Relaxed) {
            return false;
        }
        with_state(|state| state.creation_windows += 1);
        true
    }

    /// Closes the program scope [`States::begin_program`] opened, when it opened one.
    pub fn end_program(opened: bool) {
        if opened {
            with_state(|state| state.creation_windows -= 1);
        }
    }

    /// The live cells of this thread.
    pub fn live_cells() -> usize {
        with_state(|state| state.cells.len())
    }

    /// Records `error` for the next `prism_compose_state_error`.
    pub fn record_error(error: StateError) {
        with_state(|state| state.state_error = Some(error))
    }

    /// The error the last failed operation recorded.
    pub fn recorded_error() -> Option<StateError> {
        with_state(|state| state.state_error)
    }
}

/// Frees the retired cells, under the allocation check, then gives the report of compositions
/// still live: the finish report `prism-codegen` installs (D7 of `owned-state`). A retired cell
/// may hold a composition, whose drop disposes it, so the cells go first.
pub fn finish_report() -> Option<String> {
    States::free_all_retired();
    crate::thread::live_compositions_report()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_are_registered_once_and_pruned_before_growing() {
        let mut readers = Readers::default();
        let scopes: Vec<Handle> = (1..=20).map(|index| Handle::new(index, 1)).collect();
        for _ in 0..3 {
            for scope in &scopes {
                readers.register(*scope, |_| true);
            }
        }
        assert_eq!(readers.scopes(), &scopes[..], "each scope once, in order");
        assert!(readers.set.is_some(), "a set beyond eight readers");
        // The scopes that left their composition are removed before the vector grows.
        let capacity = readers.list.capacity();
        let live = |scope: Handle| scope.index() > 15;
        for index in 0..(capacity - readers.list.len() + 1) as u32 {
            readers.register(Handle::new(100 + index, 1), live);
        }
        assert!(readers.scopes().iter().all(|scope| scope.index() > 15));
        assert!(!readers.contains(Handle::new(3, 1)));
        readers.clear();
        assert!(readers.scopes().is_empty());
    }
}
