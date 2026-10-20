//! Function values and trees built from Rust, for the tests and benchmarks of this crate. Not
//! for generated code, which builds its own.
//!
//! - [`handler`], [`bool_handler`] and [`text_handler`] wrap Rust closures in `() -> Unit`,
//!   `(Bool) -> Unit` and `(String) -> Unit` function values with boxed environments, whose
//!   drop entries record `drop <name>` (see [`drops`]), as an owning closure's would free its
//!   environment. They are not keepable.
//! - [`inline_handler`] builds a keepable `() -> Unit` value whose environment is one inline
//!   word and whose descriptor is static, as an escaping closure over one `Copy` handle is
//!   (`escaping-closures` D9): building, comparing, duplicating and dropping it allocates
//!   nothing. Its calls are counted ([`inline_calls`]).
//! - [`TreeBuilder`] builds trees with modifier chains without a composition, for layout,
//!   drawing and input tests and the benchmarks.
//! - [`policy`] gives the measure policy of a column, row, box or spacer as Rust code, a port of
//!   what `/prism/compose/layout` computes in Prism, calling the same Rust API behind the entry
//!   points a Prism policy calls (D17 of `layout-primitive`); [`TreeBuilder::add`] gives every
//!   node of those kinds its policy, and [`node_group`] does it in a composition.
//!   [`counting_policy`] is a column's policy that counts its calls ([`policy_calls`]), and
//!   [`probe_policy`] runs a Rust closure as a policy.
//! - [`element_measurer`] gives the steps of the built-in layout modifiers (`padding`, `size`,
//!   `defaultMinSize`, `fillMax*`, `offset`) as Rust code, a port of `/prism/compose/ui`'s
//!   `measureElement` through the same entry points (D7 of `layout-in-prism`): the element
//!   measurer of [`TreeBuilder`] trees and of the apps of [`crate::App::start`] and
//!   [`crate::app::run_app_with`].
//! - [`leak_node`] leaves a node in the thread's tree that no node group owns and no parent
//!   holds, as a runtime bug would, for the allocation check's live-node report (D6 of
//!   `reactive-app-verification`).

use crate::error::{raise, UiResult};
use crate::geometry::{Constraints, Size, UNBOUNDED};
use crate::handle::{HandlerId, NodeId};
use crate::handlers::{release_functions, HandlerKind, HandlerTable};
use crate::layout::{self, Layout, Position};
use crate::modifier::{Element, ElementMirror};
use crate::node::NodeKind;
use crate::properties::{Alignment, Arrangement, Color, Role, Shape, TextStyle};
use crate::runtime::{
    ConstraintsMirror, MeasurableMirror, MeasureResultMirror, MeasureScopeMirror,
};
use crate::text::TextMeasurer;
use crate::tree::UiTree;
use prism_runtime::{
    prism_rt_empty_environments_equal, Descriptor, FunctionValue, PrismList, PrismString,
};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! {
    static DROPS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static ON_DROP: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    static INLINE_CALLS: Cell<u64> = const { Cell::new(0) };
    static INLINE_LAST: Cell<u64> = const { Cell::new(0) };
    static POLICY_CALLS: Cell<u64> = const { Cell::new(0) };
    static PROBE: RefCell<Option<ProbeBody>> = const { RefCell::new(None) };
}

/// The body of a [`probe_policy`]: its scope, its lent measurables and its constraints.
type ProbeBody = std::rc::Rc<dyn Fn(Position, &[MeasurableMirror], Constraints) -> Size>;

/// Forgets the drops recorded on this thread.
pub fn reset_drops() {
    DROPS.with(|drops| drops.borrow_mut().clear());
}

/// The drops recorded on this thread, in order.
pub fn drops() -> Vec<String> {
    DROPS.with(|drops| drops.borrow().clone())
}

/// Runs `probe` inside the next recorded drop, to check what state a drop entry sees.
pub fn on_drop(probe: Box<dyn FnOnce()>) {
    ON_DROP.with(|slot| *slot.borrow_mut() = Some(probe));
}

fn record_drop(name: &str) {
    if let Some(probe) = ON_DROP.with(|slot| slot.borrow_mut().take()) {
        probe();
    }
    DROPS.with(|drops| drops.borrow_mut().push(format!("drop {}", name)));
}

/// An environment that records its drop.
struct Environment<F: ?Sized> {
    name: &'static str,
    body: Box<F>,
}

impl<F: ?Sized> Drop for Environment<F> {
    fn drop(&mut self) {
        record_drop(self.name);
    }
}

type UnitEnvironment = Environment<dyn Fn()>;
type BoolEnvironment = Environment<dyn Fn(bool)>;
type TextEnvironment = Environment<dyn Fn(&str)>;

