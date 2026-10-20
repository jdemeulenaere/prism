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
//!
//! The scenario "The statistics allocate nothing" of `reactive-app-verification`'s `ui` delta
//! drives an app through 100 frames of `App::run` with frame statistics on: a press and a
//! release whose handler writes nothing, then a frame, which builds and presents the display
//! list and ends a measured frame. Nothing is allocated after the first frame.

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

/// The tests switch the process-wide frame statistics, one at a time.
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The counter screen, whose button's handler writes nothing.
fn counter_screen() -> prism_runtime::FunctionValue {
    content(|k| {
        let column = node_group(k, 1, 1);
        setters::set_modifier(column, &ModifierMirror::encode(&[padding(16)])).unwrap();
        setters::set_arrangement(column, 8, 8).unwrap();
        let text = NodeId(k.start_node_group(2, 4).unwrap().0);
        setters::set_text(text, "Count: 0").unwrap();
        k.end_group().unwrap();
        let button = node_group(k, 3, 3);
        let handler = setters::register(HandlerKind::Click, inline_handler(1)).unwrap();
        let chain = ModifierMirror::encode(&[
            background(0xFF67_50A4, Shape::new(0, 50)),
            clickable(handler, true, Some(Role::Button)),
        ]);
        setters::set_modifier(button, &chain).unwrap();
        setters::set_content_color(button, 0xFFFF_FFFF).unwrap();
        let row = node_group(k, 4, 2);
        let chain = ModifierMirror::encode(&[default_min_size(58, 40), padding_each(24, 8, 24, 8)]);
        setters::set_modifier(row, &chain).unwrap();
        let label = NodeId(k.start_node_group(5, 4).unwrap().0);
        setters::set_text(label, "+").unwrap();
        k.end_group().unwrap();
        k.end_group().unwrap();
        k.end_group().unwrap();
        k.end_group().unwrap();
    })
}

