//! The allocator and allocation checking (D8, D12 and D13 of `runtime-strings-and-drops`).
//!
//! Every allocation of a running program goes through [`prism_rt_allocate`] and
//! [`prism_rt_free`], which take the size and alignment of the memory both when allocating and
//! when freeing, over Rust's global allocator. Freeing with the size lets a size-class allocator
//! replace the global one without changing any caller, and lets the allocation check verify
//! each free. An allocation that fails, or a size no allocation can have, is the runtime error
//! "out of memory".
//!
//! Under `prism run --check-allocations` ([`crate::start`] with checking on), the runtime keeps
//! a map from the address of every live allocation to its size and alignment. A free the map
//! does not match ends the program with [`ALLOCATION_CHECK_EXIT_STATUS`]; when `main` returns,
//! [`crate::finish`] releases the memory of initialized top-level values, which are never
//! dropped otherwise, and reports the allocations still live. Without the option, the two
//! functions test one relaxed atomic flag and track nothing.

use crate::output::flush;
use crate::panic::{panic_without_location, PanicKind};
use std::alloc::Layout;
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The exit status of a program whose allocation check failed: it leaked memory or freed memory
/// that is not allocated. Distinct from `0` (`main` returned), `1` (the program did not compile)
/// and `101` (a runtime error), so a harness tells a compiler bug from a program error without
/// reading stderr.
pub const ALLOCATION_CHECK_EXIT_STATUS: i32 = 102;

/// The message of a free that matches no live allocation.
pub const INVALID_FREE_MESSAGE: &str = "allocation check: freed memory that is not allocated";

/// Whether allocations are tracked, read by every allocation and free.
static CHECKING: AtomicBool = AtomicBool::new(false);

/// The function that drops a top-level value in place, its drop glue.
pub type DropFunction = extern "C" fn(*mut u8);

/// What the allocation check tracks.
struct Tracking {
    allocations: AllocationMap,
    /// The storage of each initialized top-level value that owns memory, and its drop glue, in
    /// order of initialization.
    top_level_values: Vec<(usize, DropFunction)>,
}

static TRACKING: Mutex<Tracking> = Mutex::new(Tracking {
    allocations: AllocationMap {
        live: None,
        allocations_made: 0,
        live_bytes: 0,
    },
    top_level_values: Vec::new(),
});

fn with_tracking<T>(f: impl FnOnce(&mut Tracking) -> T) -> T {
    // A poisoned lock only means an earlier panic of the host; the map is still usable.
    let mut tracking = TRACKING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut tracking)
}

/// The totals of the allocation check, maintained while it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AllocationStatistics {
    /// Allocations made since the run started.
    pub allocations_made: u64,
    /// Allocations not freed yet.
    pub live_allocations: usize,
    /// The bytes of the live allocations.
    pub live_bytes: usize,
}

/// A free that matches no live allocation: an address that is not allocated (a double free, a
/// free of static text), or one allocated with another size or alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidFree;

/// The live allocations of a checked run, by address.
#[derive(Debug, Default)]
pub struct AllocationMap {
    /// Address to size and alignment; `None` until the first allocation, so the map can be a
    /// `static`.
    live: Option<HashMap<usize, (usize, usize)>>,
    allocations_made: u64,
    live_bytes: usize,
}