static UNIT_SLOT: AtomicUsize = AtomicUsize::new(0);
static BOOL_SLOT: AtomicUsize = AtomicUsize::new(0);
static TEXT_SLOT: AtomicUsize = AtomicUsize::new(0);
static INLINE_SLOT: AtomicUsize = AtomicUsize::new(0);

static UNIT_DESCRIPTOR: Descriptor = Descriptor {
    slot: &UNIT_SLOT,
    drop_environment: Some(drop_unit),
    equals_environment: None,
    duplicate_environment: None,
};
static BOOL_DESCRIPTOR: Descriptor = Descriptor {
    slot: &BOOL_SLOT,
    drop_environment: Some(drop_bool),
    equals_environment: None,
    duplicate_environment: None,
};
static TEXT_DESCRIPTOR: Descriptor = Descriptor {
    slot: &TEXT_SLOT,
    drop_environment: Some(drop_text),
    equals_environment: None,
    duplicate_environment: None,
};
static INLINE_DESCRIPTOR: Descriptor = Descriptor {
    slot: &INLINE_SLOT,
    drop_environment: None,
    equals_environment: Some(inline_words_equal),
    duplicate_environment: None,
};

/// The environment equality entry of an inline environment of one `Copy` word, as generated
/// code gives a closure capturing one `Int` (D9 of `compose-transform`).
unsafe extern "C" fn inline_words_equal(a: *const u8, b: *const u8) -> u8 {
    (a == b) as u8
}

unsafe extern "C" fn call_unit(environment: *mut u8) {
    ((*(environment as *const UnitEnvironment)).body)()
}

unsafe extern "C" fn call_bool(environment: *mut u8, value: u8) {
    ((*(environment as *const BoolEnvironment)).body)(value != 0)
}

/// Takes the string the caller moved in, as closure code owns its parameters, and drops it.
unsafe extern "C" fn call_text(environment: *mut u8, value: *mut PrismString) {
    let mut value = std::ptr::read(value);
    ((*(environment as *const TextEnvironment)).body)(value.as_str());
    prism_runtime::prism_rt_string_drop(&mut value);
}

unsafe extern "C" fn call_inline(environment: *mut u8) {
    INLINE_CALLS.with(|calls| calls.set(calls.get() + 1));
    INLINE_LAST.with(|last| last.set(environment as usize as u64));
}

unsafe extern "C" fn drop_unit(environment: *mut u8) {
    drop(Box::from_raw(environment as *mut UnitEnvironment))
}

unsafe extern "C" fn drop_bool(environment: *mut u8) {
    drop(Box::from_raw(environment as *mut BoolEnvironment))
}

unsafe extern "C" fn drop_text(environment: *mut u8) {
    drop(Box::from_raw(environment as *mut TextEnvironment))
}

/// A `() -> Unit` handler running `body`, whose drop records `drop <name>`.
pub fn handler(name: &'static str, body: impl Fn() + 'static) -> FunctionValue {
    UNIT_SLOT.store(call_unit as *const () as usize, Ordering::Release);
    let environment: Box<UnitEnvironment> = Box::new(Environment {
        name,
        body: Box::new(body),
    });
    FunctionValue {
        descriptor: &UNIT_DESCRIPTOR,
        environment: Box::into_raw(environment) as *mut u8,
    }
}

/// A `(Bool) -> Unit` handler running `body`, whose drop records `drop <name>`.
pub fn bool_handler(name: &'static str, body: impl Fn(bool) + 'static) -> FunctionValue {
    BOOL_SLOT.store(call_bool as *const () as usize, Ordering::Release);
    let environment: Box<BoolEnvironment> = Box::new(Environment {
        name,
        body: Box::new(body),
    });
    FunctionValue {
        descriptor: &BOOL_DESCRIPTOR,
        environment: Box::into_raw(environment) as *mut u8,
    }
}

/// A `(String) -> Unit` handler running `body` with the new text, whose drop records
/// `drop <name>`.
pub fn text_handler(name: &'static str, body: impl Fn(&str) + 'static) -> FunctionValue {
    TEXT_SLOT.store(call_text as *const () as usize, Ordering::Release);
    let environment: Box<TextEnvironment> = Box::new(Environment {
        name,
        body: Box::new(body),
    });
    FunctionValue {
        descriptor: &TEXT_DESCRIPTOR,
        environment: Box::into_raw(environment) as *mut u8,
    }
}

/// A keepable `() -> Unit` handler holding `word` inline, which counts its calls.
pub fn inline_handler(word: u64) -> FunctionValue {
    INLINE_SLOT.store(call_inline as *const () as usize, Ordering::Release);
    FunctionValue {
        descriptor: &INLINE_DESCRIPTOR,
        environment: std::ptr::without_provenance_mut(word as usize),
    }
}

