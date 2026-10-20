//! Rust tests of state cells (`state-handles`, `owned-state`): what a Prism program cannot
//! observe (flag transitions, deduplicated readers, retirement and frees, a free due while a
//! guard holds the cell, another thread) and the protocol of cells, remember windows and side
//! effects driven from Rust content.

use crate::applier::NodeTree;
use crate::composer::Composer;
use crate::composition::Composition;
use crate::group::GroupKind;
use crate::handle::Handle;
use crate::slot::SlotType;
use crate::state::{finish_report, StateError, States};
use crate::test_support::{content, effect, slot_type};
use crate::thread::with_state;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    static EVENTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn events() -> Vec<String> {
    EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()))
}

fn event(what: impl Into<String>) {
    EVENTS.with(|events| events.borrow_mut().push(what.into()));
}

/// A value whose drop is recorded.
#[derive(Debug, PartialEq)]
struct Tracked(&'static str);

impl Drop for Tracked {
    fn drop(&mut self) {
        event(format!("drop {}", self.0));
    }
}

fn create<T>(record: &'static SlotType, value: T) -> Result<Handle, StateError> {
    let mut value = std::mem::ManuallyDrop::new(value);
    unsafe { States::create(record, (&mut *value as *mut T).cast()) }
}

fn write<T>(handle: Handle, record: &'static SlotType, value: T) -> Result<(), StateError> {
    let mut value = std::mem::ManuallyDrop::new(value);
    unsafe { States::write(handle, record, (&mut *value as *mut T).cast()) }
}

fn read<T: Copy>(handle: Handle, record: &'static SlotType) -> Result<T, StateError> {
    States::read(handle, record).map(|value| unsafe { *(value as *const T) })
}

/// Frees what the tests left retired on this thread.
fn reset() {
    finish_report();
    events();
}

/// A state object as generated code holds one: its handle, retired when it is dropped.
#[derive(Debug, PartialEq)]
struct StateObject(Handle);

impl Drop for StateObject {
    fn drop(&mut self) {
        States::retire(self.0);
    }
}

#[test]
fn flags_guard_conflicting_accesses() {
    let int = slot_type::<i64>("Int");
    let cell = create(int, 5i64).unwrap();
    // Shared borrows count, and conflict with writes, takes and edits.
    States::begin_read(cell, int).unwrap();
    States::begin_read(cell, int).unwrap();
    assert_eq!(
        read::<i64>(cell, int),
        Ok(5),
        "a read beside a shared borrow"
    );
    assert_eq!(write(cell, int, 6i64), Err(StateError::Borrowed));
    assert_eq!(States::begin_edit(cell, int), Err(StateError::Borrowed));
    States::end_read(cell);
    assert_eq!(
        write(cell, int, 6i64),
        Err(StateError::Borrowed),
        "one borrow is left"
    );
    States::end_read(cell);
    // An emptied cell can be neither read nor written until it is put back.
    let mut taken = 0i64;
    unsafe { States::take(cell, int, (&mut taken as *mut i64).cast()) }.unwrap();
    assert_eq!(taken, 5);
    assert_eq!(read::<i64>(cell, int), Err(StateError::Borrowed));
    assert_eq!(States::begin_read(cell, int), Err(StateError::Borrowed));
    let mut doubled = taken * 2;
    unsafe { States::put(cell, int, (&mut doubled as *mut i64).cast()) }.unwrap();
    assert_eq!(read::<i64>(cell, int), Ok(10));
    // An edited cell can be neither read nor borrowed.
    let value = States::begin_edit(cell, int).unwrap();
    assert_eq!(read::<i64>(cell, int), Err(StateError::Borrowed));
    assert_eq!(States::begin_read(cell, int), Err(StateError::Borrowed));
    unsafe { *(value as *mut i64) += 1 };
    States::end_edit(cell);
    assert_eq!(read::<i64>(cell, int), Ok(11));
    write(cell, int, 12i64).unwrap();
    assert_eq!(read::<i64>(cell, int), Ok(12));
    reset();
}

#[test]
fn stale_handles_are_detected() {
    let int = slot_type::<i64>("Int");
    let text = slot_type::<Tracked>("Tracked");
    let cell = create(text, Tracked("a")).unwrap();
    // Another record is another type: the handle names no cell of it.
    assert_eq!(read::<i64>(cell, int), Err(StateError::Stale));
    // The word 0 and an index never used name nothing.
    assert_eq!(States::read(Handle(0), text).err(), Some(StateError::Stale));
    assert_eq!(
        States::read(Handle::new(9999, 1), text).err(),
        Some(StateError::Stale)
    );
    // With no composition live, a retired cell is freed at once.
    States::retire(cell);
    assert_eq!(events(), ["drop a"]);
    assert_eq!(States::read(cell, text).err(), Some(StateError::Stale));
    // The index is reused with a new generation: the old handle stays stale.
    let again = create(text, Tracked("b")).unwrap();
    assert_eq!(again.index(), cell.index());
    assert_eq!(States::read(cell, text).err(), Some(StateError::Stale));
    assert!(States::read(again, text).is_ok());
    // A cell whose free comes due while a guard holds it is stale at once, and freed when the
    // guard ends.
    States::begin_read(again, text).unwrap();
    States::retire(again);
    assert!(events().is_empty(), "not freed while borrowed");
    assert_eq!(States::read(again, text).err(), Some(StateError::Stale));
    States::end_read(again);
    assert_eq!(events(), ["drop b"]);
    reset();
}

#[test]
fn equal_writes_keep_the_old_value_and_drop_the_new_one() {
    let text = slot_type::<Tracked>("Tracked");
    let cell = create(text, Tracked("old")).unwrap();
    write(cell, text, Tracked("old")).unwrap();
    assert_eq!(events(), ["drop old"], "the new, equal value is dropped");
    write(cell, text, Tracked("new")).unwrap();
    assert_eq!(events(), ["drop old"], "the old value is dropped");
    let value = States::read(cell, text).unwrap();
    assert_eq!(unsafe { (*(value as *const Tracked)).0 }, "new");
    States::retire(cell);
    assert_eq!(events(), ["drop new"]);
    reset();
}

/// A composition whose content reads `cell` in a restart group of key 7, logging each run.
fn reader(cell: Handle, int: &'static SlotType, log: Rc<RefCell<Vec<i64>>>) -> Handle {
    let content = content(move |k: Composer| {
        k.start_group(GroupKind::Restart, 7, 0).unwrap();
        for _ in 0..3 {
            let value = read::<i64>(cell, int).unwrap();
            if log.borrow().len() < 10 {
                log.borrow_mut().push(value);
            }
        }
        let log = log.clone();
        k.end_restart_group(content_restart(cell, int, log))
            .unwrap();
    });
    Composition::start(Box::new(NodeTree::new()), content).unwrap()
}

/// The restart function of [`reader`]'s group: reads the cell again.
fn content_restart(
    cell: Handle,
    int: &'static SlotType,
    log: Rc<RefCell<Vec<i64>>>,
) -> prism_runtime::FunctionValue {
    content(move |k: Composer| {
        k.start_group(GroupKind::Restart, 7, 0).unwrap();
        log.borrow_mut().push(read::<i64>(cell, int).unwrap());
        let log = log.clone();
        k.end_restart_group(content_restart(cell, int, log))
            .unwrap();
    })
}

#[test]
fn reads_register_their_scope_once_and_writes_invalidate_it() {
    let int = slot_type::<i64>("Int");
    let cell = create(int, 1i64).unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let c = reader(cell, int, log.clone());
    assert_eq!(*log.borrow(), [1, 1, 1]);
    let readers = with_state(|state| state.cells.get(cell).unwrap().readers.scopes().len());
    assert_eq!(readers, 1, "three reads register the scope once");
    // A read outside a pass registers nothing; a write invalidates the reader and forgets it.
    read::<i64>(cell, int).unwrap();
    write(cell, int, 2i64).unwrap();
    let readers = with_state(|state| state.cells.get(cell).unwrap().readers.scopes().len());
    assert_eq!(readers, 0);
    assert!(Composition::recompose(c).unwrap(), "the reader recomposes");
    assert_eq!(*log.borrow(), [1, 1, 1, 2]);
    assert!(!Composition::recompose(c).unwrap());
    // An equal write invalidates nothing.
    write(cell, int, 2i64).unwrap();
    assert!(!Composition::recompose(c).unwrap());
    Composition::dispose(c).unwrap();
    // The reader left its composition: a write ignores it.
    write(cell, int, 3i64).unwrap();
    reset();
}

#[test]
fn writes_and_creations_during_a_pass_fail() {
    let int = slot_type::<i64>("Int");
    let cell = create(int, 1i64).unwrap();
    let results = Rc::new(RefCell::new(Vec::new()));
    let seen = results.clone();
    let c = Composition::start(
        Box::new(NodeTree::new()),
        content(move |_| {
            seen.borrow_mut().push(write(cell, int, 2i64).err());
            seen.borrow_mut().push(States::begin_edit(cell, int).err());
            seen.borrow_mut().push(create(int, 3i64).err());
        }),
    )
    .unwrap();
    assert_eq!(
        *results.borrow(),
        [
            Some(StateError::WrittenDuringComposition),
            Some(StateError::WrittenDuringComposition),
            Some(StateError::CreatedDuringComposition),
        ]
    );
    Composition::dispose(c).unwrap();
    reset();
}

/// Stores a state object in a typed slot at the cursor, as `remember` does on a miss, inside a
/// remember window, and gives its handle.
fn remember_cell<T>(
    k: Composer,
    object_record: &'static SlotType,
    record: &'static SlotType,
    value: T,
) -> Handle {
    let address = unsafe { k.slot(object_record).unwrap() };
    if !address.is_null() {
        return unsafe { (*(address as *const StateObject)).0 };
    }
    k.begin_remember().unwrap();
    let handle = create(record, value).unwrap();
    let mut object = std::mem::ManuallyDrop::new(StateObject(handle));
    unsafe { k.set_slot(object_record, (&mut *object as *mut StateObject).cast()) }.unwrap();
    k.end_remember().unwrap();
    handle
}

/// compose: "A retired cell outlives the pass that retired it": a remembered state removed
/// by a pass is retired at its end and freed at the end of the next pass, after its releases.
#[test]
fn a_retired_cell_outlives_the_pass_that_retired_it() {
    let object_record = slot_type::<StateObject>("MutableState<Tracked>");
    let tracked = slot_type::<Tracked>("Tracked");
    let wide = slot_type::<Tracked>("Slot");
    let show = Rc::new(Cell::new(true));
    let shown = show.clone();
    let kept = Rc::new(Cell::new(None));
    let keep = kept.clone();
    let c = Composition::start(
        Box::new(NodeTree::new()),
        content(move |k: Composer| {
            if shown.get() {
                k.start_group(GroupKind::Replaceable, 1, 0).unwrap();
                keep.set(Some(remember_cell(
                    k,
                    object_record,
                    tracked,
                    Tracked("cell"),
                )));
                let mut slot = std::mem::ManuallyDrop::new(Tracked("slot"));
                unsafe { k.set_slot(wide, (&mut *slot as *mut Tracked).cast()) }.unwrap();
                k.end_group().unwrap();
            }
        }),
    )
    .unwrap();
    let cell = kept.get().unwrap();
    let dump = Composition::dump(c).unwrap();
    assert!(dump.contains("slot MutableState<Tracked>\n"), "{}", dump);
    assert!(!dump.contains("cell"), "no cell lines: {}", dump);
    assert!(States::upgrade(cell));
    show.set(false);
    invalidate_root(c);
    Composition::recompose(c).unwrap();
    // Slots last to first; the state object's slot retires its cell, which stays.
    assert_eq!(events(), ["drop slot"]);
    event("mark 1");
    assert!(!States::upgrade(cell), "a weak reference finds it retired");
    assert!(
        States::read(cell, tracked).is_ok(),
        "its value is still valid"
    );
    invalidate_root(c);
    Composition::recompose(c).unwrap();
    event("mark 2");
    assert_eq!(events(), ["mark 1", "drop cell", "mark 2"]);
    assert_eq!(States::read(cell, tracked).err(), Some(StateError::Stale));
    Composition::dispose(c).unwrap();
    reset();
}

/// compose: "A state dropped with no composition is freed at once", and a retired cell left
/// when the last composition is disposed is freed then.
#[test]
fn a_state_dropped_with_no_composition_is_freed_at_once() {
    let tracked = slot_type::<Tracked>("Tracked");
    drop(StateObject(create(tracked, Tracked("alone")).unwrap()));
    event("mark");
    assert_eq!(events(), ["drop alone", "mark"]);
    let c = Composition::start(Box::new(NodeTree::new()), content(|_| {})).unwrap();
    drop(StateObject(create(tracked, Tracked("beside")).unwrap()));
    assert!(events().is_empty(), "kept while a composition is live");
    Composition::dispose(c).unwrap();
    assert_eq!(events(), ["drop beside"], "freed with the last composition");
    reset();
}

fn invalidate_root(c: Handle) {
    crate::invalidate(Composition::root_scope(c).unwrap());
}

/// compose: "Releasing a cell during an edit": a cell retired while a guard holds it, whose
/// free comes due at the end of a pass, is freed when the guard ends.
#[test]
fn releasing_a_cell_during_an_edit() {
    let tracked = slot_type::<Tracked>("Tracked");
    let c = Composition::start(Box::new(NodeTree::new()), content(|_| {})).unwrap();
    let cell = create(tracked, Tracked("v")).unwrap();
    let value = States::begin_read(cell, tracked).unwrap();
    States::retire(cell);
    invalidate_root(c);
    Composition::recompose(c).unwrap();
    assert!(events().is_empty(), "not freed while the guard holds it");
    assert_eq!(unsafe { (*(value as *const Tracked)).0 }, "v");
    States::end_read(cell);
    assert_eq!(events(), ["drop v"], "dropped once, when the guard ends");
    assert_eq!(States::read(cell, tracked).err(), Some(StateError::Stale));
    assert!(!States::upgrade(cell));
    Composition::dispose(c).unwrap();
    reset();
}

/// A retired cell's readers are invalidated and forgotten, and later reads register none.
#[test]
fn retiring_a_cell_invalidates_its_readers() {
    let int = slot_type::<i64>("Int");
    let cell = create(int, 1i64).unwrap();
    let log = Rc::new(RefCell::new(Vec::new()));
    let c = reader(cell, int, log.clone());
    States::retire(cell);
    assert!(Composition::recompose(c).unwrap(), "the reader recomposes");
    let readers = with_state(|state| state.cells.get(cell).map(|c| c.readers.scopes().len()));
    // The pass that recomposed the reader started after the retirement, so it freed the cell.
    assert_eq!(readers, None);
    Composition::dispose(c).unwrap();
    reset();
}

#[test]
fn a_handle_used_on_another_thread_is_stale() {
    let int = slot_type::<i64>("Int");
    let cell = create(int, 1i64).unwrap();
    let error = std::thread::spawn(move || read::<i64>(cell, int).err())
        .join()
        .unwrap();
    assert_eq!(error, Some(StateError::Stale));
    reset();
}

#[test]
fn side_effects_run_after_the_pass_and_its_releases() {
    let show = Rc::new(Cell::new(true));
    let shown = show.clone();
    let c = Composition::start(
        Box::new(NodeTree::new()),
        content(move |k: Composer| {
            event("compose");
            if shown.get() {
                k.start_group(GroupKind::Replaceable, 1, 0).unwrap();
                let mut slot = std::mem::ManuallyDrop::new(Tracked("removed"));
                let record = slot_type::<Tracked>("Tracked");
                unsafe { k.set_slot(record, (&mut *slot as *mut Tracked).cast()) }.unwrap();
                k.end_group().unwrap();
            }
            k.side_effect(effect(|| event("effect 1"))).unwrap();
            k.side_effect(effect(|| event("effect 2"))).unwrap();
        }),
    )
    .unwrap();
    assert_eq!(events(), ["compose", "effect 1", "effect 2"]);
    show.set(false);
    invalidate_root(c);
    Composition::recompose(c).unwrap();
    assert_eq!(
        events(),
        ["compose", "drop removed", "effect 1", "effect 2"],
        "effects run after the releases, before recompose returns"
    );
    Composition::dispose(c).unwrap();
    reset();
}

#[test]
fn a_side_effect_runs_as_a_handler_and_may_write() {
    let int = slot_type::<i64>("Int");
    let cell = create(int, 0i64).unwrap();
    let made = Rc::new(Cell::new(None));
    let keep = made.clone();
    let c = Composition::start(
        Box::new(NodeTree::new()),
        content(move |k: Composer| {
            let keep = keep.clone();
            k.side_effect(effect(move || {
                write(cell, int, 1i64).unwrap();
                keep.set(Some(create(int, 5i64).unwrap()));
            }))
            .unwrap();
        }),
    )
    .unwrap();
    assert_eq!(read::<i64>(cell, int), Ok(1));
    let made = made.get().unwrap();
    assert_eq!(read::<i64>(made, int), Ok(5));
    Composition::dispose(c).unwrap();
    assert_eq!(
        read::<i64>(made, int),
        Ok(5),
        "a state belongs to its state object, not to the composition"
    );
    States::retire(made);
    States::retire(cell);
    assert_eq!(read::<i64>(made, int), Err(StateError::Stale));
    reset();
}