#[test]
fn steady_state_iterations_allocate_nothing() {
    let _lock = lock();
    prism_ui::host::set_frame_statistics(None);
    let content = counter_screen();
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

/// A column of 20 rows, each a box holding a text, then a text, every container measured by
/// the test support's policies.
fn rows() -> prism_runtime::FunctionValue {
    content(|k| {
        node_group(k, 1, 1);
        for index in 0..20 {
            let key = 10 + index * 10;
            node_group(k, key, 2);
            let boxed = node_group(k, key + 1, 3);
            setters::set_modifier(boxed, &ModifierMirror::encode(&[padding(4)])).unwrap();
            let text = node_group(k, key + 2, 4);
            setters::set_text(text, "item").unwrap();
            k.end_group().unwrap();
            k.end_group().unwrap();
            let text = node_group(k, key + 3, 4);
            setters::set_text(text, "label").unwrap();
            k.end_group().unwrap();
            k.end_group().unwrap();
        }
        k.end_group().unwrap();
    })
}

/// The scenario "Resizing an app allocates nothing": 100 resizes alternating between two
/// window widths, each followed by a layout of the whole tree, through the policies' calls and
/// their lent measurables, and a display list build.
#[test]
fn resizing_an_app_allocates_nothing() {
    let _lock = lock();
    prism_ui::host::set_frame_statistics(None);
    let mut app = App::start(Box::new(NoEvents), Box::new(FixedAdvanceMetrics), rows()).unwrap();
    app.settle().unwrap();
    let mut counts = Vec::with_capacity(100);
    let layouts = app.counters.layouts;
    COUNTING.with(|counting| counting.set(true));
    for index in 0..100 {
        let width = if index % 2 == 0 { 401 } else { 400 };
        app.handle(HostEvent::Resize(width, 300)).unwrap();
        app.settle().unwrap();
        app.build_display_list(true);
        counts.push(ALLOCATIONS.with(Cell::get));
    }
    COUNTING.with(|counting| counting.set(false));
    assert_eq!(app.counters.layouts, layouts + 100, "every resize laid out");
    assert_eq!(
        counts[99],
        counts[0],
        "allocations after the first iteration: {:?}",
        &counts[..10]
    );
    app.finish();
}

/// A column of 20 rows whose nodes carry every built-in layout step: a filled, padded row of a
/// sized box holding a text with a minimum width, a spacer of a width and an offset text.
fn padded_rows() -> prism_runtime::FunctionValue {
    content(|k| {
        node_group(k, 1, 1);
        for index in 0..20 {
            let key = 10 + index * 10;
            let row = node_group(k, key, 2);
            setters::set_modifier(
                row,
                &ModifierMirror::encode(&[fill_max(true, false, 0.5), padding_each(2, 1, 2, 1)]),
            )
            .unwrap();
            let boxed = node_group(k, key + 1, 3);
            setters::set_modifier(boxed, &ModifierMirror::encode(&[size(30, 20)])).unwrap();
            let text = node_group(k, key + 2, 4);
            setters::set_modifier(text, &ModifierMirror::encode(&[default_min_size(12, -1)]))
                .unwrap();
            setters::set_text(text, "item").unwrap();
            k.end_group().unwrap();
            k.end_group().unwrap();
            let spacer = node_group(k, key + 3, 5);
            setters::set_modifier(spacer, &ModifierMirror::encode(&[width(4)])).unwrap();
            k.end_group().unwrap();
            let text = node_group(k, key + 4, 4);
            let offset = prism_ui::modifier::Element::Offset { x: 3, y: 1 };
            setters::set_modifier(text, &ModifierMirror::encode(&[offset, padding(1)])).unwrap();
            setters::set_text(text, "label").unwrap();
            k.end_group().unwrap();
            k.end_group().unwrap();
        }
        k.end_group().unwrap();
    })
}

/// The scenario "Resizing padded rows allocates nothing" (`layout-in-prism`): 100 resizes
/// alternating between two window widths, each followed by a layout of the whole tree, through
/// the steps of every layout element and the policies, and a display list build.
#[test]
fn resizing_padded_rows_allocates_nothing() {
    let _lock = lock();
    prism_ui::host::set_frame_statistics(None);
    let mut app = App::start(
        Box::new(NoEvents),
        Box::new(FixedAdvanceMetrics),
        padded_rows(),
    )
    .unwrap();
    app.settle().unwrap();
    let mut counts = Vec::with_capacity(100);
    let layouts = app.counters.layouts;
    COUNTING.with(|counting| counting.set(true));
    for index in 0..100 {
        let width = if index % 2 == 0 { 401 } else { 400 };
        app.handle(HostEvent::Resize(width, 300)).unwrap();
        app.settle().unwrap();
        app.build_display_list(true);
        counts.push(ALLOCATIONS.with(Cell::get));
    }
    COUNTING.with(|counting| counting.set(false));
    assert_eq!(app.counters.layouts, layouts + 100, "every resize laid out");
    assert_eq!(
        counts[99],
        counts[0],
        "allocations after the first iteration: {:?}",
        &counts[..10]
    );
    app.finish();
}

/// A host giving 100 frames of a press, a release and a frame, which records this thread's
/// allocation count at each present into a buffer allocated beforehand.
struct Frames {
    events: std::vec::IntoIter<HostEvent>,
    counts: std::rc::Rc<std::cell::RefCell<Vec<u64>>>,
}

impl Host for Frames {
    fn size(&self) -> (i64, i64) {
        (400, 300)
    }

    fn next_event(&mut self) -> Option<HostEvent> {
        self.events.next()
    }

    fn present(&mut self, _list: &prism_ui::draw::DisplayList) {
        let count = ALLOCATIONS.with(Cell::get);
        self.counts.borrow_mut().push(count);
    }
}

#[test]
fn the_statistics_allocate_nothing() {
    let _lock = lock();
    let mut events = Vec::with_capacity(300);
    for _ in 0..100 {
        events.extend([
            HostEvent::Press(45, 64),
            HostEvent::Release(45, 64),
            HostEvent::Frame,
        ]);
    }
    let counts = std::rc::Rc::new(std::cell::RefCell::new(Vec::with_capacity(100)));
    let host = Frames {
        events: events.into_iter(),
        counts: counts.clone(),
    };
    prism_ui::host::set_frame_statistics(Some(std::time::Instant::now()));
    COUNTING.with(|counting| counting.set(true));
    let mut app = App::start(
        Box::new(host),
        Box::new(FixedAdvanceMetrics),
        counter_screen(),
    )
    .unwrap();
    app.run().unwrap();
    COUNTING.with(|counting| counting.set(false));
    prism_ui::host::set_frame_statistics(None);
    let statistics = app.frame_statistics().expect("frame statistics are on");
    let report = statistics.report(&[]);
    assert!(
        report.starts_with("frame stats: frames 100, events 200, passes 1, "),
        "{}",
        report
    );
    let counts = counts.borrow();
    assert_eq!(counts.len(), 100);
    assert_eq!(
        counts[99],
        counts[0],
        "allocations after the first frame: {:?}",
        &counts[..10]
    );
    app.finish();
}