impl AllocationMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the allocation of `size` bytes aligned to `alignment` at `address`.
    pub fn record_allocation(&mut self, address: usize, size: usize, alignment: usize) {
        self.live
            .get_or_insert_with(HashMap::new)
            .insert(address, (size, alignment));
        self.allocations_made += 1;
        self.live_bytes += size;
    }

    /// Records the free of `size` bytes aligned to `alignment` at `address`, which must be a
    /// live allocation of exactly that size and alignment.
    pub fn record_free(
        &mut self,
        address: usize,
        size: usize,
        alignment: usize,
    ) -> Result<(), InvalidFree> {
        let live = self.live.get_or_insert_with(HashMap::new);
        match live.get(&address) {
            Some(&recorded) if recorded == (size, alignment) => {
                live.remove(&address);
                self.live_bytes -= size;
                Ok(())
            }
            _ => Err(InvalidFree),
        }
    }

    /// The totals of the map.
    pub fn statistics(&self) -> AllocationStatistics {
        AllocationStatistics {
            allocations_made: self.allocations_made,
            live_allocations: self.live.as_ref().map_or(0, HashMap::len),
            live_bytes: self.live_bytes,
        }
    }

    /// The report of the allocations still live, if any: what the check writes when `main`
    /// returns.
    pub fn leak_report(&self) -> Option<String> {
        let statistics = self.statistics();
        (statistics.live_allocations > 0).then(|| {
            format!(
                "allocation check: allocations still live at exit: {} ({} bytes)",
                statistics.live_allocations, statistics.live_bytes
            )
        })
    }

    fn clear(&mut self) {
        *self = AllocationMap {
            live: self.live.take().map(|mut live| {
                live.clear();
                live
            }),
            allocations_made: 0,
            live_bytes: 0,
        };
    }
}

/// The layout of an allocation of `size` bytes aligned to `alignment`, or `None` when no
/// allocation can have it: an alignment that is not a power of two, or a size that, rounded up
/// to the alignment, exceeds `isize::MAX`. Such a request is out of memory.
pub fn allocation_layout(size: usize, alignment: usize) -> Option<Layout> {
    Layout::from_size_align(size, alignment).ok()
}

/// Starts a run with the allocation check on or off, forgetting what an earlier run tracked.
pub(crate) fn start_tracking(check_allocations: bool) {
    with_tracking(|tracking| {
        tracking.allocations.clear();
        tracking.top_level_values.clear();
    });
    CHECKING.store(check_allocations, Ordering::Relaxed);
}

/// Ends a checked run whose `main` returned: drops the registered top-level values in reverse
/// order of initialization, then gives the report of the allocations still live, if any. Without
/// the check, gives nothing.
pub(crate) fn finish_tracking() -> Option<String> {
    if !CHECKING.load(Ordering::Relaxed) {
        return None;
    }
    // The drops free through `prism_rt_free`, which takes the lock: release it first.
    let values = with_tracking(|tracking| std::mem::take(&mut tracking.top_level_values));
    for (storage, drop) in values.into_iter().rev() {
        drop(storage as *mut u8);
    }
    with_tracking(|tracking| tracking.allocations.leak_report())
}

/// The totals of the current run's allocation check; all zero when it is off.
pub fn allocation_statistics() -> AllocationStatistics {
    with_tracking(|tracking| tracking.allocations.statistics())
}

/// Flushes the program's output, writes `message` to stderr and exits with
/// [`ALLOCATION_CHECK_EXIT_STATUS`].
fn report_check_failure(message: &str) -> ! {
    flush();
    {
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "{}", message);
        let _ = stderr.flush();
    }
    crate::panic::stop(ALLOCATION_CHECK_EXIT_STATUS)
}

/// Allocates `size` bytes aligned to `alignment`, a power of two, raising "out of memory" when
/// the system cannot. A request of zero bytes allocates nothing and gives a dangling pointer
/// aligned to `alignment`, which [`prism_rt_free`] accepts with the same size.
#[no_mangle]
pub extern "C" fn prism_rt_allocate(size: usize, alignment: usize) -> *mut u8 {
    let Some(layout) = allocation_layout(size, alignment) else {
        panic_without_location(PanicKind::OutOfMemory, "")
    };
    if size == 0 {
        return std::ptr::without_provenance_mut(alignment);
    }
    // SAFETY: the layout has a non-zero size.
    let pointer = unsafe { std::alloc::alloc(layout) };
    if pointer.is_null() {
        panic_without_location(PanicKind::OutOfMemory, "")
    }
    if CHECKING.load(Ordering::Relaxed) {
        with_tracking(|tracking| {
            tracking
                .allocations
                .record_allocation(pointer as usize, size, alignment)
        });
    }
    pointer
}

