//! The scenario "Allocation-counting test" of the `desktop` spec (design D19 of
//! `desktop-window`): once its buffers have grown, a window frame allocates nothing, on either
//! thread.
//!
//! A counting global allocator counts the allocations of the test's own thread, which plays
//! both: it presents the counter screen's display list through a [`WindowHost`] (the program
//! thread's side), then swaps it out of the mailbox, rasterizes it at scale 2 and converts the
//! pixels into a surface buffer (the main thread's side). The mailbox and the main thread trade
//! their two lists at each frame, so each list grows once: the first two frames allocate, and
//! none after them may.

use prism_desktop::host::{connect, FrameSink};
use prism_desktop::test_support::counter_screen_list;
use prism_desktop::{Rasterizer, WindowHost};
use prism_ui::draw::DisplayList;
use prism_ui::Host;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

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

/// A proxy that only counts, as the event loop's would send one message.
#[derive(Default)]
struct CountingSink(AtomicU64);

impl FrameSink for CountingSink {
    fn frame_ready(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn close_window(&self) {}
}

#[test]
fn steady_state_frames_allocate_nothing() {
    let sink = Arc::new(CountingSink::default());
    let (connection, endpoint) = connect((400, 300), 2.0, sink.clone());
    let mut host = WindowHost::new(connection);
    let list = counter_screen_list();
    let mut rasterizer = Rasterizer::new();
    let (mut current, mut seen) = (DisplayList::new(), 0);
    let mut buffer = vec![0u32; 800 * 600];
    let mut frame = || {
        host.present(&list);
        assert!(endpoint.mailbox.take(&mut current, &mut seen));
        rasterizer.rasterize(&current, 2.0, 800, 600);
        rasterizer.copy_to_surface(&mut buffer);
    };
    COUNTING.with(|counting| counting.set(true));
    frame();
    frame();
    let growing = ALLOCATIONS.with(|count| count.replace(0));
    for _ in 0..100 {
        frame();
    }
    COUNTING.with(|counting| counting.set(false));
    let steady = ALLOCATIONS.with(Cell::get);
    assert!(growing > 0, "the first frames grow the buffers");
    assert_eq!(steady, 0, "allocations in 100 steady-state frames");
    assert_eq!(sink.0.load(Ordering::Relaxed), 102, "one message per frame");
    assert_eq!(
        rasterizer.statistics.glyphs, 9,
        "the counter screen's glyphs were drawn"
    );
}
