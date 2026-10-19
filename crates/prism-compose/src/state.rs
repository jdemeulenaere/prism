//! State cells (D6, D8 to D12, D14 and D15 of `state-handles`).
//!
//! A `State<T>` or `MutableState<T>` is one word, a generational [`Handle`] of a [`Cell`] in
//! the thread's cell arena. A cell holds its value's slot type record, its value in a block of
//! exactly the record's size and alignment that never moves while the cell lives (so a borrow
//! of it taken during a pass stays valid however many cells the pass creates), a borrow flag,
//! the scopes that read it, and nothing about its owner but whether it was released.
//!
//! - **Validation** (D8, D15). Every access validates the handle's index and generation and
//!   compares the record it is given with the cell's, so a handle can never reach a freed cell
//!   or a cell of another layout; a failure is the runtime error `stale state handle`.
//! - **Borrow flags** (D6). A cell is free, shared by `read` closures (a count), empty while
//!   `update` holds its value, or exclusive while `edit` holds it or a write drops the old
//!   value. A conflicting access is `state accessed while borrowed`.
//! - **Readers** (D9). A read during a pass registers the pass's current scope once per cell:
//!   a vector with a linear membership test, backed by a hash set beyond eight scopes, pruned
//!   of scopes that left their composition before it grows. A write invalidates them and
//!   forgets them.
//! - **Equal writes** (D10). A `value =` write compares the new value with the current one
//!   through the record's equality entry; an equal one is dropped, and nothing is invalidated.
//! - **Creation owners** (D11, D12). A stack of who owns the cells created now: a `remember`
//!   window, whose cells are attached to the slot it writes, a composition, for the code that
//!   runs as one of its handlers or side effects, or the program, for top-level initializers.
//!   With an empty stack, a cell created outside a pass belongs to the program and one created
//!   during a pass is `state created during composition outside remember`.
//! - **Release** (D12). Freeing a cell drops its value through its record and frees its block,
//!   outside the thread's state; a cell borrowed or emptied when its owner releases it becomes
//!   stale at once and is freed when the borrowing call ends.
//! - **Errors** (D14). An entry that fails records the error's kind in the thread's state and
//!   returns a failure result; generated code then calls `prism_compose_state_error` with the
//!   access's location.

use crate::composer::invalidate_in;
use crate::handle::Handle;
use crate::slot::{allocate_block, free_block, SlotType};
use crate::thread::{with_state, State};
use prism_runtime::PanicKind;
use std::collections::HashSet;

/// How many readers a cell tests linearly before it keeps a hash set of them too.
const LINEAR_READERS: usize = 8;

/// How a cell is held (D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    /// Nothing holds the cell.
    Free,
    /// `count` `read` closures borrow its value shared.
    Shared(u32),
    /// `update` moved its value out.
    Empty,
    /// `edit` borrows its value mutably, or a write drops its old value.
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
    /// Whether the cell's owner released it while it was held: it is stale from then on, and
    /// freed when the borrowing call ends (D12).
    pub released: bool,
}

