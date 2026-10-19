//! What this crate takes from `prism-runtime` and `prism-compose` for running program code:
//! calling handler function values with their closure code signatures, their keepability,
//! duplicates and environment equality (D9 of `compose-transform`), the composition of the
//! thread's active pass, running a handler as one of a composition (D12 of `state-handles`),
//! and writing to the program's output. One place for these seams, so that the rest of the
//! crate does not depend on how the two runtimes spell them.

use prism_compose::Handle;
use prism_runtime::{FunctionValue, PrismString};

/// Calls a `() -> Unit` function value.
///
/// # Safety
///
/// `function` must be a live function value of type `() -> Unit`.
pub unsafe fn call_unit(function: &FunctionValue) {
    function.call_unit()
}

/// Calls a `(Bool) -> Unit` function value with `value`.
///
/// # Safety
///
/// `function` must be a live function value of type `(Bool) -> Unit`.
pub unsafe fn call_with_bool(function: &FunctionValue, value: bool) {
    function.call_with_bool(value)
}

/// Calls a `(String) -> Unit` function value with `value`, which the callee owns from then on.
///
/// # Safety
///
/// `function` must be a live function value of type `(String) -> Unit`.
pub unsafe fn call_with_string(function: &FunctionValue, value: PrismString) {
    function.call_with_string(value)
}

/// Whether a handler may be given to a second node, by a duplicate of its environment: its
/// descriptor has an environment equality entry, so it owns only `Copy` captures.
pub fn is_keepable(function: &FunctionValue) -> bool {
    // SAFETY: a handler's function value is live while its entry holds it.
    unsafe { function.is_keepable() }
}

/// A duplicate of a keepable handler, which owns its own environment; `None` when the handler
/// is not keepable.
pub fn duplicate(function: &FunctionValue) -> Option<FunctionValue> {
    // SAFETY: a handler's function value is live while its entry holds it.
    unsafe { function.duplicate() }
}

/// Whether two handlers have the same code and equal captures: the same descriptor, and
/// environments that its equality entry finds equal. A handler that is not keepable is equal to
/// no other.
pub fn environments_equal(a: &FunctionValue, b: &FunctionValue) -> bool {
    // SAFETY: both function values are live while their entries hold them.
    unsafe { a.equals(b) }
}

/// The composition of the pass running on this thread, if one is.
pub fn active_composition() -> Option<Handle> {
    prism_compose::active_composition()
}

/// Runs `handler` as a handler of `composition`, so that the cells it creates belong to it.
pub fn run_handler(composition: Handle, handler: impl FnOnce()) {
    prism_compose::Composition::run_handler(composition, handler)
}

/// Appends `text` to the program's output, in order with what the program prints.
pub fn write_output(text: &str) {
    prism_runtime::write_output(text)
}

/// Bitwise copy of a function value the handler table keeps owning, to call it outside the
/// table (D2: a handler's function value is copied out of its entry before it is called).
pub fn borrowed(function: &FunctionValue) -> FunctionValue {
    FunctionValue {
        descriptor: function.descriptor,
        environment: function.environment,
    }
}
