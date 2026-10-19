//! The `extern "C"` entry points of the compose runtime (D2, D3, D6, D10 to D12 of
//! `compose-runtime`), each listed in [`crate::COMPOSE_FUNCTIONS`] with its machine signature.
//!
//! Each is a thin wrapper over the Rust API that validates its handles, which are plain 64-bit
//! words (a composer is the active pass's identifier), and turns a misuse into the runtime
//! error `composition misuse: <detail>`, which writes no location: an entry point has none.
//! Function values passed by address are taken over, as a Prism callee owns a value moved into
//! it. Compositions started here use the built-in node tree applier.

use crate::applier::{NodeHandle, NodeTree};
use crate::composer::{invalidate, ComposeResult, Composer, Misuse};
use crate::composition::Composition;
use crate::group::GroupKind;
use crate::handle::Handle;
use crate::slot::SlotType;
use prism_runtime::{panic_without_location, FunctionValue, PanicKind, PrismString};

/// The value of `result`, or the runtime error of its misuse.
fn or_misuse<T>(result: ComposeResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(Misuse(detail)) => panic_without_location(PanicKind::CompositionMisuse, detail),
    }
}

/// Starts a composition of `content` with the node tree applier and runs its first pass.
///
/// # Safety
///
/// `content` must point at a `(Int) -> Unit` function value the caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_compose_start(content: *mut u8) -> u64 {
    let content = FunctionValue::read_from(content);
    or_misuse(Composition::start(Box::new(NodeTree::new()), content)).word()
}

/// Replaces a composition's content and recomposes its root.
///
/// # Safety
///
/// `content` must point at a `(Int) -> Unit` function value the caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_compose_set_content(composition: u64, content: *mut u8) {
    let content = FunctionValue::read_from(content);
    or_misuse(Composition::set_content(Handle(composition), content))
}

/// Runs a pass when a scope of the composition is invalid; 1 when it ran one.
#[no_mangle]
pub extern "C" fn prism_compose_recompose(composition: u64) -> u8 {
    or_misuse(Composition::recompose(Handle(composition))) as u8
}

/// Disposes a composition.
#[no_mangle]
pub extern "C" fn prism_compose_dispose(composition: u64) {
    or_misuse(Composition::dispose(Handle(composition)))
}

/// The root scope of a composition.
#[no_mangle]
pub extern "C" fn prism_compose_root_scope(composition: u64) -> u64 {
    or_misuse(Composition::root_scope(Handle(composition))).word()
}

/// Writes the dump of a composition, a new string, to `destination`.
///
/// # Safety
///
/// `destination` must point at writable memory for a string.
#[no_mangle]
pub unsafe extern "C" fn prism_compose_dump(destination: *mut PrismString, composition: u64) {
    let text = or_misuse(Composition::dump(Handle(composition)));
    destination.write(PrismString::owned(&text));
}

/// Starts a replaceable group of `key`.
#[no_mangle]
pub extern "C" fn prism_compose_start_group(composer: u64, key: i64) {
    or_misuse(Composer(composer).start_group(GroupKind::Replaceable, key, 0))
}

/// Starts a movable group of `key` and `data_key`.
#[no_mangle]
pub extern "C" fn prism_compose_start_movable_group(composer: u64, key: i64, data_key: i64) {
    or_misuse(Composer(composer).start_group(GroupKind::Movable, key, data_key))
}

/// Starts a restart group of `key`.
#[no_mangle]
pub extern "C" fn prism_compose_start_restart_group(composer: u64, key: i64) {
    or_misuse(Composer(composer).start_group(GroupKind::Restart, key, 0))
}

/// Starts a node group of `key` whose node has `kind`, giving its node.
#[no_mangle]
pub extern "C" fn prism_compose_start_node_group(composer: u64, key: i64, kind: i64) -> u64 {
    let NodeHandle(node) = or_misuse(Composer(composer).start_node_group(key, kind));
    node
}

/// Ends a replaceable, movable or node group.
#[no_mangle]
pub extern "C" fn prism_compose_end_group(composer: u64) {
    or_misuse(Composer(composer).end_group())
}

