//! The scenario "Allocation-counting test" of the `ui` spec (design D19 of `ui-toolkit`): once
//! its buffers have grown, an iteration of the frame loop whose handler, pass, layout and
//! display list create nothing new allocates nothing, including the registration, comparison
//! and release of a handler that owns one `Copy` word.
//!
//! A counting global allocator counts the allocations of the test's own thread. The app is
//! driven from Rust: each iteration clicks the button (its handler writes nothing), invalidates
//! the root scope so that a pass follows, which registers the button's clickable handler again,
//! finds it equal to the one the node holds and releases it at the end of the pass, then lays
//! out (a clean tree) and rebuilds the display list.

use prism_compose::test_support::content;
use prism_compose::{invalidate, Composition};
use prism_ui::handlers::HandlerKind;
use prism_ui::host::{Host, HostEvent};
use prism_ui::modifier::ModifierMirror;
use prism_ui::properties::{Role, Shape};
use prism_ui::test_support::*;
use prism_ui::text::FixedAdvanceMetrics;
use prism_ui::{setters, App, NodeId};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn count() {
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count();
        System.realloc(pointer, layout, size)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

struct NoEvents;

impl Host for NoEvents {
    fn size(&self) -> (i64, i64) {
        (400, 300)
    }

    fn next_event(&mut self) -> Option<HostEvent> {
        None
    }

    fn present(&mut self, _list: &prism_ui::draw::DisplayList) {}
}

#[test]
fn steady_state_iterations_allocate_nothing() {
    let content = content(|k| {
        let column = NodeId(k.start_node_group(1, 1).unwrap().0);
        setters::set_modifier(column, &ModifierMirror::encode(&[padding(16)])).unwrap();
        setters::set_arrangement(column, 8, 8).unwrap();
        let text = NodeId(k.start_node_group(2, 4).unwrap().0);
        setters::set_text(text, "Count: 0").unwrap();
        k.end_group().unwrap();
        let button = NodeId(k.start_node_group(3, 3).unwrap().0);
        let handler = setters::register(HandlerKind::Click, inline_handler(1)).unwrap();
        let chain = ModifierMirror::encode(&[
            background(0xFF67_50A4, Shape::new(0, 50)),
            clickable(handler, true, Some(Role::Button)),
        ]);
        setters::set_modifier(button, &chain).unwrap();
        setters::set_content_color(button, 0xFFFF_FFFF).unwrap();
        let row = NodeId(k.start_node_group(4, 2).unwrap().0);
        let chain = ModifierMirror::encode(&[default_min_size(58, 40), padding_each(24, 8, 24, 8)]);
        setters::set_modifier(row, &chain).unwrap();
        let label = NodeId(k.start_node_group(5, 4).unwrap().0);
        setters::set_text(label, "+").unwrap();
        k.end_group().unwrap();
        k.end_group().unwrap();
        k.end_group().unwrap();
        k.end_group().unwrap();
    });
    let mut app = App::start(Box::new(NoEvents), Box::new(FixedAdvanceMetrics), content).unwrap();
    app.settle().unwrap();
    let root_scope = Composition::root_scope(app.composition()).unwrap();
    let mut counts = Vec::with_capacity(100);
    let (calls, _) = inline_calls();
    COUNTING.with(|counting| counting.set(true));
    for _ in 0..100 {
        app.handle(HostEvent::Press(45, 64)).unwrap();
        app.handle(HostEvent::Release(45, 64)).unwrap();
        invalidate(root_scope);
        app.settle().unwrap();
        app.build_display_list(true);
        counts.push(ALLOCATIONS.with(Cell::get));
    }
    // The counter sees this thread's allocations.
    let before = ALLOCATIONS.with(Cell::get);
    drop(std::hint::black_box(Vec::<u8>::with_capacity(8)));
    assert_eq!(ALLOCATIONS.with(Cell::get), before + 1);
    COUNTING.with(|counting| counting.set(false));
    assert_eq!(
        inline_calls().0,
        calls + 100,
        "every click called the handler"
    );
    assert_eq!(
        app.counters.passes, 101,
        "each click was followed by a pass"
    );
    assert_eq!(app.display_list().commands.len(), 4);
    assert_eq!(
        counts[99],
        counts[0],
        "allocations after the first iteration: {:?}",
        &counts[..10]
    );
    app.finish();
}
