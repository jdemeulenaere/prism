//! compose: "A steady-state pass allocates nothing" (D16 of `compose-runtime`), and "Steady-state
//! reads allocate nothing" (D9 of `state-handles`). A counting
//! global allocator counts the allocations and frees of this thread while a composition of
//! 1,000 restart groups, each holding a word slot, runs passes that recompose the root, with
//! every child skipping, and one invalid leaf scope. Its content and restart functions have
//! static descriptors and inline environments, as escaping closures over one `Int` have, so
//! only the runtime's own allocations are counted.

use prism_compose::test_support::{descriptor, inline_function, inline_word, slot_type};
use prism_compose::{invalidate, Composer, Composition, GroupKind, Handle, NodeTree, States};
use prism_runtime::Descriptor;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static EVENTS: Cell<u64> = const { Cell::new(0) };
    static CHILD: Cell<Option<&'static Descriptor>> = const { Cell::new(None) };
    static LEAF: Cell<u64> = const { Cell::new(0) };
    static READER: Cell<Option<&'static Descriptor>> = const { Cell::new(None) };
    static STATE: Cell<(u64, usize)> = const { Cell::new((0, 0)) };
}

fn count() {
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            let _ = EVENTS.try_with(|events| events.set(events.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        count();
        System.dealloc(pointer, layout)
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count();
        System.realloc(pointer, layout, size)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const CHILDREN: u64 = 1000;

/// One child: a restart group holding a word slot, skipping when it may.
fn child(k: Composer, index: u64) {
    k.start_group(GroupKind::Restart, 1, 0).unwrap();
    if index == CHILDREN - 1 {
        LEAF.with(|leaf| leaf.set(k.current_scope().unwrap().word()));
    }
    if k.skipping().unwrap() {
        k.skip_to_group_end().unwrap();
    } else if !k.has_word(0).unwrap() {
        k.set_word(0, index as i64).unwrap();
    } else {
        k.word().unwrap();
    }
    let restart = inline_function(CHILD.with(Cell::get).unwrap(), index);
    k.end_restart_group(restart).unwrap();
}

unsafe extern "C" fn child_code(environment: *mut u8, composer: u64) {
    child(Composer(composer), inline_word(environment));
}

unsafe extern "C" fn root_code(_: *mut u8, composer: u64) {
    for index in 0..CHILDREN {
        child(Composer(composer), index);
    }
}

#[test]
fn a_steady_state_pass_allocates_nothing() {
    CHILD.with(|child| child.set(Some(descriptor(child_code))));
    let root = descriptor(root_code);
    let composition =
        Composition::start(Box::new(NodeTree::new()), inline_function(root, 0)).unwrap();
    let root_scope = Composition::root_scope(composition).unwrap();
    let leaf = Handle(LEAF.with(Cell::get));
    let mut counts = Vec::with_capacity(100);
    for _ in 0..100 {
        EVENTS.with(|events| events.set(0));
        COUNTING.with(|counting| counting.set(true));
        invalidate(root_scope);
        invalidate(leaf);
        let ran = Composition::recompose(composition).unwrap();
        COUNTING.with(|counting| counting.set(false));
        assert!(ran);
        counts.push(EVENTS.with(Cell::get));
    }
    assert!(
        counts[2..].iter().all(|count| *count == 0),
        "allocations and frees per pass: {:?}",
        counts
    );
    // The allocator does count this thread's allocations.
    EVENTS.with(|events| events.set(0));
    COUNTING.with(|counting| counting.set(true));
    drop(std::hint::black_box(Box::new(1)));
    COUNTING.with(|counting| counting.set(false));
    assert_eq!(EVENTS.with(Cell::get), 2);
    Composition::dispose(composition).unwrap();
}

/// One reader: a restart group that reads the state each time it is composed.
fn reader(k: Composer, index: u64) {
    k.start_group(GroupKind::Restart, 2, 0).unwrap();
    let (handle, record) = STATE.with(Cell::get);
    let record = record as *const prism_compose::SlotType;
    let value = States::read(Handle(handle), record).unwrap();
    std::hint::black_box(unsafe { *(value as *const i64) });
    let restart = inline_function(READER.with(Cell::get).unwrap(), index);
    k.end_restart_group(restart).unwrap();
}

unsafe extern "C" fn reader_code(environment: *mut u8, composer: u64) {
    reader(Composer(composer), inline_word(environment));
}

unsafe extern "C" fn readers_code(_: *mut u8, composer: u64) {
    for index in 0..CHILDREN {
        reader(Composer(composer), index);
    }
}

/// compose: "Steady-state reads allocate nothing": 1,000 restart scopes that each read one
/// state are recomposed 100 times without a write; once each scope is registered, a read
/// allocates nothing.
#[test]
fn steady_state_reads_allocate_nothing() {
    READER.with(|reader| reader.set(Some(descriptor(reader_code))));
    let record = slot_type::<i64>("Int");
    let mut value = 7i64;
    let handle = unsafe { States::create(record, (&mut value as *mut i64).cast()) }.unwrap();
    STATE.with(|state| state.set((handle.word(), record as *const _ as usize)));
    let root = descriptor(readers_code);
    let composition =
        Composition::start(Box::new(NodeTree::new()), inline_function(root, 0)).unwrap();
    let root_scope = Composition::root_scope(composition).unwrap();
    let mut counts = Vec::with_capacity(100);
    for _ in 0..100 {
        EVENTS.with(|events| events.set(0));
        COUNTING.with(|counting| counting.set(true));
        invalidate(root_scope);
        let ran = Composition::recompose(composition).unwrap();
        COUNTING.with(|counting| counting.set(false));
        assert!(ran);
        counts.push(EVENTS.with(Cell::get));
    }
    assert!(
        counts[2..].iter().all(|count| *count == 0),
        "allocations and frees per pass: {:?}",
        counts
    );
    Composition::dispose(composition).unwrap();
    States::release(&[handle]);
}
