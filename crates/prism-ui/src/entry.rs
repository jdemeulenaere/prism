//! The `extern "C"` entry points of the UI toolkit (D2, D3, D15, D17 and D23 of `ui-toolkit`),
//! each listed in [`crate::UI_FUNCTIONS`] with its machine signature.
//!
//! Each is a thin wrapper over the Rust API ([`crate::setters`], [`crate::app`]) that decodes
//! its words and addresses and turns an error into the runtime error it names, without a
//! location: an entry point has none. Node and handler handles are one-word intrinsic structs,
//! passed as their words; strings and modifiers are passed by address and borrowed; function
//! values passed by address are taken over, as a Prism callee owns a value moved into it.

use crate::app::run_app;
use crate::error::{raise, UiResult};
use crate::handle::{HandlerId, NodeId};
use crate::handlers::HandlerKind;
use crate::layout;
use crate::modifier::ModifierMirror;
use crate::runtime::{
    AlignmentMirror, ArrangementMirror, ConstraintsMirror, ParentDataMirror, PlaceableMirror,
};
use crate::setters;
use prism_runtime::{FunctionValue, PrismString};

/// The value of `result`, or the runtime error it holds.
fn or_raise<T>(result: UiResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => raise(&error),
    }
}

/// Sets a text's text or a text field's value.
///
/// # Safety
///
/// `text` must point at a string.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_set_text(node: u64, text: *const PrismString) {
    or_raise(setters::set_text(NodeId(node), (*text).as_str()))
}

/// Sets a node's text style: a colour's `argb` word and a font size, either unspecified.
#[no_mangle]
pub extern "C" fn prism_ui_set_text_style(node: u64, color: i64, font_size: i64) {
    or_raise(setters::set_text_style(NodeId(node), color, font_size))
}

/// Sets a node's modifier chain.
///
/// # Safety
///
/// `modifier` must point at a `Modifier`.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_set_modifier(node: u64, modifier: *const u8) {
    let modifier = &*(modifier as *const ModifierMirror);
    or_raise(setters::set_modifier(NodeId(node), modifier))
}

/// Sets a row's or column's arrangement: its code and spacing.
#[no_mangle]
pub extern "C" fn prism_ui_set_arrangement(node: u64, code: i64, space: i64) {
    or_raise(setters::set_arrangement(NodeId(node), code, space))
}

/// Sets a box's, row's or column's alignment code.
#[no_mangle]
pub extern "C" fn prism_ui_set_alignment(node: u64, code: i64) {
    or_raise(setters::set_alignment(NodeId(node), code))
}

/// Sets a node's content colour.
#[no_mangle]
pub extern "C" fn prism_ui_set_content_color(node: u64, color: i64) {
    or_raise(setters::set_content_color(NodeId(node), color))
}

/// Sets whether a text field is enabled.
#[no_mangle]
pub extern "C" fn prism_ui_set_enabled(node: u64, enabled: u8) {
    or_raise(setters::set_enabled(NodeId(node), enabled != 0))
}

/// Gives a text field its value-change handler.
#[no_mangle]
pub extern "C" fn prism_ui_set_on_value_change(node: u64, handler: u64) {
    or_raise(setters::set_on_value_change(
        NodeId(node),
        HandlerId(handler),
    ))
}

/// Registers a `clickable`'s `() -> Unit` handler, taken over, and gives its id.
///
/// # Safety
///
/// `handler` must point at a `() -> Unit` function value the caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_register_click(handler: *mut u8) -> u64 {
    let handler = FunctionValue::read_from(handler);
    or_raise(setters::register(HandlerKind::Click, handler)).word()
}

/// Registers a `toggleable`'s `(Bool) -> Unit` handler, taken over, and gives its id.
///
/// # Safety
///
/// `handler` must point at a `(Bool) -> Unit` function value the caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_register_toggle(handler: *mut u8) -> u64 {
    let handler = FunctionValue::read_from(handler);
    or_raise(setters::register(HandlerKind::Toggle, handler)).word()
}

/// Registers a text field's `(String) -> Unit` handler, taken over, and gives its id.
///
/// # Safety
///
/// `handler` must point at a `(String) -> Unit` function value the caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_register_text(handler: *mut u8) -> u64 {
    let handler = FunctionValue::read_from(handler);
    or_raise(setters::register(HandlerKind::Text, handler)).word()
}