/// How many times inline handlers were called on this thread, and the word of the last one.
pub fn inline_calls() -> (u64, u64) {
    (INLINE_CALLS.with(Cell::get), INLINE_LAST.with(Cell::get))
}

/// `padding(all)`.
pub fn padding(all: i64) -> Element {
    Element::Padding {
        start: all,
        top: all,
        end: all,
        bottom: all,
    }
}

/// `padding(start, top, end, bottom)`.
pub fn padding_each(start: i64, top: i64, end: i64, bottom: i64) -> Element {
    Element::Padding {
        start,
        top,
        end,
        bottom,
    }
}

/// `size(width, height)`.
pub fn size(width: i64, height: i64) -> Element {
    Element::Size {
        width: Some(width),
        height: Some(height),
    }
}

/// `width(width)`.
pub fn width(width: i64) -> Element {
    Element::Size {
        width: Some(width),
        height: None,
    }
}

/// `height(height)`.
pub fn height(height: i64) -> Element {
    Element::Size {
        width: None,
        height: Some(height),
    }
}

/// `defaultMinSize(width, height)`, negative values unspecified.
pub fn default_min_size(width: i64, height: i64) -> Element {
    Element::DefaultMinSize {
        width: (width >= 0).then_some(width),
        height: (height >= 0).then_some(height),
    }
}

/// `fillMaxWidth(fraction)`, `fillMaxHeight(fraction)` or `fillMaxSize(fraction)`.
pub fn fill_max(width: bool, height: bool, fraction: f64) -> Element {
    Element::FillMax {
        width,
        height,
        fraction,
    }
}

/// `weight(weight, fill)`.
pub fn weight(weight: f64, fill: bool) -> Element {
    Element::Weight { weight, fill }
}

/// `align(alignment)`.
pub fn align(alignment: Alignment) -> Element {
    Element::Align(alignment)
}

/// `background(color, shape)`.
pub fn background(argb: i64, shape: Shape) -> Element {
    Element::Background {
        color: Color(argb),
        shape,
    }
}

/// `border(width, color, shape)`.
pub fn border(width: i64, argb: i64, shape: Shape) -> Element {
    Element::Border {
        width,
        color: Color(argb),
        shape,
    }
}

/// `clickable(enabled, role)` naming `handler`.
pub fn clickable(handler: HandlerId, enabled: bool, role: Option<Role>) -> Element {
    Element::Clickable {
        handler,
        enabled,
        role,
    }
}

/// `toggleable(value, enabled, role)` naming `handler`.
pub fn toggleable(handler: HandlerId, value: bool, enabled: bool, role: Option<Role>) -> Element {
    Element::Toggleable {
        handler,
        value,
        enabled,
        role,
    }
}

/// A tree built from Rust, with its handler table.
pub struct TreeBuilder {
    pub tree: UiTree,
    pub handlers: HandlerTable,
    drops: Vec<FunctionValue>,
    /// The element measurer its layouts use, [`element_measurer`] unless a test gives another.
    pub steps: fn() -> FunctionValue,
}

impl TreeBuilder {
    /// A tree with a window of `width` × `height`.
    pub fn new(width: i64, height: i64) -> TreeBuilder {
        TreeBuilder {
            tree: UiTree::new(width, height),
            handlers: HandlerTable::new(),
            drops: Vec::new(),
            steps: element_measurer,
        }
    }

    pub fn root(&self) -> NodeId {
        self.tree.root()
    }

    /// A node of `kind` appended to `parent`'s children, with its kind's test policy when it
    /// has one.
    pub fn add(&mut self, parent: NodeId, kind: NodeKind) -> NodeId {
        let node = self.tree.create_kind(kind);
        let last = self.tree.node(parent).unwrap().last_child;
        self.tree.place_after(parent, last, node);
        if let Some(policy) = policy(kind) {
            self.set_policy(node, policy);
        }
        node
    }

    /// Gives `node` the measure policy `function`.
    pub fn set_policy(&mut self, node: NodeId, function: FunctionValue) -> &mut Self {
        let id = self.handlers.register(HandlerKind::Measure, function);
        let equal = self
            .tree
            .measure_policy_pair(node, id)
            .is_some_and(|(named, current)| self.handlers.equal(named, current));
        self.tree
            .set_measure_policy(node, id, equal, &mut self.handlers, &mut self.drops)
            .unwrap();
        release_functions(&mut self.drops);
        self
    }

    /// Runs `f` with this tree and handler table as the thread's, as an app's are, and its
    /// element measurer as the app's, so that the layout pass and the policies and steps it
    /// calls reach them.
    pub fn installed<R>(&mut self, f: impl FnOnce() -> R) -> R {
        let mut steps = Some((self.steps)());
        let mut swap = |builder: &mut TreeBuilder| {
            crate::thread::with_state(|state| {
                std::mem::swap(&mut state.tree, &mut builder.tree);
                std::mem::swap(&mut state.handlers, &mut builder.handlers);
                std::mem::swap(&mut state.layout.element_measurer, &mut steps);
            })
        };
        swap(self);
        let result = f();
        swap(self);
        result
    }