/// Ends a restart group, which keeps `restart`.
///
/// # Safety
///
/// `restart` must point at a `(Int) -> Unit` function value the caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_compose_end_restart_group(composer: u64, restart: *mut u8) {
    let restart = FunctionValue::read_from(restart);
    or_misuse(Composer(composer).end_restart_group(restart))
}

/// 1 when the innermost restart group may skip its body.
#[no_mangle]
pub extern "C" fn prism_compose_skipping(composer: u64) -> u8 {
    or_misuse(Composer(composer).skipping()) as u8
}

/// Keeps the rest of the innermost group as it is.
#[no_mangle]
pub extern "C" fn prism_compose_skip_to_group_end(composer: u64) {
    or_misuse(Composer(composer).skip_to_group_end())
}

/// The current scope of the composer.
#[no_mangle]
pub extern "C" fn prism_compose_current_scope(composer: u64) -> u64 {
    or_misuse(Composer(composer).current_scope()).word()
}

/// Invalidates a scope; a word that names no live scope is ignored.
#[no_mangle]
pub extern "C" fn prism_compose_invalidate(scope: u64) {
    invalidate(Handle(scope))
}

/// 1 when the slot at the cursor is a word slot of `tag`.
#[no_mangle]
pub extern "C" fn prism_compose_has_word(composer: u64, tag: i64) -> u8 {
    or_misuse(Composer(composer).has_word(tag)) as u8
}

/// The word slot at the cursor, moving past it.
#[no_mangle]
pub extern "C" fn prism_compose_word(composer: u64) -> i64 {
    or_misuse(Composer(composer).word())
}

/// Writes a word slot of `tag` at the cursor.
#[no_mangle]
pub extern "C" fn prism_compose_set_word(composer: u64, tag: i64, value: i64) {
    or_misuse(Composer(composer).set_word(tag, value))
}

/// Compares `value` with the word slot of `tag` at the cursor, storing it when it differs.
#[no_mangle]
pub extern "C" fn prism_compose_changed_word(composer: u64, tag: i64, value: i64) -> u8 {
    or_misuse(Composer(composer).changed_word(tag, value)) as u8
}

/// The address of the value of the slot at the cursor when it has `slot_type`, else null.
///
/// # Safety
///
/// `slot_type` must point at a valid slot type record.
#[no_mangle]
pub unsafe extern "C" fn prism_compose_slot(
    composer: u64,
    slot_type: *const SlotType,
) -> *const u8 {
    or_misuse(Composer(composer).slot(slot_type))
}

/// Moves the value at `value` into the slot at the cursor.
///
/// # Safety
///
/// `slot_type` must be a valid record and `value` a value of it that the caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_compose_set_slot(
    composer: u64,
    slot_type: *const SlotType,
    value: *mut u8,
) {
    or_misuse(Composer(composer).set_slot(slot_type, value))
}

/// Compares the value at `value` with the slot at the cursor; 1 when it changed and was
/// stored.
///
/// # Safety
///
/// `slot_type` must be a valid record and `value` a value of it that the caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_compose_changed(
    composer: u64,
    slot_type: *const SlotType,
    value: *mut u8,
) -> u8 {
    or_misuse(Composer(composer).changed(slot_type, value)) as u8
}

/// Keeps an argument of a skippable composable in the slot at the cursor by the change bits
/// `passed` its caller gave it; the bits the body now knows, plus 4 when the value changed
/// (D8 of `compose-transform`).
///
/// # Safety
///
/// `slot_type` must be a valid record and `value` a value of it that the caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_compose_keep(
    composer: u64,
    slot_type: *const SlotType,
    value: *mut u8,
    passed: i64,
) -> i64 {
    or_misuse(Composer(composer).keep(slot_type, value, passed))
}

/// During a restart, the address of kept slot `index` of the restarted scope's group when it
/// has `slot_type`; the misuse `no slot to read` otherwise (D10 of `compose-transform`).
///
/// # Safety
///
/// `slot_type` must point at a valid slot type record.
#[no_mangle]
pub unsafe extern "C" fn prism_compose_kept_slot(
    composer: u64,
    index: i64,
    slot_type: *const SlotType,
) -> *const u8 {
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    or_misuse(Composer(composer).kept_slot(index, slot_type))
}