/// `HandlerId`'s equality: 1 when the ids hold equal handlers.
#[no_mangle]
pub extern "C" fn prism_ui_handlers_equal(a: u64, b: u64) -> u8 {
    setters::handlers_equal(HandlerId(a), HandlerId(b)) as u8
}

/// Ends the program: a seventeenth element was appended to a modifier chain.
#[no_mangle]
pub extern "C" fn prism_ui_modifier_overflow() -> ! {
    raise(&setters::modifier_overflow())
}

/// Registers a layout's measure policy, taken over, and gives its id (D8 of
/// `layout-primitive`).
///
/// # Safety
///
/// `policy` must point at a `MeasureScope.(&List<Measurable>, Constraints) -> MeasureResult`
/// function value the caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_register_measure_policy(policy: *mut u8) -> u64 {
    let policy = FunctionValue::read_from(policy);
    or_raise(setters::register_measure_policy(policy)).word()
}

/// Gives a layout node its measure policy.
#[no_mangle]
pub extern "C" fn prism_ui_set_measure_policy(node: u64, policy: u64) {
    or_raise(setters::set_measure_policy(NodeId(node), HandlerId(policy)))
}

/// `Measurable.measure(constraints)`: measures a child of the running policy's node and
/// writes its `Placeable` to `result` (D6 of `layout-primitive`).
///
/// # Safety
///
/// `result` must point at storage for a `Placeable`, and `constraints` at a `Constraints`.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_measure(
    result: *mut PlaceableMirror,
    node: u64,
    constraints: *const ConstraintsMirror,
) {
    let size = or_raise(layout::measure_child(NodeId(node), (*constraints).into()));
    result.write(PlaceableMirror {
        node,
        width: size.width,
        height: size.height,
    });
}

/// The `Placeable` of a child measured by the running policy (D9 of `layout-primitive`).
///
/// # Safety
///
/// `result` must point at storage for a `Placeable`.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_measured(result: *mut PlaceableMirror, node: u64) {
    let size = or_raise(layout::measured(NodeId(node)));
    result.write(PlaceableMirror {
        node,
        width: size.width,
        height: size.height,
    });
}

/// The start of `layout(width, height) { ... }` in the policy of `node`.
#[no_mangle]
pub extern "C" fn prism_ui_begin_placement(node: u64, width: i64, height: i64) {
    or_raise(layout::begin_placement(NodeId(node), width, height))
}

/// The end of `layout`'s placement block.
#[no_mangle]
pub extern "C" fn prism_ui_end_placement(node: u64) {
    or_raise(layout::end_placement(NodeId(node)))
}

/// `Placeable.place(x, y)` in the placement block of `parent`'s policy.
#[no_mangle]
pub extern "C" fn prism_ui_place(parent: u64, child: u64, x: i64, y: i64) {
    or_raise(layout::place(NodeId(parent), NodeId(child), x, y))
}

/// A row's or column's arrangement, as its policy reads it.
///
/// # Safety
///
/// `result` must point at storage for an `Arrangement`.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_arrangement(result: *mut ArrangementMirror, node: u64) {
    let arrangement = or_raise(layout::arrangement(NodeId(node)));
    let (code, space) = arrangement.words();
    result.write(ArrangementMirror { code, space });
}

/// A box's, row's or column's alignment, as its policy reads it.
///
/// # Safety
///
/// `result` must point at storage for an `Alignment`.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_alignment(result: *mut AlignmentMirror, node: u64) {
    let alignment = or_raise(layout::alignment(NodeId(node)));
    result.write(AlignmentMirror {
        code: alignment.code(),
    });
}

/// A child's parent data: its first `weight` and `fill`, and its first `align`.
///
/// # Safety
///
/// `result` must point at storage for a `ParentData`.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_parent_data(result: *mut ParentDataMirror, node: u64) {
    let (weight, fill, alignment) = or_raise(layout::parent_data(NodeId(node)));
    result.write(ParentDataMirror {
        weight,
        alignment: alignment.map_or(-1, |alignment| alignment.code()),
        fill: fill as u8,
    });
}