    /// Lays the tree out with `layout`, measuring text with `measurer`: [`Layout::run`] on
    /// this tree.
    pub fn lay_out_with(&mut self, layout: &mut Layout, measurer: &dyn TextMeasurer) -> bool {
        self.installed(|| layout.run(measurer))
    }

    /// Lays the tree out with fixed advances.
    pub fn lay_out(&mut self) -> bool {
        self.lay_out_with(&mut Layout::new(), &crate::text::FixedAdvanceMetrics)
    }

    /// A text node showing `text`, appended to `parent`.
    pub fn text(&mut self, parent: NodeId, text: &str) -> NodeId {
        let node = self.add(parent, NodeKind::Text);
        self.tree.set_text(node, text);
        node
    }

    /// A text field showing `value`, appended to `parent`.
    pub fn text_field(&mut self, parent: NodeId, value: &str) -> NodeId {
        let node = self.add(parent, NodeKind::TextField);
        self.tree.set_text(node, value);
        node
    }

    /// Sets `node`'s chain, claiming its handlers.
    pub fn chain(&mut self, node: NodeId, elements: &[Element]) -> &mut Self {
        self.tree
            .set_chain_comparing(node, elements, &mut self.handlers, &mut self.drops)
            .unwrap();
        release_functions(&mut self.drops);
        self
    }

    pub fn style(&mut self, node: NodeId, argb: i64, font_size: i64) -> &mut Self {
        self.tree.set_text_style(
            node,
            TextStyle {
                color: Color(argb),
                font_size,
            },
        );
        self
    }

    pub fn arrangement(&mut self, node: NodeId, arrangement: Arrangement) -> &mut Self {
        self.tree.set_arrangement(node, arrangement);
        self
    }

    pub fn alignment(&mut self, node: NodeId, alignment: Alignment) -> &mut Self {
        self.tree.set_alignment(node, alignment);
        self
    }

    pub fn content_color(&mut self, node: NodeId, argb: i64) -> &mut Self {
        self.tree.set_content_color(node, Color(argb));
        self
    }

    pub fn enabled(&mut self, node: NodeId, enabled: bool) -> &mut Self {
        self.tree.set_enabled(node, enabled);
        self
    }

    /// Registers `function` as a handler of `kind`, pending.
    pub fn register(&mut self, kind: HandlerKind, function: FunctionValue) -> HandlerId {
        self.handlers.register(kind, function)
    }

    /// Gives a text field its value-change handler.
    pub fn on_value_change(&mut self, node: NodeId, handler: HandlerId) -> &mut Self {
        let equal = self
            .tree
            .value_change_pair(node, handler)
            .is_some_and(|(named, current)| self.handlers.equal(named, current));
        self.tree
            .set_on_value_change(node, handler, equal, &mut self.handlers, &mut self.drops)
            .unwrap();
        release_functions(&mut self.drops);
        self
    }

    /// The window size.
    pub fn window(&self) -> Size {
        self.tree.window_size()
    }
}

impl Drop for TreeBuilder {
    fn drop(&mut self) {
        self.tree.release_all(&mut self.handlers, &mut self.drops);
        self.handlers.release_all(&mut self.drops);
        release_functions(&mut self.drops);
    }
}

/// The nodes of [`counter_screen`].
#[derive(Debug, Clone, Copy)]
pub struct CounterScreen {
    pub column: NodeId,
    pub count: NodeId,
    pub button: NodeId,
    pub row: NodeId,
    pub label: NodeId,
    pub handler: HandlerId,
}

/// The counter screen of the `ui` scenarios, as its composables build it (D24):
/// `Column(Modifier.padding(16.dp), Arrangement.spacedBy(8.dp)) { Text(count); Button(onClick)
/// { Text("+") } }`, the button a box with the primary background in `CircleShape`, a
/// clickable with the role `Button` and a white content colour, holding a centred row.
pub fn counter_screen(
    builder: &mut TreeBuilder,
    count: &str,
    on_click: FunctionValue,
) -> CounterScreen {
    let root = builder.root();
    let column = builder.add(root, NodeKind::Column);
    builder
        .chain(column, &[padding(16)])
        .arrangement(column, Arrangement::SpacedBy(8));
    let count = builder.text(column, count);
    let button = builder.add(column, NodeKind::Box);
    let handler = builder.register(HandlerKind::Click, on_click);
    builder
        .chain(
            button,
            &[
                background(0xFF67_50A4, Shape::new(0, 50)),
                clickable(handler, true, Some(Role::Button)),
            ],
        )
        .content_color(button, 0xFFFF_FFFF);
    let row = builder.add(button, NodeKind::Row);
    builder
        .chain(row, &[default_min_size(58, 40), padding_each(24, 8, 24, 8)])
        .arrangement(row, Arrangement::Center)
        .alignment(row, Alignment::CENTER_VERTICALLY);
    let label = builder.text(row, "+");
    CounterScreen {
        column,
        count,
        button,
        row,
        label,
        handler,
    }
}

