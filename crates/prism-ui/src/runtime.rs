//! What this crate needs from `prism-runtime` and `prism-compose` that task group 3 of
//! `ui-toolkit` (and `state-handles`) will move there: calling handler function values with
//! their closure code signatures, their keepability, duplicates and environment equality, the
//! composition of the thread's active pass, running a handler as one of a composition, and
//! writing to the program's output. Each function names the task that replaces it, so the
//! integration step changes this file and none of its callers.

use prism_compose::Handle;
use prism_runtime::{FunctionValue, PrismString};
use std::sync::atomic::Ordering;

/// Calls a `() -> Unit` function value (task 3.3's `FunctionValue::call_unit`).
///
/// # Safety
///
/// `function` must be a live function value of type `() -> Unit`.
pub unsafe fn call_unit(function: &FunctionValue) {
    function.call()
}

/// Calls a `(Bool) -> Unit` function value with `value` (task 3.3's `call_with_bool`). A
/// `Bool` parameter is one byte, 0 or 1, as closure code takes it.
///
/// # Safety
///
/// `function` must be a live function value of type `(Bool) -> Unit`.
pub unsafe fn call_with_bool(function: &FunctionValue, value: bool) {
    let code = (*(*function.descriptor).slot).load(Ordering::Acquire);
    let code: unsafe extern "C" fn(*mut u8, u8) = std::mem::transmute(code);
    code(function.environment, value as u8)
}

/// Calls a `(String) -> Unit` function value with `value`, which the callee owns from then on
/// (task 3.3's `call_with_string`): the string is passed by address, as aggregates are, and
/// the callee drops it.
///
/// # Safety
///
/// `function` must be a live function value of type `(String) -> Unit`.
pub unsafe fn call_with_string(function: &FunctionValue, value: PrismString) {
    let code = (*(*function.descriptor).slot).load(Ordering::Acquire);
    let code: unsafe extern "C" fn(*mut u8, *mut PrismString) = std::mem::transmute(code);
    let mut value = std::mem::ManuallyDrop::new(value);
    code(function.environment, &mut *value)
}

/// Whether a handler may be given to a second node, by a duplicate of its environment (task
/// 3.3's `is_keepable`, over `compose-transform` D9's descriptor entries).
///
/// Until descriptors carry the environment equality and duplicate entries
/// (`compose-transform` task 7.4), a value is keepable exactly when its environment needs no
/// drop: it captures nothing, or holds its captures inline in the environment word, which
/// only `Copy` captures of one word do (`escaping-closures` D9).
pub fn is_keepable(function: &FunctionValue) -> bool {
    // SAFETY: a function value's descriptor is valid static data.
    unsafe { (*function.descriptor).drop_environment.is_none() }
}

/// A duplicate of a keepable handler, which owns its own environment (task 3.3's
/// `duplicate`): a copy of the environment word, until descriptors have a duplicate entry.
pub fn duplicate(function: &FunctionValue) -> Option<FunctionValue> {
    is_keepable(function).then_some(FunctionValue {
        descriptor: function.descriptor,
        environment: function.environment,
    })
}

/// Whether two handlers have the same code and equal captures (task 3.3's `equals`): the same
/// descriptor, and equal environments. Until descriptors have an environment equality entry,
/// only keepable values compare their environment words; any other pair is unequal.
pub fn environments_equal(a: &FunctionValue, b: &FunctionValue) -> bool {
    std::ptr::eq(a.descriptor, b.descriptor)
        && is_keepable(a)
        && is_keepable(b)
        && a.environment == b.environment
}

/// The composition of the pass running on this thread, if one is (task 3.4's
/// `prism_compose::active_composition`).
pub fn active_composition() -> Option<Handle> {
    prism_compose::thread::with_state(|state| state.active.map(|pass| pass.composition))
}

/// Runs `handler` as a handler of `composition`, so that the cells it creates belong to it
/// (`state-handles`' `Composition::run_handler`, its D12).
pub fn run_handler(composition: Handle, handler: impl FnOnce()) {
    prism_compose::Composition::run_handler(composition, handler)
}

/// Appends `text` to the program's output, in order with what the program prints (task 3.2's
/// `prism_runtime::write_output`).
pub fn write_output(text: &str) {
    let string = PrismString {
        pointer: text.as_ptr(),
        byte_length: text.len(),
        character_count: text.chars().count(),
        capacity: 0,
    };
    // SAFETY: the string points at `text`, which outlives the call, and owns nothing.
    unsafe { prism_runtime::prism_rt_print_string(&string) }
}

/// Bitwise copy of a function value the handler table keeps owning, to call it outside the
/// table (D2: a handler's function value is copied out of its entry before it is called).
pub fn borrowed(function: &FunctionValue) -> FunctionValue {
    FunctionValue {
        descriptor: function.descriptor,
        environment: function.environment,
    }
}