/// Reallocates the `old_size` bytes aligned to `alignment` at `pointer`, which
/// [`prism_rt_allocate`] or this function gave, to `new_size` bytes, keeping the first
/// `old_size` bytes, and returns the new address, which may be the old one (D14 of
/// `collections`). Under the allocation check, the map entry moves to the new address, and a
/// pointer the map does not hold with that size ends the program with
/// [`ALLOCATION_CHECK_EXIT_STATUS`] as an invalid free. A failure is "out of memory". Both sizes
/// must be positive.
///
/// # Safety
///
/// Outside the allocation check, `pointer` must be a live allocation of `old_size` bytes aligned
/// to `alignment`.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_reallocate(
    pointer: *mut u8,
    old_size: usize,
    new_size: usize,
    alignment: usize,
) -> *mut u8 {
    let (Some(old), Some(_)) = (
        allocation_layout(old_size, alignment),
        allocation_layout(new_size, alignment),
    ) else {
        panic_without_location(PanicKind::OutOfMemory, "")
    };
    if CHECKING.load(Ordering::Relaxed) {
        let verdict = with_tracking(|tracking| {
            tracking
                .allocations
                .record_free(pointer as usize, old_size, alignment)
        });
        if verdict.is_err() {
            report_check_failure(INVALID_FREE_MESSAGE)
        }
    }
    let moved = std::alloc::realloc(pointer, old, new_size);
    if moved.is_null() {
        panic_without_location(PanicKind::OutOfMemory, "")
    }
    if CHECKING.load(Ordering::Relaxed) {
        with_tracking(|tracking| {
            tracking
                .allocations
                .record_allocation(moved as usize, new_size, alignment)
        });
    }
    moved
}

/// Frees the `size` bytes aligned to `alignment` at `pointer`, which [`prism_rt_allocate`] gave
/// for that size and alignment. Under the allocation check, a free that matches no live
/// allocation ends the program with [`ALLOCATION_CHECK_EXIT_STATUS`] instead.
///
/// # Safety
///
/// Outside the allocation check, `pointer` must be a live allocation of `size` bytes aligned to
/// `alignment`.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_free(pointer: *mut u8, size: usize, alignment: usize) {
    // A zero-sized allocation is a dangling pointer that was never allocated or tracked.
    if size == 0 {
        return;
    }
    if CHECKING.load(Ordering::Relaxed) {
        let verdict = with_tracking(|tracking| {
            tracking
                .allocations
                .record_free(pointer as usize, size, alignment)
        });
        if verdict.is_err() {
            report_check_failure(INVALID_FREE_MESSAGE)
        }
    }
    let Some(layout) = allocation_layout(size, alignment) else {
        report_check_failure(INVALID_FREE_MESSAGE)
    };
    std::alloc::dealloc(pointer, layout);
}

/// Records that the top-level value whose storage is at `storage` was initialized and owns
/// memory, which `drop` frees. Called once per value by its accessor, after its initializer
/// returns. Under the allocation check, [`crate::finish`] runs the drops in reverse order of
/// registration before reporting what is still live; otherwise nothing is recorded, since
/// top-level values are never dropped.
#[no_mangle]
pub extern "C" fn prism_rt_register_top_level_value(storage: *mut u8, drop: DropFunction) {
    if CHECKING.load(Ordering::Relaxed) {
        with_tracking(|tracking| tracking.top_level_values.push((storage as usize, drop)));
    }
}