/// Runs an app titled `title` of `width` × `height` composing `content`, `runApp`'s
/// composable content, which it takes over and calls with the composer and the force bit (D4
/// of `compose-library-in-prism`), and returns when its events end.
///
/// # Safety
///
/// `title` must point at a string, and `content` at a `compose () -> Unit` function value the
/// caller gives up.
#[no_mangle]
pub unsafe extern "C" fn prism_ui_run_app(
    title: *const PrismString,
    width: i64,
    height: i64,
    content: *mut u8,
) {
    let content = prism_compose::RootContent::Composable(FunctionValue::read_from(content));
    or_raise(run_app((*title).as_str(), width, height, content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::host::{Host, HostEvent};
    use crate::test_support::{inline_handler, run_in_child};
    use crate::text::FixedAdvanceMetrics;
    use prism_compose::test_support::content;
    use std::cell::Cell;
    use std::rc::Rc;

    struct NoEvents;

    impl Host for NoEvents {
        fn size(&self) -> (i64, i64) {
            (100, 100)
        }

        fn next_event(&mut self) -> Option<HostEvent> {
            None
        }

        fn present(&mut self, _list: &crate::draw::DisplayList) {}
    }

    fn stderr(output: &std::process::Output) -> String {
        assert_eq!(output.status.code(), Some(101));
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    /// The scenario "A stale node handle": the text setter, during a pass of an app, given a
    /// released node.
    fn stale_node_in_child() -> ! {
        let released = Rc::new(Cell::new(None));
        let seen = released.clone();
        let pass = Rc::new(Cell::new(0));
        let content = content(move |k| {
            pass.set(pass.get() + 1);
            if pass.get() == 1 {
                let node = k.start_node_group(1, 4).unwrap();
                seen.set(Some(node.0));
                k.end_group().unwrap();
            } else {
                let text = PrismString::of_static("a");
                unsafe { prism_ui_set_text(seen.get().unwrap(), &text) };
            }
        });
        let mut app =
            App::start(Box::new(NoEvents), Box::new(FixedAdvanceMetrics), content).unwrap();
        // The second pass stops starting the node group, whose node is released at its end,
        // and the third uses the handle.
        let root = prism_compose::Composition::root_scope(app.composition()).unwrap();
        prism_compose::invalidate(root);
        app.settle().unwrap();
        assert!(released.get().is_some());
        prism_compose::invalidate(root);
        let _ = app.settle();
        unreachable!("the setter ends the program")
    }

    #[test]
    fn a_stale_node_handle() {
        let output = run_in_child("entry::tests::a_stale_node_handle", stale_node_in_child);
        assert_eq!(stderr(&output), "panic: ui misuse: stale node handle\n");
    }

    fn unknown_kind_in_child() -> ! {
        let content = content(|k| {
            k.start_node_group(1, 42).unwrap();
        });
        let _ = App::start(Box::new(NoEvents), Box::new(FixedAdvanceMetrics), content);
        unreachable!("the applier ends the program")
    }

    #[test]
    fn an_unknown_node_kind() {
        let output = run_in_child("entry::tests::an_unknown_node_kind", unknown_kind_in_child);
        assert_eq!(stderr(&output), "panic: ui misuse: unknown node kind 42\n");
    }

    fn outside_composition_in_child() -> ! {
        prism_ui_set_text_style(1, 0, 16);
        unreachable!()
    }

    #[test]
    fn a_node_property_set_outside_composition() {
        let output = run_in_child(
            "entry::tests::a_node_property_set_outside_composition",
            outside_composition_in_child,
        );
        assert_eq!(
            stderr(&output),
            "panic: ui misuse: node property set outside composition\n"
        );
    }

    fn register_without_app_in_child() -> ! {
        let mut handler = std::mem::ManuallyDrop::new(inline_handler(0));
        unsafe { prism_ui_register_click((&mut *handler as *mut FunctionValue).cast()) };
        unreachable!()
    }

    #[test]
    fn registering_with_no_app() {
        let output = run_in_child(
            "entry::tests::registering_with_no_app",
            register_without_app_in_child,
        );
        assert_eq!(
            stderr(&output),
            "panic: ui misuse: modifier handlers are made only while runApp runs\n"
        );
    }

    fn overflow_in_child() -> ! {
        prism_ui_modifier_overflow()
    }

    #[test]
    fn a_seventeenth_element() {
        let output = run_in_child("entry::tests::a_seventeenth_element", overflow_in_child);
        assert_eq!(
            stderr(&output),
            "panic: ui misuse: a modifier chain holds more than 16 elements\n"
        );
    }

    fn no_host_in_child() -> ! {
        crate::host::configure(crate::host::HostConfiguration::None);
        let title = PrismString::of_static("Prism");
        let prism_compose::RootContent::Composable(content) =
            prism_compose::test_support::composable_content(|_, _| {})
        else {
            unreachable!()
        };
        let mut content = std::mem::ManuallyDrop::new(content);
        unsafe { prism_ui_run_app(&title, 10, 10, (&mut *content as *mut FunctionValue).cast()) };
        unreachable!()
    }

    #[test]
    fn no_window_and_no_headless_mode() {
        let output = run_in_child(
            "entry::tests::no_window_and_no_headless_mode",
            no_host_in_child,
        );
        assert_eq!(
            stderr(&output),
            "panic: ui misuse: no window system is available; run with --headless\n"
        );
    }

    #[test]
    fn handlers_compare_through_the_entry_point() {
        crate::thread::with_state(|state| {
            state.app = Some(crate::thread::RunningApp { composition: None })
        });
        let a = setters::register(HandlerKind::Click, inline_handler(3)).unwrap();
        let b = setters::register(HandlerKind::Click, inline_handler(3)).unwrap();
        let c = setters::register(HandlerKind::Click, inline_handler(4)).unwrap();
        assert_eq!(prism_ui_handlers_equal(a.word(), b.word()), 1);
        assert_eq!(prism_ui_handlers_equal(a.word(), c.word()), 0);
        crate::thread::with_state(|state| {
            let crate::thread::State {
                handlers, drops, ..
            } = state;
            handlers.release_all(drops);
            state.app = None;
        });
        crate::thread::release_dropped();
    }

    /// The scenario "Measuring with no layout running": the measure entry point, called by a
    /// click handler of a running app with a child of a laid-out column.
    fn measure_outside_layout_in_child() -> ! {
        use crate::test_support::{clickable, node_group};
        let child = Rc::new(Cell::new(0u64));
        let seen = child.clone();
        let content = content(move |k| {
            let column = node_group(k, 1, 1);
            let text = node_group(k, 2, 4);
            setters::set_text(text, "a").unwrap();
            seen.set(text.word());
            k.end_group().unwrap();
            let target = child.clone();
            let probe = crate::test_support::handler("measure", move || {
                let constraints = crate::runtime::ConstraintsMirror::from(
                    crate::geometry::Constraints::up_to(100, 100),
                );
                let mut placeable = std::mem::MaybeUninit::uninit();
                unsafe { prism_ui_measure(placeable.as_mut_ptr(), target.get(), &constraints) };
            });
            let handler = setters::register(HandlerKind::Click, probe).unwrap();
            let chain = ModifierMirror::encode(&[
                crate::test_support::size(100, 100),
                clickable(handler, true, None),
            ]);
            setters::set_modifier(column, &chain).unwrap();
            k.end_group().unwrap();
        });
        let mut app =
            App::start(Box::new(NoEvents), Box::new(FixedAdvanceMetrics), content).unwrap();
        app.settle().unwrap();
        app.handle(HostEvent::Press(50, 50)).unwrap();
        app.handle(HostEvent::Release(50, 50)).unwrap();
        unreachable!("the entry point ends the program")
    }

    #[test]
    fn measuring_with_no_layout_running() {
        let output = run_in_child(
            "entry::tests::measuring_with_no_layout_running",
            measure_outside_layout_in_child,
        );
        assert_eq!(
            stderr(&output),
            "panic: ui misuse: a measurable was measured outside its parent's measure policy\n"
        );
    }

    /// The scenario "A policy that never calls layout": a policy that measures its child and
    /// returns.
    fn no_layout_in_child() -> ! {
        use crate::node::NodeKind;
        use crate::test_support::{probe_policy, TreeBuilder};
        let mut b = TreeBuilder::new(100, 100);
        let root = b.root();
        let node = b.add(root, NodeKind::Layout);
        b.text(node, "a");
        b.set_policy(
            node,
            probe_policy(|_, measurables, c| {
                crate::layout::measure_child(NodeId(measurables[0]), c).unwrap()
            }),
        );
        b.lay_out();
        unreachable!("the layout pass ends the program")
    }

    #[test]
    fn a_policy_that_never_calls_layout() {
        let output = run_in_child(
            "entry::tests::a_policy_that_never_calls_layout",
            no_layout_in_child,
        );
        assert_eq!(
            stderr(&output),
            "panic: ui misuse: a measure policy returned without calling layout\n"
        );
    }
}