/// Runs `child` in a child process running only the test `test` (its path in the crate, as
/// `--exact` matches it), and gives the child's output: for the runtime errors that end the
/// process.
pub fn run_in_child(test: &str, child: fn() -> !) -> std::process::Output {
    const VARIABLE: &str = "PRISM_UI_CHILD_TEST";
    if std::env::var(VARIABLE).as_deref() == Ok(test) {
        child()
    }
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(VARIABLE, test)
        .output()
        .unwrap()
}

/// Starts a node group of `kind` keyed `key` in a composition of the running app, giving its
/// node its kind's test policy when it has one: what `Node` and `LayoutNode` do.
pub fn node_group(k: prism_compose::Composer, key: i64, kind: i64) -> NodeId {
    let node = NodeId(k.start_node_group(key, kind).unwrap().0);
    if let Some(policy) = NodeKind::from_code(kind).and_then(policy) {
        let id = crate::setters::register_measure_policy(policy).unwrap();
        crate::setters::set_measure_policy(node, id).unwrap();
    }
    node
}

/// The test policy of `kind`: a port of the Prism policy of a column, row, box or spacer.
pub fn policy(kind: NodeKind) -> Option<FunctionValue> {
    let (slot, descriptor, code): (&AtomicUsize, &'static Descriptor, PolicyCode) = match kind {
        NodeKind::Column => (&COLUMN_SLOT, &COLUMN_DESCRIPTOR, column_policy),
        NodeKind::Row => (&ROW_SLOT, &ROW_DESCRIPTOR, row_policy),
        NodeKind::Box => (&BOX_SLOT, &BOX_DESCRIPTOR, box_policy),
        NodeKind::Spacer => (&SPACER_SLOT, &SPACER_DESCRIPTOR, spacer_policy),
        _ => return None,
    };
    Some(capture_free(slot, descriptor, code))
}

/// A column's policy that counts its calls on this thread ([`policy_calls`]).
pub fn counting_policy() -> FunctionValue {
    capture_free(&COUNTING_SLOT, &COUNTING_DESCRIPTOR, counting_policy_code)
}

/// How many times [`counting_policy`] ran on this thread.
pub fn policy_calls() -> u64 {
    POLICY_CALLS.with(Cell::get)
}

/// A policy running `body` on this thread with its scope, its lent measurables and its
/// constraints: `body` calls the layout API itself, `layout` included, and gives the size it
/// gave. One body per thread, the last given, which may branch on the node for nested layouts.
pub fn probe_policy(
    body: impl Fn(Position, &[MeasurableMirror], Constraints) -> Size + 'static,
) -> FunctionValue {
    PROBE.with(|probe| *probe.borrow_mut() = Some(std::rc::Rc::new(body)));
    capture_free(&PROBE_SLOT, &PROBE_DESCRIPTOR, probe_policy_code)
}

/// Another function value of the probe policy, running the body last given to
/// [`probe_policy`] on this thread.
pub fn probe_policy_again() -> FunctionValue {
    capture_free(&PROBE_SLOT, &PROBE_DESCRIPTOR, probe_policy_code)
}

unsafe extern "C" fn probe_policy_code(
    result: *mut MeasureResultMirror,
    _: *mut u8,
    scope: *const MeasureScopeMirror,
    list: *const PrismList,
    constraints: *const ConstraintsMirror,
) {
    let (scope, measurables, c) = policy_arguments(scope, list, constraints);
    let body = PROBE
        .with(|probe| probe.borrow().clone())
        .expect("a probe policy's body");
    write_result(result, scope, body(scope, measurables, c));
}

/// A policy that measures every child under its constraints but places only the first, at the
/// content's top-left corner, as large as it: a layout hiding the rest (D7 of
/// `layout-primitive`).
pub fn first_only_policy() -> FunctionValue {
    probe_policy(|scope, measurables, c| {
        let sizes: Vec<Size> = measurables
            .iter()
            .map(|child| layout::measure_child(NodeId(child.node), c).unwrap())
            .collect();
        let size = sizes.first().copied().unwrap_or(Size::ZERO);
        layout::begin_placement(scope, size.width, size.height).unwrap();
        if let Some(first) = measurables.first() {
            layout::place(scope, Position::child(NodeId(first.node)), 0, 0).unwrap();
        }
        layout::end_placement(scope).unwrap();
        size
    })
}

