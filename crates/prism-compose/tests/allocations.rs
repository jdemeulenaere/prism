//! compose: "A steady-state pass allocates nothing" (D16 of `compose-runtime`). A counting
//! global allocator counts the allocations and frees of this thread while a composition of
//! 1,000 restart groups, each holding a word slot, runs passes that recompose the root, with
//! every child skipping, and one invalid leaf scope. Its content and restart functions have
//! static descriptors and inline environments, as escaping closures over one `Int` have, so
//! only the runtime's own allocations are counted.

use prism_compose::test_support::{descriptor, inline_function, inline_word};
use prism_compose::{invalidate, Composer, Composition, GroupKind, Handle, NodeTree};
use prism_runtime::Descriptor;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static EVENTS: Cell<u64> = const { Cell::new(0) };
    static CHILD: Cell<Option<&'static Descriptor>> = const { Cell::new(None) };
    static LEAF: Cell<u64> = const { Cell::new(0) };
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
