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
//! - [`leak_node`] leaves a node in the thread's tree that no node group owns and no parent
//!   holds, as a runtime bug would, for the allocation check's live-node report (D6 of
//!   `reactive-app-verification`).

use crate::geometry::Size;
use crate::handle::{HandlerId, NodeId};
use crate::handlers::{release_functions, HandlerKind, HandlerTable};
use crate::modifier::Element;
use crate::node::NodeKind;
use crate::properties::{Alignment, Arrangement, Color, Role, Shape, TextStyle};
use crate::tree::UiTree;
use prism_runtime::{Descriptor, FunctionValue, PrismString};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicUsize, Ordering};

thread_local! {
    static DROPS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static ON_DROP: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    static INLINE_CALLS: Cell<u64> = const { Cell::new(0) };
    static INLINE_LAST: Cell<u64> = const { Cell::new(0) };
}

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
}

impl TreeBuilder {
    /// A tree with a window of `width` × `height`.
    pub fn new(width: i64, height: i64) -> TreeBuilder {
        TreeBuilder {
            tree: UiTree::new(width, height),
            handlers: HandlerTable::new(),
            drops: Vec::new(),
        }
    }

    pub fn root(&self) -> NodeId {
        self.tree.root()
    }

    /// A node of `kind` appended to `parent`'s children.
    pub fn add(&mut self, parent: NodeId, kind: NodeKind) -> NodeId {
        let node = self.tree.create_kind(kind);
        let last = self.tree.node(parent).unwrap().last_child;
        self.tree.place_after(parent, last, node);
        node
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

/// Creates a text node in the thread's tree and never places or releases it.
pub fn leak_node() {
    crate::thread::with_state(|state| {
        state.tree.create_kind(NodeKind::Text);
    });
}