/// The closure code of a measure policy: `(result, environment, scope, measurables,
/// constraints)`.
pub type PolicyCode = unsafe extern "C" fn(
    *mut MeasureResultMirror,
    *mut u8,
    *const MeasureScopeMirror,
    *const PrismList,
    *const ConstraintsMirror,
);

fn capture_free(
    slot: &'static AtomicUsize,
    descriptor: &'static Descriptor,
    code: PolicyCode,
) -> FunctionValue {
    slot.store(code as *const () as usize, Ordering::Release);
    FunctionValue {
        descriptor,
        environment: std::ptr::null_mut(),
    }
}

macro_rules! policy_descriptor {
    ($slot:ident, $descriptor:ident) => {
        static $slot: AtomicUsize = AtomicUsize::new(0);
        static $descriptor: Descriptor = Descriptor {
            slot: &$slot,
            drop_environment: None,
            equals_environment: Some(prism_rt_empty_environments_equal_entry),
            duplicate_environment: None,
        };
    };
}

unsafe extern "C" fn prism_rt_empty_environments_equal_entry(a: *const u8, b: *const u8) -> u8 {
    prism_rt_empty_environments_equal(a, b)
}

policy_descriptor!(COLUMN_SLOT, COLUMN_DESCRIPTOR);
policy_descriptor!(ROW_SLOT, ROW_DESCRIPTOR);
policy_descriptor!(BOX_SLOT, BOX_DESCRIPTOR);
policy_descriptor!(SPACER_SLOT, SPACER_DESCRIPTOR);
policy_descriptor!(COUNTING_SLOT, COUNTING_DESCRIPTOR);
policy_descriptor!(PROBE_SLOT, PROBE_DESCRIPTOR);

/// The value of `result`, or the runtime error it holds, as an entry point raises it.
fn or_raise<T>(result: UiResult<T>) -> T {
    result.unwrap_or_else(|error| raise(&error))
}

/// The scope, measurables and constraints a policy is called with.
///
/// # Safety
///
/// The pointers must be those the layout pass passes.
pub unsafe fn policy_arguments<'a>(
    scope: *const MeasureScopeMirror,
    list: *const PrismList,
    constraints: *const ConstraintsMirror,
) -> (Position, &'a [MeasurableMirror], Constraints) {
    let list = &*list;
    let measurables =
        std::slice::from_raw_parts(list.pointer as *const MeasurableMirror, list.size);
    let scope = Position::of_words((*scope).node, (*scope).element);
    (scope, measurables, (*constraints).into())
}

/// Writes the result of `layout` in `scope`, of `size`.
///
/// # Safety
///
/// `result` must be the policy's result address.
pub unsafe fn write_result(result: *mut MeasureResultMirror, scope: Position, size: Size) {
    result.write(MeasureResultMirror {
        width: size.width,
        height: size.height,
        node: scope.node.word(),
        element: scope.element as u64,
    });
}

unsafe extern "C" fn column_policy(
    result: *mut MeasureResultMirror,
    _: *mut u8,
    scope: *const MeasureScopeMirror,
    list: *const PrismList,
    constraints: *const ConstraintsMirror,
) {
    let (scope, measurables, c) = policy_arguments(scope, list, constraints);
    write_result(result, scope, measure_line(scope, measurables, c, true));
}

unsafe extern "C" fn row_policy(
    result: *mut MeasureResultMirror,
    _: *mut u8,
    scope: *const MeasureScopeMirror,
    list: *const PrismList,
    constraints: *const ConstraintsMirror,
) {
    let (scope, measurables, c) = policy_arguments(scope, list, constraints);
    write_result(result, scope, measure_line(scope, measurables, c, false));
}

unsafe extern "C" fn box_policy(
    result: *mut MeasureResultMirror,
    _: *mut u8,
    scope: *const MeasureScopeMirror,
    list: *const PrismList,
    constraints: *const ConstraintsMirror,
) {
    let (scope, measurables, c) = policy_arguments(scope, list, constraints);
    write_result(result, scope, measure_box(scope, measurables, c));
}

unsafe extern "C" fn spacer_policy(
    result: *mut MeasureResultMirror,
    _: *mut u8,
    scope: *const MeasureScopeMirror,
    list: *const PrismList,
    constraints: *const ConstraintsMirror,
) {
    let (scope, _, c) = policy_arguments(scope, list, constraints);
    let size = c.constrain(Size::ZERO);
    or_raise(layout::begin_placement(scope, size.width, size.height));
    or_raise(layout::end_placement(scope));
    write_result(result, scope, size);
}

unsafe extern "C" fn counting_policy_code(
    result: *mut MeasureResultMirror,
    environment: *mut u8,
    scope: *const MeasureScopeMirror,
    list: *const PrismList,
    constraints: *const ConstraintsMirror,
) {
    POLICY_CALLS.with(|calls| calls.set(calls.get() + 1));
    column_policy(result, environment, scope, list, constraints)
}

