//! Slots and their types (D6 of `compose-runtime`).
//!
//! A typed slot holds one value with a pointer to its [`SlotType`], a static read-only record
//! that generated code points to: the value's 256-bit type fingerprint, size, alignment, drop
//! entry, equality entry and name. Reads compare the record a caller gives with the slot's, by
//! pointer and then by fingerprint, so a group matched by mistake (after an edit under hot
//! reload) can never give back a value of another type.
//!
//! A typed value lives in a block of its own, allocated when the slot is created, so its
//! address stays valid until the slot is released at the end of a pass whatever happens to the
//! group's slot vector (D6, revised at apply time). A word slot holds a 64-bit value and a type
//! tag, for hand-written code that cannot name a record: it needs no block, since its value is
//! only ever read by value.

use std::alloc::Layout;

/// The static record of a slot's type.
#[repr(C)]
#[derive(Debug)]
pub struct SlotType {
    /// The 256-bit structural type fingerprint of `hot-reload` (its D6).
    pub fingerprint: [u64; 4],
    pub size: usize,
    pub alignment: usize,
    /// Drops a value in place; `None` for a value without drop glue.
    pub drop: Option<unsafe extern "C" fn(*mut u8)>,
    /// Compares two values, giving 1 when they are equal; `None` compares as always changed.
    pub equals: Option<unsafe extern "C" fn(*const u8, *const u8) -> u8>,
    /// The type's name, UTF-8, for dumps.
    pub name: *const u8,
    pub name_length: usize,
}

// SAFETY: a slot type is static, read-only data.
unsafe impl Sync for SlotType {}
unsafe impl Send for SlotType {}

impl SlotType {
    /// The type's name.
    pub fn name(&self) -> &str {
        if self.name.is_null() || self.name_length == 0 {
            return "";
        }
        // SAFETY: a record names `name_length` bytes of UTF-8 text at `name`.
        unsafe {
            std::str::from_utf8(std::slice::from_raw_parts(self.name, self.name_length))
                .unwrap_or("?")
        }
    }

    /// Whether a value of `self` may be read as a value of `other`: the same record, or one
    /// with the same fingerprint, size and alignment, as records of the same type in two loads
    /// of a module are.
    pub fn matches(&self, other: &SlotType) -> bool {
        std::ptr::eq(self, other)
            || (self.fingerprint == other.fingerprint
                && self.size == other.size
                && self.alignment == other.alignment)
    }

    /// The layout of a block holding one value.
    fn layout(&self) -> Option<Layout> {
        Layout::from_size_align(self.size, self.alignment.max(1)).ok()
    }
}

/// One slot of a group.
#[derive(Debug)]
pub enum Slot {
    /// A word slot: a 64-bit value and its type tag.
    Word { tag: i64, value: i64 },
    /// A typed slot: its record, the block holding its value, and the state cells created
    /// by the `remember` calculation whose result it holds, released after its value (D12
    /// of `state-handles`).
    Typed {
        slot_type: *const SlotType,
        value: *mut u8,
        cells: Vec<crate::handle::Handle>,
    },
}

impl Slot {
    /// A typed slot holding the `slot_type.size` bytes at `source`, moved into a new block.
    ///
    /// # Safety
    ///
    /// `slot_type` must be a valid record, and `source` must point at a value of it that the
    /// caller gives up.
    pub unsafe fn typed(slot_type: *const SlotType, source: *const u8) -> Slot {
        let record = &*slot_type;
        let value = allocate_block(record);
        std::ptr::copy_nonoverlapping(source, value, record.size);
        Slot::Typed {
            slot_type,
            value,
            cells: Vec::new(),
        }
    }

    /// The record of a typed slot that matches `slot_type`, with its value's address.
    ///
    /// # Safety
    ///
    /// `slot_type` must be a valid record.
    pub unsafe fn typed_match(&self, slot_type: *const SlotType) -> Option<*mut u8> {
        match self {
            Slot::Typed {
                slot_type: own,
                value,
                ..
            } if (**own).matches(&*slot_type) => Some(*value),
            _ => None,
        }
    }

