//! What this crate takes from `prism-runtime` and `prism-compose` for running program code:
//! calling handler function values with their closure code signatures, their keepability,
//! duplicates and environment equality (D9 of `compose-transform`), the composition of the
//! thread's active pass, running a handler as one of a composition (D12 of `state-handles`),
//! and writing to the program's output. One place for these seams, so that the rest of the
//! crate does not depend on how the two runtimes spell them.

use crate::geometry::Constraints;
use prism_compose::Handle;
use prism_runtime::{FunctionValue, PrismList, PrismString};

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

/// `MeasureScope` of `/prism/compose/ui` as generated code lays it out: its node's word.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasureScopeMirror {
    pub node: u64,
}

/// `Constraints` of `/prism/compose/ui`: `minWidth`, `maxWidth`, `minHeight`, `maxHeight`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConstraintsMirror {
    pub min_width: i64,
    pub max_width: i64,
    pub min_height: i64,
    pub max_height: i64,
}

impl From<Constraints> for ConstraintsMirror {
    fn from(c: Constraints) -> ConstraintsMirror {
        ConstraintsMirror {
            min_width: c.min_width,
            max_width: c.max_width,
            min_height: c.min_height,
            max_height: c.max_height,
        }
    }
}

impl From<ConstraintsMirror> for Constraints {
    fn from(c: ConstraintsMirror) -> Constraints {
        Constraints {
            min_width: c.min_width,
            max_width: c.max_width,
            min_height: c.min_height,
            max_height: c.max_height,
        }
    }
}

/// `Measurable`: its node's word, as the lent buffer holds it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasurableMirror {
    pub node: u64,
}

/// `Placeable`: `node`, `width` and `height`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaceableMirror {
    pub node: u64,
    pub width: i64,
    pub height: i64,
}

/// `MeasureResult`: `width`, `height` and `node`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MeasureResultMirror {
    pub width: i64,
    pub height: i64,
    pub node: u64,
}

/// `LineProperties` of `/prism/compose/layout`: a row's or column's arrangement (`code` and
/// `space`) and alignment, read in one call by its policy.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinePropertiesMirror {
    pub code: i64,
    pub space: i64,
    pub alignment: i64,
}

/// `MeasuredChild` of `/prism/compose/ui`: a child's measured `width` and `height` and its own
/// alignment code (-1 for none), read in one call by a container's placement block.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeasuredChildMirror {
    pub width: i64,
    pub height: i64,
    pub alignment: i64,
}

/// `Alignment` of `/prism/compose/ui`: `code`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlignmentMirror {
    pub code: i64,
}

/// `ParentData` of `/prism/compose/ui`: `weight` (0.0 for none), `alignment` (-1 for none)
/// and `fill`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParentDataMirror {
    pub weight: f64,
    pub alignment: i64,
    pub fill: u8,
}

/// Calls a measure policy with its scope, its measurables and its constraints, and gives its
/// result (D8 of `layout-primitive`).
///
/// # Safety
///
/// `function` must be a live function value of type `MeasureScope.(&List<Measurable>,
/// Constraints) -> MeasureResult`, and `list` must describe live `Measurable` words that stay
/// untouched during the call.
pub unsafe fn call_measure_policy(
    function: &FunctionValue,
    scope: &MeasureScopeMirror,
    list: &PrismList,
    constraints: &ConstraintsMirror,
) -> MeasureResultMirror {
    let mut result = MeasureResultMirror::default();
    function.call_with_addresses(
        (&mut result as *mut MeasureResultMirror).cast(),
        (scope as *const MeasureScopeMirror).cast(),
        (list as *const PrismList).cast(),
        (constraints as *const ConstraintsMirror).cast(),
    );
    result
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

/// The code and environment word that identify a handler whose environment is that word and
/// owns nothing: a keepable closure (it has an environment equality entry) with neither a drop
/// nor a duplicate entry, which captures nothing or one `Copy` word. `None` for every other.
pub fn intern_key(function: &FunctionValue) -> Option<(usize, usize)> {
    // SAFETY: a handler's descriptor is static data of the program, live while it runs.
    let descriptor = unsafe { &*function.descriptor };
    let word_only = descriptor.equals_environment.is_some()
        && descriptor.drop_environment.is_none()
        && descriptor.duplicate_environment.is_none();
    word_only.then_some((function.descriptor as usize, function.environment as usize))
}

/// The composition of the pass running on this thread, if one is.
pub fn active_composition() -> Option<Handle> {
    prism_compose::active_composition()
}

/// Runs `handler` as a handler of `composition`, so that the cells it creates belong to it.
pub fn run_handler<R>(composition: Handle, handler: impl FnOnce() -> R) -> R {
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