/// The constraints of one axis pair, main axis first, for rows (horizontal) and columns.
fn oriented(
    vertical: bool,
    main_min: i64,
    main_max: i64,
    cross_min: i64,
    cross_max: i64,
) -> Constraints {
    if vertical {
        Constraints {
            min_width: cross_min,
            max_width: cross_max,
            min_height: main_min,
            max_height: main_max,
        }
    } else {
        Constraints {
            min_width: main_min,
            max_width: main_max,
            min_height: cross_min,
            max_height: cross_max,
        }
    }
}

fn main_of(vertical: bool, size: Size) -> i64 {
    if vertical {
        size.height
    } else {
        size.width
    }
}

fn cross_of(vertical: bool, size: Size) -> i64 {
    if vertical {
        size.width
    } else {
        size.height
    }
}

/// A box: children under the constraints loosened, the size of the largest at least the
/// minimum, each child placed by its own alignment or the box's.
fn measure_box(scope: Position, measurables: &[MeasurableMirror], c: Constraints) -> Size {
    let alignment = or_raise(layout::alignment(scope.node));
    let loose = c.loosened();
    let mut largest = Size::ZERO;
    for measurable in measurables {
        let size = or_raise(layout::measure_child(NodeId(measurable.node), loose));
        largest.width = largest.width.max(size.width);
        largest.height = largest.height.max(size.height);
    }
    let size = c.constrain(largest);
    or_raise(layout::begin_placement(scope, size.width, size.height));
    for measurable in measurables {
        let child = NodeId(measurable.node);
        let measured = or_raise(layout::measured(child));
        let (_, _, own) = or_raise(layout::parent_data(child));
        let own = own.unwrap_or(alignment);
        or_raise(layout::place(
            scope,
            Position::child(child),
            own.horizontal().offset(size.width - measured.width),
            own.vertical().offset(size.height - measured.height),
        ));
    }
    or_raise(layout::end_placement(scope));
    size
}

/// A column (`vertical`) or a row: unweighted children in order under what remains of the main
/// axis, then weighted children sharing what is left of a bounded main axis, then placement by
/// the arrangement and each child's alignment.
fn measure_line(
    scope: Position,
    measurables: &[MeasurableMirror],
    c: Constraints,
    vertical: bool,
) -> Size {
    let arrangement = or_raise(layout::arrangement(scope.node));
    let alignment = or_raise(layout::alignment(scope.node));
    let spacing = arrangement.spacing();
    let (main_max, cross_max) = if vertical {
        (c.max_height, c.max_width)
    } else {
        (c.max_width, c.max_height)
    };
    let bounded = main_max != UNBOUNDED;
    let count = measurables.len() as i64;
    let mut measured: i64 = 0;
    let mut used: i64 = 0;
    let mut cross: i64 = 0;
    let mut total_weight = 0.0;
    let mut weighted: i64 = 0;
    for measurable in measurables {
        let child = NodeId(measurable.node);
        let (weight, _, _) = or_raise(layout::parent_data(child));
        if bounded && weight > 0.0 {
            total_weight += weight;
            weighted += 1;
        } else {
            let remaining = if bounded {
                (main_max - used - spacing * measured).max(0)
            } else {
                UNBOUNDED
            };
            let size = or_raise(layout::measure_child(
                child,
                oriented(vertical, 0, remaining, 0, cross_max),
            ));
            used += main_of(vertical, size);
            cross = cross.max(cross_of(vertical, size));
            measured += 1;
        }
    }
    let gaps = spacing * (count - 1).max(0);
    if weighted > 0 {
        let remaining = (main_max - used - gaps).max(0);
        let mut given = 0;
        let mut seen = 0;
        for measurable in measurables {
            let child = NodeId(measurable.node);
            let (weight, fill, _) = or_raise(layout::parent_data(child));
            if weight > 0.0 {
                seen += 1;
                let share = if seen == weighted {
                    remaining - given
                } else {
                    let value = (remaining as f64 * (weight / total_weight)).floor();
                    if value.is_nan() {
                        0
                    } else {
                        value as i64
                    }
                };
                given += share;
                let main_min = if fill { share } else { 0 };
                let size = or_raise(layout::measure_child(
                    child,
                    oriented(vertical, main_min, share, 0, cross_max),
                ));
                used += main_of(vertical, size);
                cross = cross.max(cross_of(vertical, size));
            }
        }
    }
    let content = used + gaps;
    let size = if vertical {
        c.constrain(Size::new(cross, content))
    } else {
        c.constrain(Size::new(content, cross))
    };
    let (main_size, cross_size) = (main_of(vertical, size), cross_of(vertical, size));
    let (start, gap) = arrangement.start_and_gap(main_size - content, count);
    or_raise(layout::begin_placement(scope, size.width, size.height));
    let mut position = start;
    for measurable in measurables {
        let child = NodeId(measurable.node);
        let child_size = or_raise(layout::measured(child));
        let (_, _, own) = or_raise(layout::parent_data(child));
        let own = own.unwrap_or(alignment);
        let part = if vertical {
            own.horizontal()
        } else {
            own.vertical()
        };
        let along = part.offset(cross_size - cross_of(vertical, child_size));
        if vertical {
            or_raise(layout::place(
                scope,
                Position::child(child),
                along,
                position,
            ));
        } else {
            or_raise(layout::place(
                scope,
                Position::child(child),
                position,
                along,
            ));
        }
        position += main_of(vertical, child_size) + spacing + gap;
    }
    or_raise(layout::end_placement(scope));
    size
}

