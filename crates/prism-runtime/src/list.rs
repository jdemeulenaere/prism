//! Lists (D13 to D15 of `collections`): the buffer operations that generated code does not do
//! inline, type-erased by the element's size and alignment.
//!
//! A `List<T>` is a [`PrismList`]: the address of its elements, its size and its capacity, in
//! pointer-sized words. Generated code reads the size, computes element addresses (checking
//! them against the size), writes an added element in place and calls [`prism_rt_list_grow`]
//! only when the buffer is full, and moves elements in and out by their layouts. The functions
//! here allocate, grow, open and close gaps, reverse and free buffers through the runtime's
//! allocator, and raise the two list errors. Every size given here is an element's stride: its
//! size rounded up to its alignment.
//!
//! A list owns no memory when its capacity is 0 or its elements are zero-sized; its pointer is
//! then the element alignment, never null. A list of zero-sized elements has the largest `Int`
//! as its capacity, so it never grows.

use crate::allocation::{prism_rt_allocate, prism_rt_free, prism_rt_reallocate};
use crate::panic::{panic_at, panic_without_location, text, PanicKind};

/// The runtime's view of a `List<T>`, laid out as generated code lays it out.
#[repr(C)]
#[derive(Debug)]
pub struct PrismList {
    /// The first element, never null: the element alignment when the list owns no buffer.
    pub pointer: *mut u8,
    /// The number of elements.
    pub size: usize,
    /// The number of elements the buffer holds.
    pub capacity: usize,
}

/// The capacity of a list of zero-sized elements, which never allocates: the largest `Int`.
pub const ZERO_SIZED_CAPACITY: usize = i64::MAX as usize;

/// The capacity of a list's first buffer (D14 of `collections`, Rust's minimum for small
/// elements).
pub const FIRST_CAPACITY: usize = 4;

/// The bytes of `capacity` elements of `element_size` bytes, raising "out of memory" when they
/// do not fit in a word.
fn buffer_bytes(capacity: usize, element_size: usize) -> usize {
    capacity
        .checked_mul(element_size)
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .unwrap_or_else(|| panic_without_location(PanicKind::OutOfMemory, ""))
}

/// Writes into `destination` an empty list with a buffer of exactly `capacity` elements (none
/// for a capacity of 0 or zero-sized elements): what `listOf` with `capacity` arguments and
/// `clone()` of a list of that size start from (D14 of `collections`).
///
/// # Safety
///
/// `destination` must point at writable storage for a [`PrismList`], and `element_alignment`
/// must be a power of two.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_list_with_capacity(
    destination: *mut PrismList,
    capacity: usize,
    element_size: usize,
    element_alignment: usize,
) {
    let empty = std::ptr::without_provenance_mut(element_alignment);
    let list = if element_size == 0 {
        PrismList {
            pointer: empty,
            size: 0,
            capacity: ZERO_SIZED_CAPACITY,
        }
    } else if capacity == 0 {
        PrismList {
            pointer: empty,
            size: 0,
            capacity: 0,
        }
    } else {
        let bytes = buffer_bytes(capacity, element_size);
        PrismList {
            pointer: prism_rt_allocate(bytes, element_alignment),
            size: 0,
            capacity,
        }
    };
    destination.write(list);
}

/// Grows the buffer of the full list `list` to twice its capacity, or to [`FIRST_CAPACITY`]
/// from 0, keeping its elements (D14 of `collections`). Never called for zero-sized elements,
/// whose capacity is never reached.
///
/// # Safety
///
/// `list` must point at a list of elements of `element_size` bytes aligned to
/// `element_alignment`.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_list_grow(
    list: *mut PrismList,
    element_size: usize,
    element_alignment: usize,
) {
    let list = &mut *list;
    if element_size == 0 {
        return;
    }
    let capacity = if list.capacity == 0 {
        FIRST_CAPACITY
    } else {
        list.capacity
            .checked_mul(2)
            .unwrap_or_else(|| panic_without_location(PanicKind::OutOfMemory, ""))
    };
    let bytes = buffer_bytes(capacity, element_size);
    list.pointer = if list.capacity == 0 {
        prism_rt_allocate(bytes, element_alignment)
    } else {
        let old = list.capacity * element_size;
        prism_rt_reallocate(list.pointer, old, bytes, element_alignment)
    };
    list.capacity = capacity;
}

