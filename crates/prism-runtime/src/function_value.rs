//! Function values seen from Rust (D2 of `compose-runtime`): calling and releasing the closures
//! that generated code builds, for the runtimes that hold program code (`prism-compose`'s
//! restart functions and contents, `prism-ui`'s handlers).
//!
//! A function value is 16 bytes (D2 and D3 of `closures`): the address of its closure's
//! descriptor, never null, and its environment word, which is the address of its environment,
//! null when the closure captures nothing, or the environment itself when it fits in one word
//! (D9 of `escaping-closures`). A descriptor's first word is the address of the function-table
//! slot of the closure's code, and its second the environment's drop entry, or null when the
//! environment is borrowed or held inline. Closure code takes `(result?, environment,
//! parameters...)` with the host's C calling convention.
//!
//! Calls load the code from the slot each time, so a slot repointed by a hot reload is followed
//! by every value built before it. Nothing here keeps a code address.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The static descriptor of a closure.
#[repr(C)]
#[derive(Debug)]
pub struct Descriptor {
    /// The function-table slot that holds the closure's code.
    pub slot: *const AtomicUsize,
    /// The drop entry of an allocated environment, called with the environment's address.
    pub drop_environment: Option<unsafe extern "C" fn(*mut u8)>,
}

// SAFETY: a descriptor is read-only static data; its slot is only accessed atomically.
unsafe impl Sync for Descriptor {}
unsafe impl Send for Descriptor {}

/// A function value: its descriptor and its environment word.
#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub struct FunctionValue {
    pub descriptor: *const Descriptor,
    pub environment: *mut u8,
}

impl FunctionValue {
    /// Reads the function value stored at `address`, taking it over: the memory at `address`
    /// no longer owns it. What an entry point does with a function value passed by address,
    /// since a Prism callee owns a value moved into it.
    ///
    /// # Safety
    ///
    /// `address` must point at a function value that the caller gives up.
    pub unsafe fn read_from(address: *const u8) -> FunctionValue {
        std::ptr::read(address.cast::<FunctionValue>())
    }

    /// The slot that holds this value's code.
    ///
    /// # Safety
    ///
    /// The value's descriptor must be valid.
    pub unsafe fn slot(&self) -> *const AtomicUsize {
        (*self.descriptor).slot
    }

    /// Calls the value as a `(Int) -> Unit` function with `word`: loads the code from the
    /// descriptor's slot, with acquire ordering so code a reload published is seen whole, and
    /// calls it with the environment word and `word`.
    ///
    /// # Safety
    ///
    /// The value must be a live function value of type `(Int) -> Unit`.
    pub unsafe fn call_with_word(&self, word: u64) {
        let code = (*(*self.descriptor).slot).load(Ordering::Acquire);
        let code: unsafe extern "C" fn(*mut u8, u64) = std::mem::transmute(code);
        code(self.environment, word)
    }

    /// Calls the value as a `() -> Unit` function.
    ///
    /// # Safety
    ///
    /// The value must be a live function value of type `() -> Unit`.
    pub unsafe fn call(&self) {
        let code = (*(*self.descriptor).slot).load(Ordering::Acquire);
        let code: unsafe extern "C" fn(*mut u8) = std::mem::transmute(code);
        code(self.environment)
    }

    /// Releases the value: calls its environment's drop entry when the descriptor has one, and
    /// does nothing otherwise.
    ///
    /// # Safety
    ///
    /// The value must be live, and is released once: nothing may use it afterwards.
    pub unsafe fn release(self) {
        if let Some(drop) = (*self.descriptor).drop_environment {
            drop(self.environment)
        }
    }
}

/// The environment equality entry of every closure that captures nothing (D9 of
/// `compose-transform`): two such environments are always equal.
#[no_mangle]
pub extern "C" fn prism_rt_empty_environments_equal(_: *const u8, _: *const u8) -> u8 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        static SEEN: Cell<u64> = const { Cell::new(0) };
        static DROPS: Cell<u32> = const { Cell::new(0) };
    }

    unsafe extern "C" fn first(environment: *mut u8, word: u64) {
        SEEN.with(|seen| seen.set(environment as u64 + word));
    }

    unsafe extern "C" fn second(_: *mut u8, word: u64) {
        SEEN.with(|seen| seen.set(word * 100));
    }

    unsafe extern "C" fn count_drop(_: *mut u8) {
        DROPS.with(|drops| drops.set(drops.get() + 1));
    }

    #[test]
    fn a_call_goes_through_the_slot() {
        let slot = AtomicUsize::new(first as *const () as usize);
        let descriptor = Descriptor {
            slot: &slot,
            drop_environment: None,
        };
        let value = FunctionValue {
            descriptor: &descriptor,
            environment: std::ptr::without_provenance_mut(5),
        };
        unsafe { value.call_with_word(2) };
        assert_eq!(
            SEEN.with(Cell::get),
            7,
            "the inline environment word is passed"
        );
        // A repointed slot is followed by the value built before.
        slot.store(second as *const () as usize, Ordering::Release);
        unsafe { value.call_with_word(3) };
        assert_eq!(SEEN.with(Cell::get), 300);
        assert_eq!(unsafe { value.slot() }, &slot as *const AtomicUsize);
    }

    #[test]
    fn release_calls_the_drop_entry_once_and_skips_a_null_one() {
        let slot = AtomicUsize::new(first as *const () as usize);
        let with_drop = Descriptor {
            slot: &slot,
            drop_environment: Some(count_drop),
        };
        let without_drop = Descriptor {
            slot: &slot,
            drop_environment: None,
        };
        DROPS.with(|drops| drops.set(0));
        unsafe {
            FunctionValue {
                descriptor: &with_drop,
                environment: std::ptr::null_mut(),
            }
            .release();
            FunctionValue {
                descriptor: &without_drop,
                environment: std::ptr::null_mut(),
            }
            .release();
        }
        assert_eq!(DROPS.with(Cell::get), 1);
    }

    #[test]
    fn a_value_is_read_from_its_address() {
        let slot = AtomicUsize::new(first as *const () as usize);
        let descriptor = Descriptor {
            slot: &slot,
            drop_environment: None,
        };
        let stored = FunctionValue {
            descriptor: &descriptor,
            environment: std::ptr::without_provenance_mut(9),
        };
        let read = unsafe { FunctionValue::read_from((&stored as *const FunctionValue).cast()) };
        assert_eq!(read, stored);
        assert_eq!(std::mem::size_of::<FunctionValue>(), 16);
    }
}