/// Serializes the tests of this process that start runs or read the allocation check, which is
/// process-wide.
#[doc(hidden)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panic::panic_message;

    #[test]
    fn an_allocation_and_its_free_leave_nothing_live() {
        let mut map = AllocationMap::new();
        map.record_allocation(0x1000, 5, 1);
        map.record_allocation(0x2000, 16, 8);
        assert_eq!(
            map.statistics(),
            AllocationStatistics {
                allocations_made: 2,
                live_allocations: 2,
                live_bytes: 21
            }
        );
        assert_eq!(map.record_free(0x1000, 5, 1), Ok(()));
        assert_eq!(map.record_free(0x2000, 16, 8), Ok(()));
        assert_eq!(map.statistics().live_allocations, 0);
        assert_eq!(map.statistics().allocations_made, 2);
        assert_eq!(map.leak_report(), None);
    }

    #[test]
    fn invalid_frees_are_reported() {
        let mut map = AllocationMap::new();
        map.record_allocation(0x1000, 5, 1);
        // Another size, another alignment, an address never allocated.
        assert_eq!(map.record_free(0x1000, 6, 1), Err(InvalidFree));
        assert_eq!(map.record_free(0x1000, 5, 8), Err(InvalidFree));
        assert_eq!(map.record_free(0x3000, 5, 1), Err(InvalidFree));
        // The same allocation twice.
        assert_eq!(map.record_free(0x1000, 5, 1), Ok(()));
        assert_eq!(map.record_free(0x1000, 5, 1), Err(InvalidFree));
    }

    #[test]
    fn a_leak_is_reported() {
        // The scenario "A leak is reported" of the `execution` spec.
        let mut map = AllocationMap::new();
        map.record_allocation(0x1000, 5, 1);
        assert_eq!(
            map.leak_report().as_deref(),
            Some("allocation check: allocations still live at exit: 1 (5 bytes)")
        );
    }

    #[test]
    fn out_of_memory() {
        assert_eq!(panic_message(PanicKind::OutOfMemory, ""), "out of memory");
        assert!(allocation_layout(usize::MAX, 1).is_none());
        assert!(allocation_layout(isize::MAX as usize, 8).is_none());
        assert!(allocation_layout(16, 3).is_none());
        assert!(allocation_layout(16, 8).is_some());
    }

    extern "C" fn free_five_bytes(storage: *mut u8) {
        // SAFETY: the storage holds the address of a 5-byte allocation.
        unsafe { prism_rt_free(*(storage as *mut *mut u8), 5, 1) };
    }

    #[test]
    fn checked_runs_track_allocations_and_release_top_level_values() {
        let _lock = test_lock();
        start_tracking(true);
        let pointer = prism_rt_allocate(5, 1);
        assert_eq!(allocation_statistics().live_allocations, 1);
        // A top-level value owning the only allocation is released before the check.
        let mut storage = pointer;
        prism_rt_register_top_level_value(std::ptr::addr_of_mut!(storage).cast(), free_five_bytes);
        assert_eq!(finish_tracking(), None);
        assert_eq!(
            allocation_statistics(),
            AllocationStatistics {
                allocations_made: 1,
                live_allocations: 0,
                live_bytes: 0
            }
        );

        start_tracking(true);
        let leaked = prism_rt_allocate(5, 1);
        assert_eq!(
            finish_tracking().as_deref(),
            Some("allocation check: allocations still live at exit: 1 (5 bytes)")
        );
        // SAFETY: allocated above with this size and alignment.
        unsafe { prism_rt_free(leaked, 5, 1) };
        start_tracking(false);
    }

    #[test]
    fn a_reallocation_moves_the_tracked_allocation() {
        let _lock = test_lock();
        start_tracking(true);
        let pointer = prism_rt_allocate(16, 8);
        // SAFETY: allocated above, 16 bytes aligned to 8.
        unsafe {
            pointer.cast::<u64>().write(7);
            let grown = prism_rt_reallocate(pointer, 16, 64, 8);
            assert_eq!(grown.cast::<u64>().read(), 7);
            assert_eq!(
                allocation_statistics(),
                AllocationStatistics {
                    allocations_made: 2,
                    live_allocations: 1,
                    live_bytes: 64
                }
            );
            prism_rt_free(grown, 64, 8);
        }
        assert_eq!(finish_tracking(), None);
        start_tracking(false);
    }

    #[test]
    fn unchecked_runs_track_nothing() {
        let _lock = test_lock();
        start_tracking(false);
        let pointer = prism_rt_allocate(32, 8);
        assert!(!pointer.is_null());
        assert_eq!(pointer as usize % 8, 0);
        assert_eq!(allocation_statistics(), AllocationStatistics::default());
        assert_eq!(finish_tracking(), None);
        // SAFETY: allocated above with this size and alignment.
        unsafe { prism_rt_free(pointer, 32, 8) };
        let empty = prism_rt_allocate(0, 8);
        assert_eq!(empty as usize, 8);
        // SAFETY: a zero-sized allocation is freed with its size.
        unsafe { prism_rt_free(empty, 0, 8) };
    }
}