/// Opens a gap for one element at `index` of `list`, growing it when it is full and moving the
/// elements from `index` on one place up, and returns the gap's address, which the caller
/// fills: `insert(index, element)`. An index outside `0..=size` raises "list index <i> out of
/// range for size <n>" at `file:line:column`, before the list changes.
///
/// # Safety
///
/// `list` must point at a list of elements of `element_size` bytes aligned to
/// `element_alignment`, and `file` at `file_length` readable bytes.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn prism_rt_list_open_gap(
    list: *mut PrismList,
    index: i64,
    element_size: usize,
    element_alignment: usize,
    file: *const u8,
    file_length: usize,
    line: u32,
    column: u32,
) -> *mut u8 {
    let size = (*list).size;
    let Some(position) = usize::try_from(index).ok().filter(|index| *index <= size) else {
        prism_rt_list_index_out_of_range(index, size as i64, file, file_length, line, column)
    };
    if size == (*list).capacity {
        prism_rt_list_grow(list, element_size, element_alignment);
    }
    let list = &mut *list;
    let gap = list.pointer.add(position * element_size);
    std::ptr::copy(gap, gap.add(element_size), (size - position) * element_size);
    list.size = size + 1;
    gap
}

/// Closes the gap left at `index` of `list` by an element its caller moved out, moving the later
/// elements one place down: `removeAt(index)`, after the bounds check.
///
/// # Safety
///
/// `list` must point at a list of elements of `element_size` bytes, and `index` must be below
/// its size.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_list_close_gap(
    list: *mut PrismList,
    index: usize,
    element_size: usize,
) {
    let list = &mut *list;
    let gap = list.pointer.add(index * element_size);
    let later = list.size - index - 1;
    std::ptr::copy(gap.add(element_size), gap, later * element_size);
    list.size -= 1;
}

/// Reverses the order of the elements of `list` in place: `reverse()`.
///
/// # Safety
///
/// `list` must point at a list of elements of `element_size` bytes.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_list_reverse(list: *mut PrismList, element_size: usize) {
    let list = &*list;
    if element_size == 0 || list.size < 2 {
        return;
    }
    let (mut low, mut high) = (0, list.size - 1);
    while low < high {
        std::ptr::swap_nonoverlapping(
            list.pointer.add(low * element_size),
            list.pointer.add(high * element_size),
            element_size,
        );
        low += 1;
        high -= 1;
    }
}

/// Frees the buffer of `list`, whose elements were dropped, if it owns one.
///
/// # Safety
///
/// `list` must point at a list of elements of `element_size` bytes aligned to
/// `element_alignment`.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_list_free(
    list: *mut PrismList,
    element_size: usize,
    element_alignment: usize,
) {
    let list = &*list;
    if element_size != 0 && list.capacity != 0 {
        prism_rt_free(
            list.pointer,
            list.capacity * element_size,
            element_alignment,
        );
    }
}

/// Raises "list index <i> out of range for size <n>" at `file:line:column`: an element access,
/// `insert` or `removeAt` whose index is out of range (D8 of `collections`).
///
/// # Safety
///
/// `file` must point at `file_length` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_list_index_out_of_range(
    index: i64,
    size: i64,
    file: *const u8,
    file_length: usize,
    line: u32,
    column: u32,
) -> ! {
    panic_at(
        PanicKind::ListIndexOutOfRange,
        &format!("{} out of range for size {}", index, size),
        &text(file, file_length),
        line,
        column,
    )
}

