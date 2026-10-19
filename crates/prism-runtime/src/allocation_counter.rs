//! The counting allocator of `prism run --frame-stats` (D7 of `reactive-app-verification`).
//!
//! [`CountingAllocator`] is a `GlobalAlloc` over [`System`] that, once [`enable`] has been
//! called, adds one to the current thread's allocation or free count; a reallocation counts as
//! one of each. Only the `prism` binary declares it as its `#[global_allocator]`, so library
//! tests and other embedders are unaffected, and only `--frame-stats` enables it, before the
//! program thread starts. The frame loop reads [`thread_counts`] at frame boundaries: a frame's
//! allocations are the program thread's count at its end minus its count at its start, and its
//! live allocations are allocations minus frees since the thread started.
//!
//! While disabled, an allocation costs one relaxed atomic load. The counts are `const`
//! thread-locals of `Cell<u64>`, which need no lazy initialization and no destructor, so
//! counting never allocates and never recurses into the allocator, even on a thread that never
//! touched them before.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static FREES: Cell<u64> = const { Cell::new(0) };
}

/// The system allocator, counting each thread's allocations and frees once enabled.
#[derive(Debug, Default, Clone, Copy)]
pub struct CountingAllocator;

fn count(counter: &'static std::thread::LocalKey<Cell<u64>>) {
    if ENABLED.load(Ordering::Relaxed) {
        // A thread being torn down may no longer reach its counts; nothing is counted then.
        let _ = counter.try_with(|count| count.set(count.get() + 1));
    }
}

// SAFETY: every call is forwarded to `System` unchanged; counting only touches thread-local
// cells, which neither allocate nor unwind.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(&ALLOCATIONS);
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(&ALLOCATIONS);
        System.alloc_zeroed(layout)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        count(&FREES);
        System.dealloc(pointer, layout)
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(&ALLOCATIONS);
        count(&FREES);
        System.realloc(pointer, layout, new_size)
    }
}

/// Starts counting, on every thread, for the rest of the process.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Whether counting is on.
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The current thread's counts of allocations and frees since it started, while counting was
/// on: zero when the process's global allocator is not a [`CountingAllocator`].
pub fn thread_counts() -> (u64, u64) {
    (
        ALLOCATIONS.try_with(Cell::get).unwrap_or(0),
        FREES.try_with(Cell::get).unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the tests, which switch the process-wide flag.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The allocator, called directly: the test binary's global allocator is the system's.
    fn allocate_and_free(size: usize) {
        let layout = Layout::from_size_align(size, 8).unwrap();
        // SAFETY: a non-zero layout, freed with the same layout.
        unsafe {
            let pointer = CountingAllocator.alloc(layout);
            assert!(!pointer.is_null());
            let grown = CountingAllocator.realloc(pointer, layout, size * 2);
            let layout = Layout::from_size_align(size * 2, 8).unwrap();
            CountingAllocator.dealloc(grown, layout);
            let zeroed = CountingAllocator.alloc_zeroed(layout);
            CountingAllocator.dealloc(zeroed, layout);
        }
    }

    #[test]
    fn counts_start_at_zero_and_count_while_enabled() {
        let _lock = lock();
        std::thread::spawn(|| {
            assert_eq!(thread_counts(), (0, 0), "a fresh thread starts at zero");
            ENABLED.store(false, Ordering::Relaxed);
            allocate_and_free(16);
            assert_eq!(thread_counts(), (0, 0), "nothing is counted while disabled");
            enable();
            assert!(is_enabled());
            allocate_and_free(16);
            // An allocation, a reallocation (one of each), a free, a zeroed allocation and its
            // free.
            assert_eq!(thread_counts(), (3, 3));
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_fresh_thread_counts_without_recursing() {
        let _lock = lock();
        enable();
        // The first touch of the counts on this thread happens inside the allocator.
        let counts = std::thread::spawn(|| {
            allocate_and_free(8);
            thread_counts()
        })
        .join()
        .unwrap();
        assert_eq!(counts, (3, 3));
    }
}