static STEP_SLOT: AtomicUsize = AtomicUsize::new(0);
static STEP_DESCRIPTOR: Descriptor = Descriptor {
    slot: &STEP_SLOT,
    drop_environment: None,
    equals_environment: Some(prism_rt_empty_environments_equal_entry),
    duplicate_environment: None,
};

/// The element measurer of Rust trees and apps: the steps of the built-in layout modifiers,
/// ported from `/prism/compose/ui`'s `measureElement` (D7 of `layout-in-prism`). Capture-free,
/// so building and dropping it costs nothing.
pub fn element_measurer() -> FunctionValue {
    STEP_SLOT.store(element_step as *const () as usize, Ordering::Release);
    FunctionValue {
        descriptor: &STEP_DESCRIPTOR,
        environment: std::ptr::null_mut(),
    }
}

/// The share of the maximum `fraction` gives, truncated toward zero, 0 for NaN, saturated: as
/// `Double.toInt()`.
fn fraction_of(maximum: i64, fraction: f64) -> i64 {
    let value = maximum as f64 * fraction;
    if value.is_nan() {
        0
    } else {
        value as i64
    }
}

/// The step of a built-in layout element: `(result, environment, scope, element, measurable,
/// constraints)`.
unsafe extern "C" fn element_step(
    result: *mut MeasureResultMirror,
    _: *mut u8,
    scope: *const MeasureScopeMirror,
    element: *const ElementMirror,
    measurable: *const MeasurableMirror,
    constraints: *const ConstraintsMirror,
) {
    let scope = Position::of_words((*scope).node, (*scope).element);
    let rest = Position::of_words((*measurable).node, (*measurable).element);
    let c: Constraints = (*constraints).into();
    let element = (*element).decode();
    // The constraints of the rest, and where it goes.
    let (inner, x, y) = match element {
        Some(Element::Padding {
            start,
            top,
            end,
            bottom,
        }) => (c.offset(-(start + end), -(top + bottom)), start, top),
        Some(Element::Size { width, height }) => {
            let mut inner = c;
            if let Some(width) = width {
                inner = inner.with_exact_width(width);
            }
            if let Some(height) = height {
                inner = inner.with_exact_height(height);
            }
            (inner, 0, 0)
        }
        Some(Element::DefaultMinSize { width, height }) => {
            let mut inner = c;
            if let (Some(width), 0) = (width, c.min_width) {
                inner.min_width = width.min(c.max_width);
            }
            if let (Some(height), 0) = (height, c.min_height) {
                inner.min_height = height.min(c.max_height);
            }
            (inner, 0, 0)
        }
        Some(Element::FillMax {
            width,
            height,
            fraction,
        }) => {
            let mut inner = c;
            if width && c.max_width != UNBOUNDED {
                inner = inner.with_exact_width(fraction_of(c.max_width, fraction));
            }
            if height && c.max_height != UNBOUNDED {
                inner = inner.with_exact_height(fraction_of(c.max_height, fraction));
            }
            (inner, 0, 0)
        }
        Some(Element::Offset { x, y }) => (c, x, y),
        _ => (c, 0, 0),
    };
    let measured = or_raise(layout::measure_position(rest, inner));
    let size = match element {
        Some(Element::Padding {
            start,
            top,
            end,
            bottom,
        }) => Size::new(
            c.clamp_width(measured.width + start + end),
            c.clamp_height(measured.height + top + bottom),
        ),
        _ => measured,
    };
    or_raise(layout::begin_placement(scope, size.width, size.height));
    or_raise(layout::place(scope, rest, x, y));
    or_raise(layout::end_placement(scope));
    write_result(result, scope, size);
}

/// Creates a text node in the thread's tree and never places or releases it.
pub fn leak_node() {
    crate::thread::with_state(|state| {
        state.tree.create_kind(NodeKind::Text);
    });
}
