//! Function values seen from Rust (D2 of `compose-runtime`): calling and releasing the closures
//! that generated code builds, for the runtimes that hold program code (`prism-compose`'s
//! restart functions and contents, `prism-ui`'s handlers).
//!
//! A function value is 16 bytes (D2 and D3 of `closures`): the address of its closure's
//! descriptor, never null, and its environment word, which is the address of its environment,
//! null when the closure captures nothing, or the environment itself when it fits in one word
//! (D9 of `escaping-closures`). A descriptor is four words (D9 of `compose-transform`): the
//! address of the function-table slot of the closure's code; the environment's drop entry, or
//! null when the environment is borrowed or held inline; the environment equality entry of a
//! keepable closure, or null when the closure is not keepable; and the duplicate entry of a
//! keepable closure whose environment is allocated, or null. Closure code takes `(result?,
//! environment, parameters...)` with the host's C calling convention.
//!
//! Calls load the code from the slot each time, so a slot repointed by a hot reload is followed
//! by every value built before it. Nothing here keeps a code address.

use crate::string::PrismString;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The static descriptor of a closure.
#[repr(C)]
#[derive(Debug)]
pub struct Descriptor {
    /// The function-table slot that holds the closure's code.
    pub slot: *const AtomicUsize,
    /// The drop entry of an allocated environment, called with the environment's address.
    pub drop_environment: Option<unsafe extern "C" fn(*mut u8)>,
    /// The environment equality entry of a keepable closure, called with two environment
    /// words and giving 1 when they hold equal captures; null when the closure is not
    /// keepable, because it owns a capture that is not `Copy` or borrows one.
    pub equals_environment: Option<unsafe extern "C" fn(*const u8, *const u8) -> u8>,
    /// The duplicate entry of a keepable closure whose environment is allocated, called with
    /// the environment's address and giving a new allocation holding a copy of its captures;
    /// null when a copy of the environment word is a duplicate.
    pub duplicate_environment: Option<unsafe extern "C" fn(*const u8) -> *mut u8>,
}

impl Descriptor {
    /// A descriptor of a closure with the code in `slot` and no environment entries: one that
    /// captures nothing gets [`prism_rt_empty_environments_equal`] as its equality entry.
    pub const fn new(slot: *const AtomicUsize) -> Descriptor {
        Descriptor {
            slot,
            drop_environment: None,
            equals_environment: None,
            duplicate_environment: None,
        }
    }
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

    /// Calls the value as a composable `compose () -> Unit` function, with the composer word
    /// `composer` and the one change word `changed`, the calling convention of composable
    /// function values (`code-generation`, composable signatures): what Compose's runtime does
    /// in `invokeComposable` (D4 of `compose-library-in-prism`).
    ///
    /// # Safety
    ///
    /// The value must be a live function value of type `compose () -> Unit`.
    pub unsafe fn call_composable(&self, composer: u64, changed: u64) {
        let code = (*(*self.descriptor).slot).load(Ordering::Acquire);
        let code: unsafe extern "C" fn(*mut u8, u64, u64) = std::mem::transmute(code);
        code(self.environment, composer, changed)
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

    /// Calls the value as a `(Bool) -> Unit` function with `value`, passed as one byte, 0 or 1,
    /// as closure code takes a `Bool`.
    ///
    /// # Safety
    ///
    /// The value must be a live function value of type `(Bool) -> Unit`.
    pub unsafe fn call_with_bool(&self, value: bool) {
        let code = (*(*self.descriptor).slot).load(Ordering::Acquire);
        let code: unsafe extern "C" fn(*mut u8, u8) = std::mem::transmute(code);
        code(self.environment, value as u8)
    }

    /// Calls the value as a `(String) -> Unit` function with `value`, which the callee owns
    /// from then on: a `String` is passed by address, as aggregates are, and the callee drops
    /// it (D3 of `closures`).
    ///
    /// # Safety
    ///
    /// The value must be a live function value of type `(String) -> Unit`.
    pub unsafe fn call_with_string(&self, value: PrismString) {
        let code = (*(*self.descriptor).slot).load(Ordering::Acquire);
        let code: unsafe extern "C" fn(*mut u8, *mut PrismString) = std::mem::transmute(code);
        let mut value = std::mem::ManuallyDrop::new(value);
        code(self.environment, &mut *value)
    }

    /// Calls the value as a `() -> Unit` function: [`FunctionValue::call`].
    ///
    /// # Safety
    ///
    /// The value must be a live function value of type `() -> Unit`.
    pub unsafe fn call_unit(&self) {
        self.call()
    }

    /// Whether the value may be kept and compared: its descriptor has an environment equality
    /// entry (D9 of `compose-transform`).
    ///
    /// # Safety
    ///
    /// The value's descriptor must be valid.
    pub unsafe fn is_keepable(&self) -> bool {
        (*self.descriptor).equals_environment.is_some()
    }

    /// A duplicate of a keepable value that owns its own environment: the descriptor, and the
    /// duplicate entry's copy of an allocated environment or a copy of the environment word.
    /// `None` when the value is not keepable.
    ///
    /// # Safety
    ///
    /// The value must be live.
    pub unsafe fn duplicate(&self) -> Option<FunctionValue> {
        let descriptor = &*self.descriptor;
        descriptor.equals_environment?;
        let environment = match descriptor.duplicate_environment {
            Some(duplicate) => duplicate(self.environment),
            None => self.environment,
        };
        Some(FunctionValue {
            descriptor: self.descriptor,
            environment,
        })
    }

    /// Whether `self` and `other` have the same code and equal captures: the same descriptor,
    /// keepable, and environments its equality entry finds equal. Any other pair is unequal.
    ///
    /// # Safety
    ///
    /// Both values must be live.
    pub unsafe fn equals(&self, other: &FunctionValue) -> bool {
        if !std::ptr::eq(self.descriptor, other.descriptor) {
            return false;
        }
        match (*self.descriptor).equals_environment {
            Some(equals) => equals(self.environment, other.environment) != 0,
            None => false,
        }
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
        let descriptor = Descriptor::new(&slot);
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
            drop_environment: Some(count_drop),
            ..Descriptor::new(&slot)
        };
        let without_drop = Descriptor::new(&slot);
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
        let descriptor = Descriptor::new(&slot);
        let stored = FunctionValue {
            descriptor: &descriptor,
            environment: std::ptr::without_provenance_mut(9),
        };
        let read = unsafe { FunctionValue::read_from((&stored as *const FunctionValue).cast()) };
        assert_eq!(read, stored);
        assert_eq!(std::mem::size_of::<FunctionValue>(), 16);
    }

