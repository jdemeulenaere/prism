//! Function values and slot types built from Rust, for the tests of this crate and of the
//! crates built on it (`prism-ui`). Not for generated code, which builds its own.
//!
//! - [`content`] wraps a Rust closure in a `(Int) -> Unit` function value with a boxed
//!   environment, which the value's drop entry frees, as an owning closure's would be.
//! - [`inline_function`] builds a function value whose environment is one inline word and
//!   whose descriptor is static, so building and calling it allocates nothing, as an escaping
//!   closure over one `Int` does (`escaping-closures` D9).
//! - [`slot_type`] gives a record for a Rust type, with drop and equality entries over its
//!   `Drop` and `PartialEq`.
//! - [`effect`] wraps a Rust closure in a `() -> Unit` function value, a side effect.
//! - [`leak_group`], [`leak_slot`] and [`leak_cell`] leave a group, a slot and a state cell
//!   live on the thread that no composition owns, as a runtime bug would, for the tests of the
//!   allocation check's live-object report (D6 of `reactive-app-verification`).

use crate::composer::Composer;
use crate::slot::SlotType;
use prism_runtime::{Descriptor, FunctionValue};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

type Closure = Box<dyn Fn(Composer)>;

/// The code slot shared by every [`content`] value.
static CONTENT_SLOT: AtomicUsize = AtomicUsize::new(0);

static CONTENT_DESCRIPTOR: Descriptor = Descriptor {
    slot: &CONTENT_SLOT,
    drop_environment: Some(drop_closure),
    equals_environment: None,
    duplicate_environment: None,
};

unsafe extern "C" fn call_closure(environment: *mut u8, composer: u64) {
    let closure = &*(environment as *const Closure);
    closure(Composer(composer))
}

unsafe extern "C" fn drop_closure(environment: *mut u8) {
    drop(Box::from_raw(environment as *mut Closure));
}

/// A `(Int) -> Unit` function value that calls `body` with the composer it is called with.
pub fn content(body: impl Fn(Composer) + 'static) -> FunctionValue {
    CONTENT_SLOT.store(call_closure as *const () as usize, Ordering::Release);
    let closure: Closure = Box::new(body);
    FunctionValue {
        descriptor: &CONTENT_DESCRIPTOR,
        environment: Box::into_raw(Box::new(closure)) as *mut u8,
    }
}

type Effect = Box<dyn Fn()>;

/// The code slot shared by every [`effect`] value.
static EFFECT_SLOT: AtomicUsize = AtomicUsize::new(0);

static EFFECT_DESCRIPTOR: Descriptor = Descriptor {
    slot: &EFFECT_SLOT,
    drop_environment: Some(drop_effect),
    equals_environment: None,
    duplicate_environment: None,
};

unsafe extern "C" fn call_effect(environment: *mut u8) {
    let effect = &*(environment as *const Effect);
    effect()
}

unsafe extern "C" fn drop_effect(environment: *mut u8) {
    drop(Box::from_raw(environment as *mut Effect));
}

/// A `() -> Unit` function value that calls `body`: a side effect or a handler.
pub fn effect(body: impl Fn() + 'static) -> FunctionValue {
    EFFECT_SLOT.store(call_effect as *const () as usize, Ordering::Release);
    let effect: Effect = Box::new(body);
    FunctionValue {
        descriptor: &EFFECT_DESCRIPTOR,
        environment: Box::into_raw(Box::new(effect)) as *mut u8,
    }
}

/// A static descriptor whose slot holds `code` and whose environment needs no drop. Leaked
/// once per call, so tests make it before what they measure.
pub fn descriptor(code: unsafe extern "C" fn(*mut u8, u64)) -> &'static Descriptor {
    let slot: &'static AtomicUsize =
        Box::leak(Box::new(AtomicUsize::new(code as *const () as usize)));
    Box::leak(Box::new(Descriptor {
        slot,
        drop_environment: None,
        equals_environment: None,
        duplicate_environment: None,
    }))
}

/// A function value of `descriptor` holding `word` as its inline environment: building it
/// allocates nothing.
pub fn inline_function(descriptor: &'static Descriptor, word: u64) -> FunctionValue {
    FunctionValue {
        descriptor,
        environment: std::ptr::without_provenance_mut(word as usize),
    }
}

/// The inline word of a function value built by [`inline_function`], as its code receives it.
pub fn inline_word(environment: *mut u8) -> u64 {
    environment as usize as u64
}

unsafe extern "C" fn drop_value<T>(value: *mut u8) {
    std::ptr::drop_in_place(value as *mut T)
}

unsafe extern "C" fn equal_values<T: PartialEq>(a: *const u8, b: *const u8) -> u8 {
    (*(a as *const T) == *(b as *const T)) as u8
}

static FINGERPRINTS: AtomicU64 = AtomicU64::new(1);

/// A slot type record for `T`, named `name`, with a fingerprint of its own, a drop entry when
/// `T` needs a drop and an equality entry over `PartialEq`. Leaked.
pub fn slot_type<T: PartialEq + 'static>(name: &'static str) -> &'static SlotType {
    let fingerprint = FINGERPRINTS.fetch_add(1, Ordering::Relaxed);
    slot_type_with_fingerprint::<T>(name, [fingerprint, 0x5107_7e57, 0, 0], true)
}

/// A slot type record for `T` with the given fingerprint, with or without an equality entry.
pub fn slot_type_with_fingerprint<T: PartialEq + 'static>(
    name: &'static str,
    fingerprint: [u64; 4],
    equality: bool,
) -> &'static SlotType {
    Box::leak(Box::new(SlotType {
        fingerprint,
        size: std::mem::size_of::<T>(),
        alignment: std::mem::align_of::<T>(),
        drop: std::mem::needs_drop::<T>().then_some(drop_value::<T> as _),
        equals: equality.then_some(equal_values::<T> as _),
        name: name.as_ptr(),
        name_length: name.len(),
    }))
}

/// Leaves a group in this thread's group arena that no composition holds.
pub fn leak_group() {
    crate::thread::with_state(|state| {
        state.groups.insert(crate::group::Group::new(
            crate::group::GroupKind::Replaceable,
            0,
            0,
            crate::handle::Handle::new(0, 0),
        ));
    });
}

/// Leaves a word slot in this thread's release queue that no pass releases.
pub fn leak_slot() {
    crate::thread::with_state(|state| {
        state
            .releases
            .push_back(crate::thread::Release::Slot(crate::slot::Slot::Word {
                tag: 0,
                value: 0,
            }));
    });
}

/// Leaves a state cell in this thread's cell arena that nothing owns. Its value is never read
/// or dropped.
pub fn leak_cell() {
    let record = slot_type::<i64>("Int");
    crate::thread::with_state(|state| {
        state.cells.insert(crate::state::Cell {
            record,
            value: std::ptr::null_mut(),
            flag: crate::state::Flag::Free,
            readers: Default::default(),
            released: false,
        });
    });
}