/// Raises "removeLast() on an empty list" at `file:line:column`.
///
/// # Safety
///
/// `file` must point at `file_length` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn prism_rt_list_empty(
    file: *const u8,
    file_length: usize,
    line: u32,
    column: u32,
) -> ! {
    panic_at(
        PanicKind::EmptyList,
        "",
        &text(file, file_length),
        line,
        column,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocation::{allocation_statistics, start_tracking, test_lock};

    fn empty_list() -> PrismList {
        PrismList {
            pointer: std::ptr::null_mut(),
            size: 0,
            capacity: 0,
        }
    }

    /// A list of `i64`s holding `values`, built as generated code builds one.
    unsafe fn list_of(values: &[i64]) -> PrismList {
        let mut list = empty_list();
        prism_rt_list_with_capacity(&mut list, values.len(), 8, 8);
        for value in values {
            push(&mut list, *value);
        }
        list
    }

    /// `add`, as generated code inlines it.
    unsafe fn push(list: &mut PrismList, value: i64) {
        if list.size == list.capacity {
            prism_rt_list_grow(list, 8, 8);
        }
        (list.pointer as *mut i64).add(list.size).write(value);
        list.size += 1;
    }

    unsafe fn values(list: &PrismList) -> Vec<i64> {
        std::slice::from_raw_parts(list.pointer as *const i64, list.size).to_vec()
    }

    #[test]
    fn growth_doubles_from_four_and_keeps_the_elements() {
        let _lock = test_lock();
        start_tracking(true);
        unsafe {
            let mut list = list_of(&[]);
            assert_eq!((list.pointer as usize, list.capacity), (8, 0));
            assert_eq!(allocation_statistics().allocations_made, 0);
            let mut capacities = Vec::new();
            for value in 0..20 {
                push(&mut list, value);
                if capacities.last() != Some(&list.capacity) {
                    capacities.push(list.capacity);
                }
            }
            assert_eq!(capacities, [4, 8, 16, 32]);
            assert_eq!(values(&list), (0..20).collect::<Vec<_>>());
            prism_rt_list_free(&mut list, 8, 8);
            // `listOf` allocates exactly its size.
            let mut exact = list_of(&[1, 2, 3]);
            assert_eq!((exact.size, exact.capacity), (3, 3));
            prism_rt_list_free(&mut exact, 8, 8);
        }
        assert_eq!(allocation_statistics().live_allocations, 0);
        start_tracking(false);
    }

    #[test]
    fn gaps_open_and_close_at_both_ends() {
        let _lock = test_lock();
        start_tracking(true);
        unsafe {
            let mut list = list_of(&[1, 2, 3]);
            let file = "main.prism";
            let gap = prism_rt_list_open_gap(&mut list, 0, 8, 8, file.as_ptr(), file.len(), 1, 1);
            (gap as *mut i64).write(0);
            let gap = prism_rt_list_open_gap(&mut list, 4, 8, 8, file.as_ptr(), file.len(), 1, 1);
            (gap as *mut i64).write(4);
            assert_eq!(values(&list), [0, 1, 2, 3, 4]);
            assert_eq!(list.capacity, 6);
            prism_rt_list_close_gap(&mut list, 0, 8);
            prism_rt_list_close_gap(&mut list, 3, 8);
            assert_eq!(values(&list), [1, 2, 3]);
            prism_rt_list_reverse(&mut list, 8);
            assert_eq!(values(&list), [3, 2, 1]);
            prism_rt_list_free(&mut list, 8, 8);
        }
        assert_eq!(allocation_statistics().live_allocations, 0);
        start_tracking(false);
    }

    #[test]
    fn zero_sized_elements_allocate_nothing() {
        let _lock = test_lock();
        start_tracking(true);
        unsafe {
            let mut list = empty_list();
            prism_rt_list_with_capacity(&mut list, 3, 0, 1);
            assert_eq!(
                (list.pointer as usize, list.capacity),
                (1, ZERO_SIZED_CAPACITY)
            );
            let file = "main.prism";
            prism_rt_list_open_gap(&mut list, 0, 0, 1, file.as_ptr(), file.len(), 1, 1);
            prism_rt_list_reverse(&mut list, 0);
            assert_eq!(list.size, 1);
            prism_rt_list_free(&mut list, 0, 1);
        }
        assert_eq!(allocation_statistics().allocations_made, 0);
        start_tracking(false);
    }

    #[test]
    fn list_errors() {
        use crate::panic::panic_message;
        assert_eq!(
            panic_message(PanicKind::ListIndexOutOfRange, "3 out of range for size 3"),
            "list index 3 out of range for size 3"
        );
        assert_eq!(
            panic_message(PanicKind::EmptyList, ""),
            "removeLast() on an empty list"
        );
        assert_eq!(buffer_bytes(4, 8), 32);
    }
}