/// Who owns the cells created now (D11 and D12), innermost last.
#[derive(Debug)]
pub enum CreationOwner {
    /// A `remember` calculation: the cells created while it runs, attached to its slot by
    /// `end remember`.
    Remember(Vec<Handle>),
    /// A handler or side effect of the composition.
    Composition(Handle),
    /// A top-level value's initializer.
    Program,
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

/// Registers the active pass's current scope as a reader of `handle` (D9).
fn register_reader(state: &mut State, handle: Handle) {
    if state.active.is_none() {
        return;
    }
    let Some(scope) = state.frames.last().map(|frame| frame.scope) else {
        return;
    };
    let State { cells, groups, .. } = state;
    if let Some(cell) = cells.get_mut(handle) {
        cell.readers
            .register(scope, |reader| groups.contains(reader));
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
    /// Creates a cell of `record` holding the value at `value`, moved in, owned by the
    /// innermost creation owner (D11, D12).
    ///
    /// # Safety
    ///
    /// `record` must be a valid record and `value` a value of it that the caller gives up.
    pub unsafe fn create(record: *const SlotType, value: *mut u8) -> StateResult<Handle> {
        with_state(|state| {
            if state.creation_owners.is_empty() && state.active.is_some() {
                return Err(StateError::CreatedDuringComposition);
            }
            let block = allocate_block(&*record);
            std::ptr::copy_nonoverlapping(value, block, (*record).size);
            let handle = state.cells.insert(Cell {
                record,
                value: block,
                flag: Flag::Free,
                readers: Readers::default(),
                released: false,
            });
            match state.creation_owners.last_mut() {
                Some(CreationOwner::Remember(cells)) => cells.push(handle),
                Some(CreationOwner::Composition(composition)) => {
                    let composition = *composition;
                    match state.compositions.get_mut(composition) {
                        Some(record) if !record.disposed => record.cells.push(handle),
                        _ => state.program_cells.push(handle),
                    }
                }
                Some(CreationOwner::Program) | None => state.program_cells.push(handle),
            }
            Ok(handle)
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
        Self::end_hold(handle, true);
        Ok(())
    }

    /// Borrows the cell shared for a `read` closure, giving the value's address; during a pass
    /// the current scope becomes a reader.
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

    /// Ends a shared borrow of [`States::begin_read`], freeing a cell released meanwhile once
    /// its last borrow ends.
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

    /// Moves the cell's value to `destination` for an `update` closure; the cell holds nothing
    /// until [`States::put`].
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

    /// Moves the value at `value`, the result of an `update` closure, into the cell its
    /// [`States::take`] emptied, and invalidates the readers; into a cell released meanwhile,
    /// the value is dropped and the cell freed.
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

    /// Borrows the cell mutably for an `edit` closure, giving the value's address.
    pub fn begin_edit(handle: Handle, record: *const SlotType) -> StateResult<*mut u8> {
        with_state(|state| {
            check_writable(state)?;
            let cell = cell_mut(state, handle, record)?;
            if cell.flag != Flag::Free {
                return Err(StateError::Borrowed);
            }
            cell.flag = Flag::Exclusive;
            Ok(cell.value)
        })
    }

    /// Ends the mutable borrow of [`States::begin_edit`] and invalidates the readers, or frees
    /// a cell released meanwhile.
    pub fn end_edit(handle: Handle) {
        Self::end_hold(handle, true)
    }

    /// Ends an exclusive hold of the cell, invalidating its readers when `changed`, or freeing
    /// it when its owner released it meanwhile.
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

    /// Releases the cells `handles`, in order: each is freed, its value dropped, or, when a
    /// closure member holds it, made stale and freed when that call ends (D12). Program code
    /// runs, so no borrow of the state may be held.
    pub fn release(handles: &[Handle]) {
        for &handle in handles {
            let freed = with_state(|state| {
                let cell = state.cells.get_mut(handle)?;
                if cell.flag != Flag::Free {
                    cell.released = true;
                    return None;
                }
                state.cells.remove(handle)
            });
            if let Some(cell) = freed {
                // SAFETY: the cell was removed from the arena and holds its value.
                unsafe { destroy(cell, false) }
            }
        }
    }

    /// The type name of the cell `handle`, for dumps.
    pub fn type_name(state: &State, handle: Handle) -> Option<String> {
        let cell = state.cells.get(handle)?;
        // SAFETY: records are static data.
        Some(unsafe { (*cell.record).name() }.to_string())
    }

    /// Pushes `owner` as the innermost creation owner.
    pub fn push_owner(owner: CreationOwner) {
        with_state(|state| state.creation_owners.push(owner))
    }

    /// Pops the innermost creation owner.
    pub fn pop_owner() -> Option<CreationOwner> {
        with_state(|state| state.creation_owners.pop())
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

/// Frees the cells the program owns, under the allocation check, then gives the report of
/// compositions still live: the finish report `prism-codegen` installs (D12). A program cell
/// may hold a composition, whose drop disposes it, so the cells go first.
pub fn finish_report() -> Option<String> {
    loop {
        let cells = with_state(|state| std::mem::take(&mut state.program_cells));
        if cells.is_empty() {
            break;
        }
        States::release(&cells);
    }
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