    thread_local! {
        static FLAGS: Cell<u8> = const { Cell::new(0) };
        static STRING: Cell<usize> = const { Cell::new(0) };
    }

    unsafe extern "C" fn take_bool(environment: *mut u8, value: u8) {
        FLAGS.with(|flags| flags.set(environment as u8 + value));
    }

    unsafe extern "C" fn take_string(_: *mut u8, value: *mut PrismString) {
        STRING.with(|length| length.set((*value).character_count));
        crate::prism_rt_string_drop(value);
    }

    unsafe extern "C" fn words_equal(a: *const u8, b: *const u8) -> u8 {
        (a == b) as u8
    }

    /// Two-`Int` environments, allocated: equal when both words are.
    unsafe extern "C" fn pairs_equal(a: *const u8, b: *const u8) -> u8 {
        let (a, b) = (a.cast::<[i64; 2]>(), b.cast::<[i64; 2]>());
        (*a == *b) as u8
    }

    unsafe extern "C" fn duplicate_pair(environment: *const u8) -> *mut u8 {
        Box::into_raw(Box::new(*environment.cast::<[i64; 2]>())).cast()
    }

    unsafe extern "C" fn drop_pair(environment: *mut u8) {
        drop(Box::from_raw(environment.cast::<[i64; 2]>()));
    }

    #[test]
    fn calls_with_a_bool_and_with_a_string_the_callee_frees() {
        let _lock = crate::test_lock();
        let slot = AtomicUsize::new(take_bool as *const () as usize);
        let descriptor = Descriptor::new(&slot);
        let value = FunctionValue {
            descriptor: &descriptor,
            environment: std::ptr::without_provenance_mut(10),
        };
        unsafe { value.call_with_bool(true) };
        assert_eq!(FLAGS.with(Cell::get), 11);
        crate::output::start(Vec::new(), true);
        let slot = AtomicUsize::new(take_string as *const () as usize);
        let descriptor = Descriptor::new(&slot);
        let value = FunctionValue {
            descriptor: &descriptor,
            environment: std::ptr::null_mut(),
        };
        unsafe { value.call_with_string(PrismString::owned("abc")) };
        assert_eq!(STRING.with(Cell::get), 3);
        assert_eq!(crate::finish(), 0, "the callee freed the string");
    }

    #[test]
    fn keepable_values_duplicate_and_compare_by_their_environments() {
        let slot = AtomicUsize::new(first as *const () as usize);
        let inline = Descriptor {
            equals_environment: Some(words_equal),
            ..Descriptor::new(&slot)
        };
        let not_keepable = Descriptor::new(&slot);
        let word = |descriptor: &Descriptor, word: usize| FunctionValue {
            descriptor,
            environment: std::ptr::without_provenance_mut(word),
        };
        unsafe {
            assert!(word(&inline, 1).is_keepable());
            assert!(!word(&not_keepable, 1).is_keepable());
            assert!(word(&inline, 1).equals(&word(&inline, 1)));
            assert!(!word(&inline, 1).equals(&word(&inline, 2)));
            assert!(!word(&not_keepable, 1).equals(&word(&not_keepable, 1)));
            let other = Descriptor {
                equals_environment: Some(words_equal),
                ..Descriptor::new(&slot)
            };
            assert!(!word(&inline, 1).equals(&word(&other, 1)), "another code");
            assert_eq!(word(&inline, 7).duplicate(), Some(word(&inline, 7)));
            assert_eq!(word(&not_keepable, 7).duplicate(), None);
        }
        let allocated = Descriptor {
            slot: &slot,
            drop_environment: Some(drop_pair),
            equals_environment: Some(pairs_equal),
            duplicate_environment: Some(duplicate_pair),
        };
        let pair = |a: i64, b: i64| FunctionValue {
            descriptor: &allocated,
            environment: Box::into_raw(Box::new([a, b])).cast(),
        };
        unsafe {
            let (one, same, other) = (pair(1, 2), pair(1, 2), pair(1, 3));
            assert!(one.equals(&same));
            assert!(!one.equals(&other));
            let copy = one.duplicate().expect("keepable");
            assert_ne!(copy.environment, one.environment, "a new environment");
            assert!(copy.equals(&one));
            for value in [one, same, other, copy] {
                value.release();
            }
        }
    }
}