    /// Releases the slot's block after its value was dropped. A word slot holds nothing.
    ///
    /// # Safety
    ///
    /// The value must have been dropped, or have no drop entry, and the slot is released once.
    pub unsafe fn free_block(&self) {
        if let Slot::Typed {
            slot_type, value, ..
        } = self
        {
            free_block(&**slot_type, *value);
        }
    }

    /// The drop entry and value address of a typed slot that needs a drop.
    pub fn drop_entry(&self) -> Option<(unsafe extern "C" fn(*mut u8), *mut u8)> {
        match self {
            // SAFETY: a slot's record is valid while the slot lives.
            Slot::Typed {
                slot_type, value, ..
            } => unsafe { (**slot_type).drop }.map(|drop| (drop, *value)),
            Slot::Word { .. } => None,
        }
    }

    /// The state cells attached to the slot.
    pub fn cells(&self) -> &[crate::handle::Handle] {
        match self {
            Slot::Typed { cells, .. } => cells,
            Slot::Word { .. } => &[],
        }
    }
}

/// A block for one value of `record`; a dangling, aligned pointer for a zero-sized one.
pub(crate) fn allocate_block(record: &SlotType) -> *mut u8 {
    let Some(layout) = record.layout() else {
        prism_runtime::panic_without_location(prism_runtime::PanicKind::OutOfMemory, "")
    };
    if layout.size() == 0 {
        return std::ptr::without_provenance_mut(layout.align());
    }
    // SAFETY: the layout has a non-zero size.
    let block = unsafe { std::alloc::alloc(layout) };
    if block.is_null() {
        prism_runtime::panic_without_location(prism_runtime::PanicKind::OutOfMemory, "")
    }
    block
}

/// Frees a block of [`allocate_block`].
///
/// # Safety
///
/// `block` must be a block allocated for `record`, freed once.
pub(crate) unsafe fn free_block(record: &SlotType, block: *mut u8) {
    if let Some(layout) = record.layout() {
        if layout.size() > 0 {
            std::alloc::dealloc(block, layout);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(fingerprint: u64, size: usize, alignment: usize) -> SlotType {
        SlotType {
            fingerprint: [fingerprint, 0, 0, 0],
            size,
            alignment,
            drop: None,
            equals: None,
            name: "T".as_ptr(),
            name_length: 1,
        }
    }

    #[test]
    fn small_and_large_values_round_trip_their_bytes() {
        let small = record(1, 8, 8);
        let large = record(2, 40, 8);
        let word = 0x1122_3344_5566_7788u64;
        let bytes: [u8; 40] = std::array::from_fn(|i| i as u8);
        unsafe {
            let a = Slot::typed(&small, (&word as *const u64).cast());
            let b = Slot::typed(&large, bytes.as_ptr());
            let a_value = a.typed_match(&small).unwrap();
            let b_value = b.typed_match(&large).unwrap();
            assert_eq!(a_value.cast::<u64>().read(), word);
            assert_eq!(std::slice::from_raw_parts(b_value, 40), &bytes);
            a.free_block();
            b.free_block();
        }
    }

    #[test]
    fn records_with_equal_fingerprints_match() {
        let a = record(7, 16, 8);
        let b = record(7, 16, 8);
        let c = record(8, 16, 8);
        assert!(a.matches(&b));
        assert!(!a.matches(&c));
        // A fingerprint collision with another layout never matches.
        assert!(!a.matches(&record(7, 8, 8)));
        let zero = record(9, 0, 1);
        unsafe {
            let slot = Slot::typed(&zero, std::ptr::NonNull::<u8>::dangling().as_ptr());
            assert!(slot.typed_match(&b).is_none());
            assert!(slot.typed_match(&zero).is_some());
            slot.free_block();
        }
        assert_eq!(a.name(), "T");
    }
}
