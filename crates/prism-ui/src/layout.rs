//! Constraint layout (D11 of `ui-toolkit`): one measure-and-place pass over the tree, in
//! whole logical pixels, whose containers are measured by their measure policies
//! (`layout-primitive`) and whose layout modifiers are measurement steps (`layout-in-prism`).
//!
//! Measuring a node under constraints walks its modifier chain from the first element,
//! Compose's outermost, as Compose's chain of layout modifier nodes (D1 and D6 of
//! `layout-in-prism`):
//!
//! - a **layout element** (`padding`, `width`, `height`, `size`, `defaultMinSize`,
//!   `fillMax*`, `offset`) is a step: layout calls the app's element measurer, Prism code of
//!   `/prism/compose/ui`, with the element, the measurable of the rest of the chain and the
//!   constraints. The step measures the rest once, under constraints it chooses, and calls
//!   `layout` with the size of the chain from it, placing the rest at an offset from its own
//!   top-left corner;
//! - `weight` and `align` are parent data, read by the parent's policy, the first in a chain
//!   counting, and pass the constraints on, as drawing and input elements do;
//! - past the last element, the content is measured: natively for a text or a text field, by
//!   the window for its children (under its size, at its top-left corner), and by its
//!   **measure policy** for every other node (a column, row, box, spacer or user layout), a
//!   function value of the program kept in the handler table (D8 of `layout-primitive`).
//!
//! Once the node is measured, one walk over the chain sums the steps' offsets into each
//! position's origin, the content's included, and records at each drawing and input element
//! the bounds at its position: that origin and the size of the chain from it.
//!
//! **The measurement protocol** (D6 of `layout-primitive`, D5 of `layout-in-prism`). While a
//! policy or a step runs, the pass keeps a stack of measurement frames: the node and the chain
//! position the frame measures from (a step's element, or the chain's length for the content),
//! a fresh stamp, the content's origin, whether `layout` was called and whether placement is
//! running. The program calls back through the entry points: [`measure_position`] measures a
//! child of the innermost content frame's node, or the rest of the innermost step's chain,
//! once; [`begin_placement`] and [`end_placement`] bracket `layout`'s placement block; and
//! [`place`] positions what the frame measured. The children are lent to a policy as a
//! `&List<Measurable>` over a buffer of their words, one buffer per depth of nested policies,
//! reused across passes (D4). A step that did not place the rest of its chain is a misuse (D10
//! of `layout-in-prism`).
//!
//! **Positions.** A content policy places children before the steps around it have placed
//! the content: a child is placed at the content's origin of the node's last measurement plus
//! its offset, and when the steps move the content, the end of the measurement moves the
//! children it placed by as much.
//!
//! **Re-entrancy** (D14, D21). Policies and steps are program code, which call the entry
//! points, which use the thread's state. So no borrow of the thread's state is held while
//! program code runs: every step of the pass borrows the state for its own reads and writes
//! ([`with_state`]), and the pass's own state (frames, buffers, the text measurer of the run)
//! lives in the thread's state too ([`LayoutState`]).
//!
//! **The cache.** A node that is neither `needs_measure` nor `child_needs_measure` and is
//! measured with its last constraints returns its last size without visiting its subtree. A
//! node that is only `child_needs_measure` re-measures its dirty children with their last
//! constraints, and when each keeps its size, keeps its own size and its children's positions
//! without running its steps or its policy; otherwise they run. So a relayout is linear in the
//! dirty nodes, their ancestors and those ancestors' children, and a change that keeps a node's
//! size costs a flag check per sibling on its path. A policy is taken to depend only on its
//! constraints, its children's sizes and parent data, its node's properties and its captures
//! (D11): a child it did not measure has its last constraints cleared, so that its changes run
//! the policy again. Steps within a node run whenever the node is measured.

use crate::error::{
    raise, UiResult, INVALID_CONSTRAINTS, LAYOUT_OUTSIDE, LAYOUT_TWICE, MEASURED_OUTSIDE,
    MEASURED_TWICE, NOT_PLACED, NO_LAYOUT, NO_POLICY, PLACED_OUTSIDE, STALE_NODE,
};
use crate::geometry::{clamp_extent, Constraints, Rect, Size};
use crate::handle::NodeId;
use crate::modifier::{Element, ElementMirror, CAPACITY};
use crate::node::{Node, NodeKind};
use crate::properties::{Alignment, Arrangement};
use crate::runtime::{
    self, ConstraintsMirror, MeasurableMirror, MeasureResultMirror, MeasureScopeMirror,
};
use crate::text::{break_lines, line_width, Line, TextMeasurer};
use crate::thread::{with_state, State};
use prism_compose::Handle;
use prism_runtime::{FunctionValue, PrismList};

/// The layout pass as the app drives it: its counter of the last run.
#[derive(Debug, Default)]
pub struct Layout {
    /// How many nodes the last pass measured, cached ones not counted.
    pub measured: usize,
}

/// A position in a node's chain, as `Measurable`, `Placeable`, `MeasureScope`, `PlacementScope`
/// and `MeasureResult` name it (D5 of `layout-in-prism`): a child's measurable is its node at
/// element 0, a step's scope its element and the measurable it is given the next one, and the
/// content's policy measures from the chain's length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub node: NodeId,
    pub element: usize,
}

impl Position {
    /// A child, as its parent's policy measures and places it: its whole chain.
    pub fn child(node: NodeId) -> Position {
        Position { node, element: 0 }
    }

    /// The position of the words `node` and `element` an entry point was given; an element
    /// outside a chain names no position any frame measures.
    pub fn of_words(node: u64, element: u64) -> Position {
        Position {
            node: NodeId(node),
            element: usize::try_from(element).unwrap_or(usize::MAX),
        }
    }

    /// The position after this one in its node's chain.
    pub fn next(self) -> Position {
        Position {
            node: self.node,
            element: self.element.saturating_add(1),
        }
    }

    /// The words of a `Measurable`, a `MeasureScope` or a `PlacementScope` naming it.
    fn mirror(self) -> MeasurableMirror {
        MeasurableMirror {
            node: self.node.word(),
            element: self.element as u64,
        }
    }
}

/// One measurement frame: a step of `node`'s chain at `element`, or the policy of its content,
/// at the chain's length (D6 of `layout-primitive`, D5 of `layout-in-prism`).
#[derive(Debug, Clone, Copy)]
struct Frame {
    node: NodeId,
    /// The chain position the frame measures from.
    element: usize,
    /// Whether the frame is a step's, which measures and places the rest of its chain, rather
    /// than a policy's, which measures and places the node's children.
    step: bool,
    stamp: u64,
    /// A policy's content's origin in the node as last measured, where `place(0, 0)` puts a
    /// child; a step's offset of the rest of its chain, as it placed it.
    origin: (i64, i64),
    layout_called: bool,
    placing: bool,
    /// The size `layout` gave.
    size: Size,
    /// The node's children (one, the rest, for a step), how many the policy measured and
    /// placed, and whether every child was placed before it ran: when all are measured and
    /// placed and were placed before, the end of the run has no child to update.
    children: usize,
    measured: usize,
    placed: usize,
    all_placed_before: bool,
}

/// The state of the running layout pass, kept in the thread's state so that the entry points
/// a policy or a step calls reach it (D21). Its buffers are reused across passes.
#[derive(Debug, Default)]
pub struct LayoutState {
    frames: Vec<Frame>,
    /// The measurables lent to the policy running at each depth of nested policies.
    buffers: Vec<Vec<u64>>,
    /// How many policies are running, nested: the depth of the next one's buffer.
    policies: usize,
    lines: Vec<Line>,
    measured: usize,
    /// The last stamp given to a frame.
    stamps: u64,
    /// The text measurer of the running pass, valid while it runs.
    measurer: Option<*const (dyn TextMeasurer + 'static)>,
    /// The composition policies and steps run as handlers of, when an app runs.
    composition: Option<Handle>,
    /// Whether the pass changed a node's size, position, content or element bounds.
    changed: bool,
    /// The element measurer of the running app, `/prism/compose/ui`'s `measureElement`, which
    /// `runApp` gives the runtime (D2 of `layout-in-prism`): the app installs it and releases
    /// it when it ends.
    pub element_measurer: Option<FunctionValue>,
}

impl LayoutState {
    /// The running pass's measurer, not tied to the borrow of the state.
    fn measurer(&self) -> &'static dyn TextMeasurer {
        // SAFETY: set by `Layout::run` for the duration of the pass, from a measurer that
        // outlives it, and only used during the pass.
        unsafe {
            &*self
                .measurer
                .expect("a text measured outside a layout pass")
        }
    }

    /// Whether a layout pass is running on the thread.
    pub fn running(&self) -> bool {
        self.measurer.is_some()
    }
}

impl Layout {
    pub fn new() -> Layout {
        Layout::default()
    }

    /// Lays out the thread's tree when anything in it needs measuring, and gives whether it
    /// did, which makes the display list stale.
    pub fn run(&mut self, measurer: &dyn TextMeasurer) -> bool {
        self.run_marking(measurer, false)
    }

    /// [`Layout::run`] after a reload (D12): the display list is made stale only when the pass
    /// changed a node's size, position, content or element bounds.
    pub fn run_after_reload(&mut self, measurer: &dyn TextMeasurer) -> bool {
        self.run_marking(measurer, true)
    }

    fn run_marking(&mut self, measurer: &dyn TextMeasurer, only_changes: bool) -> bool {
        // SAFETY: the pointer is cleared before this function returns, and the measurer
        // outlives the call.
        let measurer: *const (dyn TextMeasurer + 'static) =
            unsafe { std::mem::transmute(measurer as *const dyn TextMeasurer) };
        let start = with_state(|state| {
            if !state.tree.needs_layout() {
                return None;
            }
            let root = state.tree.root();
            let window = state.tree.window_size();
            let constraints = Constraints::up_to(window.width, window.height);
            let root_node = state.tree.node(root).unwrap();
            let from_root =
                root_node.needs_measure || root_node.last_constraints != Some(constraints);
            let dirty = std::mem::take(&mut state.tree.dirty);
            let layout = &mut state.layout;
            layout.measurer = Some(measurer);
            layout.composition = state.app.and_then(|app| app.composition);
            layout.measured = 0;
            layout.changed = false;
            Some((root, constraints, from_root, dirty))
        });
        let Some((root, constraints, from_root, mut dirty)) = start else {
            self.measured = 0;
            return false;
        };
        if from_root {
            measure(root, constraints);
        } else {
            for &id in &dirty {
                relayout_from(id);
            }
            for &id in &dirty {
                clear_ancestors(id);
            }
        }
        with_state(|state| {
            let layout = &mut state.layout;
            self.measured = layout.measured;
            layout.measurer = None;
            layout.composition = None;
            if !only_changes || layout.changed {
                state.tree.redraw = true;
            }
            dirty.clear();
            state.tree.dirty = dirty;
        });
        true
    }
}

/// A policy call a measurement makes once the state is let go.
struct PolicyCall {
    function: FunctionValue,
    /// The chain position of the content.
    element: usize,
    list: PrismList,
    constraints: Constraints,
    composition: Option<Handle>,
}

/// A step call a measurement makes once the state is let go (D1 of `layout-in-prism`).
struct StepCall {
    /// The app's element measurer, called with the element, or a `Modifier.layout`'s step.
    function: FunctionValue,
    /// The built-in element, as Prism lays it out, or `None` for a `Modifier.layout`.
    element: Option<ElementMirror>,
    /// The step's position.
    scope: Position,
    constraints: Constraints,
    composition: Option<Handle>,
}

/// What measuring a node's chain from a position does once the state is let go.
enum Next {
    /// The content was measured natively, of this size.
    Done(Size),
    /// The window's children are measured next.
    Window,
    /// The node's policy is called next, its frame pushed.
    Policy(PolicyCall),
    /// A step is called next, its frame pushed.
    Step(StepCall),
}

/// The first step of measuring a node: what its cache says.
enum Start {
    Done(Size),
    /// The node is clean and measured with its last constraints, but has dirty children.
    CheckChildren,
    /// The node is measured in full.
    Full,
}

/// Measures `id` under `constraints`, records its layout and gives its size.
fn measure(id: NodeId, constraints: Constraints) -> Size {
    let start = with_state(|state| start(state, id, constraints));
    complete(id, constraints, start)
}

/// The first step of measuring `id`: from the cache, or in full.
fn start(state: &mut State, id: NodeId, constraints: Constraints) -> Start {
    let node = state.tree.node(id).unwrap();
    if !node.needs_measure && node.last_constraints == Some(constraints) {
        if node.child_needs_measure {
            return Start::CheckChildren;
        }
        return Start::Done(node.size);
    }
    Start::Full
}

/// Takes the steps of a measurement left after `start`, letting the state go between them.
fn complete(id: NodeId, constraints: Constraints, start: Start) -> Size {
    match start {
        Start::Done(size) => size,
        Start::CheckChildren => {
            if remeasure_dirty_children(id) {
                return with_state(|state| {
                    let node = state.tree.node_mut(id).unwrap();
                    node.child_needs_measure = false;
                    node.size
                });
            }
            measure_full(id, constraints)
        }
        Start::Full => measure_full(id, constraints),
    }
}

/// Measures `id` in full: its chain from the first element, then its layout recorded.
fn measure_full(id: NodeId, constraints: Constraints) -> Size {
    let size = measure_from(Position::child(id), constraints);
    with_state(|state| finish(state, id, constraints, size))
}

/// The value of `result`, or the runtime error it holds.
fn or_raise<T>(result: UiResult<T>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => raise(&error),
    }
}

/// Measures the chain of a node from `position` under `c`: the step there, which measures the
/// rest, or past the last step the content. Gives the size of the chain from `position`.
fn measure_from(position: Position, c: Constraints) -> Size {
    let id = position.node;
    match or_raise(with_state(|state| next_from(state, position, c))) {
        Next::Done(size) => size,
        Next::Window => measure_window(id),
        Next::Policy(call) => {
            call_policy(id, &call);
            or_raise(with_state(|state| end_policy(state, id, c)))
        }
        Next::Step(call) => {
            call_step(&call);
            or_raise(with_state(|state| end_step(state, call.scope, c)))
        }
    }
}

/// Finds the first step of `position`'s node at or after it, and prepares its call; past the
/// last step, measures a text's or text field's content, or prepares the window's children or
/// the node's policy call.
fn next_from(state: &mut State, position: Position, c: Constraints) -> UiResult<Next> {
    let id = position.node;
    if position.element == 0 {
        state.layout.measured += 1;
    }
    let node = state.tree.node(id).unwrap();
    let count = node.chain.len();
    let mut element = position.element;
    while element < count && !node.chain[element].element.is_step() {
        element += 1;
    }
    if element < count {
        let entry = node.chain[element];
        let (function, encoded) = match entry.element {
            Element::Layout { handler } => (
                state
                    .handlers
                    .function(handler)
                    .ok_or(crate::error::STALE_HANDLER)?,
                None,
            ),
            _ => (
                state
                    .layout
                    .element_measurer
                    .as_ref()
                    .map(runtime::borrowed)
                    .expect("a layout modifier measured without an app's element measurer"),
                Some(ElementMirror::encode(&entry.element)),
            ),
        };
        push_frame(state, id, element, true, (0, 0), 1, true);
        return Ok(Next::Step(StepCall {
            function,
            element: encoded,
            scope: Position { node: id, element },
            constraints: c,
            composition: state.layout.composition,
        }));
    }
    match node.kind {
        NodeKind::Text | NodeKind::TextField => {
            let content = native_content(state, id, c);
            Ok(Next::Done(content_measured(state, id, content)))
        }
        NodeKind::Window => Ok(Next::Window),
        _ => Ok(Next::Policy(prepare_policy(state, id, c)?)),
    }
}

/// Records the size of `id`'s content, at most `MAX_EXTENT` on each axis (D23 of
/// `layout-primitive`), and gives it.
fn content_measured(state: &mut State, id: NodeId, content: Size) -> Size {
    let content = content.bounded();
    let node = state.tree.node_mut(id).unwrap();
    if node.content.width != content.width || node.content.height != content.height {
        node.content.width = content.width;
        node.content.height = content.height;
        state.layout.changed = true;
    }
    content
}

/// The window's content: its children under its size, at its content's top-left corner.
fn measure_window(id: NodeId) -> Size {
    let (window, first, (x, y)) = with_state(|state| {
        let node = state.tree.node(id).unwrap();
        (
            node.size,
            node.first_child,
            (node.content.x, node.content.y),
        )
    });
    let loose = Constraints::up_to(window.width, window.height);
    let mut child = first;
    while let Some(current) = child {
        measure(current, loose);
        child = with_state(|state| {
            set_position(state, current, x, y);
            let node = state.tree.node_mut(current).unwrap();
            state.layout.changed |= !node.placed;
            node.placed = true;
            node.next
        });
    }
    with_state(|state| content_measured(state, id, window))
}

/// Pushes a measurement frame of `node` from `element`.
fn push_frame(
    state: &mut State,
    node: NodeId,
    element: usize,
    step: bool,
    origin: (i64, i64),
    children: usize,
    all_placed_before: bool,
) {
    let layout = &mut state.layout;
    layout.stamps += 1;
    layout.frames.push(Frame {
        node,
        element,
        step,
        stamp: layout.stamps,
        origin,
        layout_called: false,
        placing: false,
        size: Size::ZERO,
        children,
        measured: 0,
        placed: 0,
        all_placed_before,
    });
}

/// Calls a step with no state borrowed, as a handler of the app's composition when an app runs.
fn call_step(call: &StepCall) {
    let scope = call.scope.mirror();
    let scope = MeasureScopeMirror {
        node: scope.node,
        element: scope.element,
    };
    let measurable = call.scope.next().mirror();
    let constraints = ConstraintsMirror::from(call.constraints);
    let run = || {
        // SAFETY: the element measurer is the app's, of its function type, live while the app
        // runs, and the element is encoded as Prism lays it out; a `Modifier.layout`'s step is
        // an entry its node holds, of its function type, live during layout, since no pass
        // runs then.
        unsafe {
            match &call.element {
                Some(element) => runtime::call_element_measurer(
                    &call.function,
                    &scope,
                    element,
                    &measurable,
                    &constraints,
                ),
                None => {
                    runtime::call_layout_modifier(&call.function, &scope, &measurable, &constraints)
                }
            }
        }
    };
    let _result: MeasureResultMirror = match call.composition {
        Some(composition) => runtime::run_handler(composition, run),
        None => run(),
    };
}

/// After a step returned: pops its frame, checks that it placed the rest of its chain (D10 of
/// `layout-in-prism`), records its size and offset in its chain entry, and gives the size,
/// clamped into its constraints and at most `MAX_EXTENT`.
fn end_step(state: &mut State, scope: Position, c: Constraints) -> UiResult<Size> {
    let frame = state.layout.frames.pop().expect("the step's frame");
    debug_assert!(frame.step && frame.node == scope.node && frame.element == scope.element);
    if frame.placed == 0 {
        return Err(NOT_PLACED);
    }
    let size = c.constrain(frame.size).bounded();
    let entry = &mut state.tree.node_mut(scope.node).unwrap().chain[scope.element];
    entry.step_size = size;
    entry.step_offset = frame.origin;
    Ok(size)
}

/// The last step of a measurement: sums the steps' offsets into each position's origin,
/// records the bounds of the drawing and input elements and the content's origin, moves the
/// children placed in this measurement when the content moved, and records the node's size,
/// `size` being the size of its chain.
fn finish(state: &mut State, id: NodeId, constraints: Constraints, size: Size) -> Size {
    let node = state.tree.node_mut(id).unwrap();
    let count = node.chain.len().min(CAPACITY);
    let mut origins = [(0i64, 0i64); CAPACITY];
    let (mut x, mut y) = (0i64, 0i64);
    for (index, entry) in node.chain[..count].iter().enumerate() {
        origins[index] = (x, y);
        if entry.element.is_step() {
            x += entry.step_offset.0;
            y += entry.step_offset.1;
        }
    }
    let mut changed = false;
    let mut rest = Size::new(node.content.width, node.content.height);
    for (index, entry) in node.chain[..count].iter_mut().enumerate().rev() {
        if entry.element.is_step() {
            rest = entry.step_size;
        } else if entry.element.records_bounds() {
            let (x, y) = origins[index];
            let bounds = Rect::new(x, y, rest.width, rest.height);
            changed |= entry.bounds != bounds;
            entry.bounds = bounds;
        }
    }
    let size = if node.kind == NodeKind::Window {
        node.size
    } else {
        size
    };
    let moved = (x - node.content.x, y - node.content.y);
    node.content.x = x;
    node.content.y = y;
    changed |= moved != (0, 0) || node.size != size;
    node.size = size;
    node.last_constraints = Some(constraints);
    node.needs_measure = false;
    node.child_needs_measure = false;
    let first = node.first_child;
    if moved != (0, 0) {
        // The children were placed at the content's last origin (see "Positions").
        let mut child = first;
        while let Some(current) = child {
            let node = state.tree.node_mut(current).unwrap();
            if node.placed {
                node.x += moved.0;
                node.y += moved.1;
            }
            child = node.next;
        }
    }
    state.layout.changed |= changed;
    size
}

/// A relayout from a node marked for measuring: re-measures it with its last constraints,
/// which stay its own while its siblings and ancestors are unchanged, and, when its size
/// changed, its parent with its policy, and so on up, stopping at the first node whose size
/// stays. A node without last constraints is measured by its parent's policy.
fn relayout_from(id: NodeId) {
    let mut current = id;
    loop {
        let step = with_state(|state| {
            let node = state.tree.node(current)?;
            if !node.needs_measure && !node.child_needs_measure {
                return None;
            }
            Some((node.parent, node.last_constraints, node.size))
        });
        let Some((parent, last, old)) = step else {
            return;
        };
        let measured = match last {
            Some(last) => measure(current, last) != old,
            None => true,
        };
        let Some(parent) = parent.filter(|_| measured) else {
            return;
        };
        with_state(|state| state.tree.node_mut(parent).unwrap().needs_measure = true);
        current = parent;
    }
}

/// Clears `child_needs_measure` on the ancestors of `id` once every dirty node has been
/// measured.
fn clear_ancestors(id: NodeId) {
    with_state(|state| {
        let tree = &mut state.tree;
        let mut parent = tree.node(id).and_then(|node| node.parent);
        while let Some(ancestor) = parent {
            let node = tree.node_mut(ancestor).unwrap();
            node.child_needs_measure = false;
            parent = node.parent;
        }
    })
}

/// For a node that is clean itself and measured with its last constraints: re-measures its
/// dirty children with their last constraints, which are still theirs while every sibling
/// keeps its size, and gives whether every one kept its size, in which case the node's size
/// and its children's positions stand. Otherwise the caller measures the node again with its
/// steps and policy.
fn remeasure_dirty_children(id: NodeId) -> bool {
    let mut child = with_state(|state| state.tree.node(id).unwrap().first_child);
    while let Some(current) = child {
        let (next, dirty, last, size) = with_state(|state| {
            let node = state.tree.node(current).unwrap();
            (
                node.next,
                node.needs_measure || node.child_needs_measure,
                node.last_constraints,
                node.size,
            )
        });
        child = next;
        if !dirty {
            continue;
        }
        let Some(last) = last else {
            return false;
        };
        if measure(current, last) != size {
            return false;
        }
    }
    true
}

/// Sets a child's position relative to its parent, noting a change.
fn set_position(state: &mut State, child: NodeId, x: i64, y: i64) {
    let node = state.tree.node_mut(child).unwrap();
    if node.x != x || node.y != y {
        node.x = x;
        node.y = y;
        state.layout.changed = true;
    }
}

/// The content of a text or a text field, measured natively. A text or text field lays out no
/// children: any a raw node group gave it are not shown (D7 of `layout-primitive`).
fn native_content(state: &mut State, id: NodeId, c: Constraints) -> Size {
    let State { tree, layout, .. } = state;
    let measurer = layout.measurer();
    let node = tree.node(id).unwrap();
    let font_size = node.style.resolved_font_size();
    let mut child = node.first_child;
    let size = if node.kind == NodeKind::Text {
        break_lines(
            node.text.as_str(),
            c.max_width,
            font_size,
            measurer,
            &mut layout.lines,
        );
        let width = layout
            .lines
            .iter()
            .map(|line| line.width)
            .max()
            .unwrap_or(0);
        let height = layout.lines.len() as i64 * measurer.line_height(font_size);
        let node = tree.node_mut(id).unwrap();
        layout.changed |= node.break_width != c.max_width;
        node.break_width = c.max_width;
        c.constrain(Size::new(width, height))
    } else {
        let width = line_width(node.text.as_str(), font_size, measurer);
        c.constrain(Size::new(width, measurer.line_height(font_size)))
    };
    while let Some(current) = child {
        let node = tree.node_mut(current).unwrap();
        layout.changed |= node.placed;
        node.placed = false;
        child = node.next;
    }
    size
}

/// Prepares the call of a node's policy (D6): pushes its frame, at the chain's length and with
/// the content's last origin, and lends its children as measurables, in the buffer of the
/// frame's depth.
fn prepare_policy(state: &mut State, id: NodeId, c: Constraints) -> UiResult<PolicyCall> {
    let State {
        tree,
        handlers,
        layout,
        ..
    } = state;
    let node = tree.node(id).unwrap();
    let function = node
        .policy
        .and_then(|policy| handlers.function(policy))
        .ok_or(NO_POLICY)?;
    let element = node.chain.len();
    let origin = (node.content.x, node.content.y);
    // One buffer per depth of nested policies, steps not counted.
    let depth = layout.policies;
    layout.policies += 1;
    if layout.buffers.len() == depth {
        layout.buffers.push(Vec::new());
    }
    let buffer = &mut layout.buffers[depth];
    buffer.clear();
    let mut child = node.first_child;
    let mut all_placed_before = true;
    while let Some(current) = child {
        // A `Measurable`: the child's word and element 0.
        buffer.push(current.word());
        buffer.push(0);
        let node = tree.node(current).unwrap();
        all_placed_before &= node.placed;
        child = node.next;
    }
    let children = buffer.len() / 2;
    // The list header over the buffer: never null, as an empty list's pointer is its element
    // alignment.
    let list = PrismList {
        pointer: if buffer.is_empty() {
            std::ptr::without_provenance_mut(std::mem::align_of::<u64>())
        } else {
            buffer.as_mut_ptr().cast()
        },
        size: children,
        capacity: children,
    };
    let composition = layout.composition;
    push_frame(
        state,
        id,
        element,
        false,
        origin,
        children,
        all_placed_before,
    );
    Ok(PolicyCall {
        function,
        element,
        list,
        constraints: c,
        composition,
    })
}

/// Calls a node's policy with no state borrowed, as a handler of the app's composition when an
/// app runs (D14).
fn call_policy(id: NodeId, call: &PolicyCall) {
    let scope = MeasureScopeMirror {
        node: id.word(),
        element: call.element as u64,
    };
    let constraints = ConstraintsMirror::from(call.constraints);
    let run = || {
        // SAFETY: the node claimed the entry as a measure policy, of the policy's function
        // type; the entry stays live during the call, since no pass runs during layout, and the
        // list's buffer is not touched until the call returns (D4).
        unsafe { runtime::call_measure_policy(&call.function, &scope, &call.list, &constraints) }
    };
    let _result: MeasureResultMirror = match call.composition {
        Some(composition) => runtime::run_handler(composition, run),
        None => run(),
    };
}

/// After a node's policy returned: pops its frame, checks that it called `layout`, forgets the
/// last constraints of the children it did not measure, notes which it placed, and gives the
/// size `layout` gave, clamped into the content's constraints, as the content's size.
fn end_policy(state: &mut State, id: NodeId, c: Constraints) -> UiResult<Size> {
    let State { tree, layout, .. } = state;
    let frame = layout.frames.pop().expect("the policy's frame");
    layout.policies -= 1;
    debug_assert!(!frame.step && frame.node == id);
    if !frame.layout_called {
        return Err(NO_LAYOUT);
    }
    let size = c.constrain(frame.size);
    if !(frame.all_placed_before
        && frame.measured == frame.children
        && frame.placed == frame.children)
    {
        let mut child = tree.node(id).unwrap().first_child;
        while let Some(current) = child {
            let node = tree.node_mut(current).unwrap();
            if node.stamp != frame.stamp {
                // Not measured: its changes run this policy again (D11).
                node.last_constraints = None;
                node.needs_measure = false;
                node.child_needs_measure = false;
            }
            // Placed by this run or not (D7).
            let placed = node.placed_stamp == frame.stamp;
            layout.changed |= placed != node.placed;
            node.placed = placed;
            child = node.next;
        }
    }
    Ok(content_measured(state, id, size))
}

/// Whether constraints have minimums of at least 0 and at most their maximums.
fn valid(c: Constraints) -> bool {
    c.min_width >= 0
        && c.min_height >= 0
        && c.min_width <= c.max_width
        && c.min_height <= c.max_height
}

/// `Measurable.measure(constraints)` of a child (D6): measures `child`, a child of the node
/// whose policy is running innermost, at most once in that run, and gives its size.
pub fn measure_child(child: NodeId, constraints: Constraints) -> UiResult<Size> {
    measure_position(Position::child(child), constraints)
}

/// What [`measure_position`] measures once the state is let go.
enum Measured {
    /// A child of the running policy's node, from the start of its measurement.
    Child(Start),
    /// The rest of the running step's chain.
    Rest,
}

/// `Measurable.measure(constraints)` of the measurable naming `position` (D5 of
/// `layout-in-prism`): a child of the node whose policy is running innermost, at element 0, or
/// the rest of the chain of the step running innermost, after its element; at most once in
/// that run. Gives its size.
pub fn measure_position(position: Position, constraints: Constraints) -> UiResult<Size> {
    let measured = with_state(|state| {
        let frame = state.layout.frames.last_mut().ok_or(MEASURED_OUTSIDE)?;
        if frame.step {
            if position
                != (Position {
                    node: frame.node,
                    element: frame.element,
                })
                .next()
            {
                return Err(MEASURED_OUTSIDE);
            }
            if frame.measured > 0 {
                return Err(MEASURED_TWICE);
            }
            if !valid(constraints) {
                return Err(INVALID_CONSTRAINTS);
            }
            frame.measured = 1;
            return Ok(Measured::Rest);
        }
        let frame = *frame;
        let child = position.node;
        let node = state.tree.node_mut(child).ok_or(MEASURED_OUTSIDE)?;
        if position.element != 0 || node.parent != Some(frame.node) {
            return Err(MEASURED_OUTSIDE);
        }
        if node.stamp == frame.stamp {
            return Err(MEASURED_TWICE);
        }
        if !valid(constraints) {
            return Err(INVALID_CONSTRAINTS);
        }
        node.stamp = frame.stamp;
        state.layout.frames.last_mut().unwrap().measured += 1;
        Ok(Measured::Child(start(state, child, constraints)))
    })?;
    Ok(match measured {
        Measured::Child(start) => complete(position.node, constraints, start),
        Measured::Rest => measure_from(position, constraints),
    })
}

/// The size of `child`, measured in the innermost running policy: the placeable a container
/// gets again in its placement block (D9).
pub fn measured(child: NodeId) -> UiResult<Size> {
    measured_child(child).map(|(size, _)| size)
}

/// The start of `layout(width, height) { ... }` in the measurement `scope` names (D6).
pub fn begin_placement(scope: Position, width: i64, height: i64) -> UiResult<()> {
    with_state(|state| {
        let frame = state.layout.frames.last_mut().ok_or(LAYOUT_OUTSIDE)?;
        if frame.node != scope.node || frame.element != scope.element {
            return Err(LAYOUT_OUTSIDE);
        }
        if frame.layout_called {
            return Err(LAYOUT_TWICE);
        }
        frame.layout_called = true;
        frame.placing = true;
        frame.size = Size::new(width, height);
        Ok(())
    })
}

/// The end of `layout`'s placement block.
pub fn end_placement(scope: Position) -> UiResult<()> {
    with_state(|state| {
        let frame = state.layout.frames.last_mut().ok_or(LAYOUT_OUTSIDE)?;
        if frame.node != scope.node || frame.element != scope.element {
            return Err(LAYOUT_OUTSIDE);
        }
        frame.placing = false;
        Ok(())
    })
}

/// `Placeable.place(x, y)` in the placement block of `scope`: places `placeable`, measured in
/// the running frame of `scope`, at `(x, y)` from the content's top-left corner for a child of
/// a policy, or from the step's for the rest of a step's chain, while that frame is placing.
pub fn place(scope: Position, placeable: Position, x: i64, y: i64) -> UiResult<()> {
    with_state(|state| {
        let State { tree, layout, .. } = state;
        let frame = layout.frames.last_mut().ok_or(PLACED_OUTSIDE)?;
        if frame.node != scope.node || frame.element != scope.element || !frame.placing {
            return Err(PLACED_OUTSIDE);
        }
        // Positions stay within `MAX_EXTENT` of the content or the step (D23).
        let (x, y) = (clamp_extent(x), clamp_extent(y));
        if frame.step {
            if placeable != scope.next() || frame.measured == 0 {
                return Err(PLACED_OUTSIDE);
            }
            frame.origin = (x, y);
            frame.placed = 1;
            return Ok(());
        }
        let node = tree.node_mut(placeable.node).ok_or(PLACED_OUTSIDE)?;
        if placeable.element != 0 || node.parent != Some(scope.node) || node.stamp != frame.stamp {
            return Err(PLACED_OUTSIDE);
        }
        let (x, y) = (frame.origin.0 + x, frame.origin.1 + y);
        if node.x != x || node.y != y {
            node.x = x;
            node.y = y;
            layout.changed = true;
        }
        if node.placed_stamp != frame.stamp {
            node.placed_stamp = frame.stamp;
            frame.placed += 1;
        }
        Ok(())
    })
}

/// The size and own alignment of `child`, measured in the innermost running policy: what a
/// container's placement block reads in one call (D25).
pub fn measured_child(child: NodeId) -> UiResult<(Size, Option<Alignment>)> {
    with_state(|state| {
        let frame = *state.layout.frames.last().ok_or(PLACED_OUTSIDE)?;
        let node = state.tree.node(child).ok_or(PLACED_OUTSIDE)?;
        if frame.step || node.parent != Some(frame.node) || node.stamp != frame.stamp {
            return Err(PLACED_OUTSIDE);
        }
        Ok((node.size, node.parent_data.2))
    })
}

/// A live node's value, for the property reads of the containers' policies.
fn read<R>(node: NodeId, read: impl FnOnce(&Node) -> R) -> UiResult<R> {
    with_state(|state| state.tree.node(node).map(read).ok_or(STALE_NODE))
}

/// A row's or column's arrangement, as its policy reads it.
pub fn arrangement(node: NodeId) -> UiResult<Arrangement> {
    read(node, |node| node.arrangement)
}

/// A row's or column's arrangement and alignment, as its policy reads them in one call (D25).
pub fn line_properties(node: NodeId) -> UiResult<(Arrangement, Alignment)> {
    read(node, |node| (node.arrangement, node.alignment))
}

/// A box's, row's or column's alignment, as its policy reads it.
pub fn alignment(node: NodeId) -> UiResult<Alignment> {
    read(node, |node| node.alignment)
}

/// A child's parent data (D9): its first `weight` (0.0 for none) with its `fill`, and its
/// first `align` (`None` for none).
pub fn parent_data(node: NodeId) -> UiResult<(f64, bool, Option<Alignment>)> {
    read(node, |node| node.parent_data)
}

/// The absolute bounds of `id`, from the positions relative to each parent.
pub fn absolute_bounds(tree: &crate::tree::UiTree, id: NodeId) -> Rect {
    let node = tree.node(id).unwrap();
    let (mut x, mut y) = (node.x, node.y);
    let mut parent = node.parent;
    while let Some(ancestor) = parent {
        let node = tree.node(ancestor).unwrap();
        x += node.x;
        y += node.y;
        parent = node.parent;
    }
    Rect::new(x, y, node.size.width, node.size.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifier::Element;
    use crate::properties::{Arrangement, Shape};
    use crate::test_support::*;
    use crate::text::FixedAdvanceMetrics;
    use std::cell::Cell;

    fn lay_out(builder: &mut TreeBuilder) {
        builder.lay_out();
    }

    /// `x,y wxh` of a node, as dumps print it.
    fn bounds(builder: &TreeBuilder, id: NodeId) -> String {
        let r = absolute_bounds(&builder.tree, id);
        format!("{},{} {}x{}", r.x, r.y, r.width, r.height)
    }

    #[test]
    fn column_stacks_children() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        b.arrangement(column, Arrangement::SpacedBy(8));
        let first = b.text(column, "a");
        let second = b.text(column, "b");
        lay_out(&mut b);
        assert_eq!(bounds(&b, column), "0,0 8x48");
        assert_eq!(bounds(&b, first), "0,0 8x20");
        assert_eq!(bounds(&b, second), "0,28 8x20");
    }

    #[test]
    fn space_between_two_texts() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let row = b.add(root, NodeKind::Row);
        b.chain(row, &[width(100)])
            .arrangement(row, Arrangement::SpaceBetween);
        let a = b.text(row, "a");
        let c = b.text(row, "b");
        lay_out(&mut b);
        assert_eq!(bounds(&b, a), "0,0 8x20");
        assert_eq!(bounds(&b, c), "92,0 8x20");
    }

    #[test]
    fn every_arrangement_on_a_main_axis() {
        let positions = |arrangement| {
            let mut b = TreeBuilder::new(400, 300);
            let root = b.root();
            let row = b.add(root, NodeKind::Row);
            b.chain(row, &[width(100)]).arrangement(row, arrangement);
            let texts = [b.text(row, "a"), b.text(row, "b"), b.text(row, "c")];
            lay_out(&mut b);
            texts.map(|text| absolute_bounds(&b.tree, text).x)
        };
        // Three texts of 8 leave 76 free.
        assert_eq!(positions(Arrangement::Start), [0, 8, 16]);
        assert_eq!(positions(Arrangement::Top), [0, 8, 16]);
        assert_eq!(positions(Arrangement::End), [76, 84, 92]);
        assert_eq!(positions(Arrangement::Bottom), [76, 84, 92]);
        assert_eq!(positions(Arrangement::Center), [38, 46, 54]);
        assert_eq!(positions(Arrangement::SpaceBetween), [0, 46, 92]);
        assert_eq!(positions(Arrangement::SpaceAround), [12, 45, 78]);
        assert_eq!(positions(Arrangement::SpaceEvenly), [19, 46, 73]);
        assert_eq!(positions(Arrangement::SpacedBy(4)), [0, 12, 24]);
    }

    #[test]
    fn centred_content_in_a_box() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let boxed = b.add(root, NodeKind::Box);
        b.chain(boxed, &[fill_max(true, true, 1.0)])
            .alignment(boxed, Alignment::CENTER);
        let text = b.text(boxed, "ab");
        lay_out(&mut b);
        assert_eq!(bounds(&b, boxed), "0,0 400x300");
        assert_eq!(bounds(&b, text), "192,140 16x20");
    }

    #[test]
    fn a_weighted_text_fills_a_row() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let row = b.add(root, NodeKind::Row);
        b.chain(row, &[width(200)]);
        let a = b.text(row, "a");
        b.chain(a, &[weight(1.0, true)]);
        let bb = b.text(row, "bb");
        lay_out(&mut b);
        assert_eq!(bounds(&b, row), "0,0 200x20");
        assert_eq!(bounds(&b, a), "0,0 184x20");
        assert_eq!(bounds(&b, bb), "184,0 16x20");
    }

    #[test]
    fn weights_share_the_rest_and_the_last_takes_the_remainder() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let row = b.add(root, NodeKind::Row);
        b.chain(row, &[width(100)]);
        let one = b.add(row, NodeKind::Box);
        b.chain(one, &[weight(1.0, true)]);
        let two = b.add(row, NodeKind::Box);
        b.chain(two, &[weight(2.0, true)]);
        let loose = b.text(row, "a");
        b.chain(loose, &[weight(1.0, false)]);
        lay_out(&mut b);
        // 100 / 4 = 25, 100 * 2 / 4 = 50, the last weighted child takes the remaining 25 at
        // most, without fill.
        assert_eq!(bounds(&b, one), "0,0 25x0");
        assert_eq!(bounds(&b, two), "25,0 50x0");
        assert_eq!(bounds(&b, loose), "75,0 8x20");
    }

    #[test]
    fn aligning_one_child() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let outer = b.add(root, NodeKind::Column);
        let inner = b.add(outer, NodeKind::Column);
        b.chain(inner, &[width(100)]);
        let a = b.text(inner, "a");
        let text_b = b.text(inner, "b");
        b.chain(text_b, &[align(Alignment::END)]);
        let boxed = b.add(outer, NodeKind::Box);
        b.chain(boxed, &[size(100, 100)]);
        let c = b.text(boxed, "c");
        b.chain(c, &[align(Alignment::BOTTOM_END)]);
        lay_out(&mut b);
        assert_eq!(bounds(&b, a), "0,0 8x20");
        assert_eq!(bounds(&b, text_b), "92,20 8x20");
        assert_eq!(bounds(&b, c), "92,120 8x20");
    }

    #[test]
    fn padding_around_a_column() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        b.chain(column, &[padding(16)]);
        let text = b.text(column, "Hi");
        lay_out(&mut b);
        assert_eq!(bounds(&b, column), "0,0 48x52");
        assert_eq!(bounds(&b, text), "16,16 16x20");
    }

    #[test]
    fn chained_and_horizontal_paddings() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        let chained = b.add(column, NodeKind::Box);
        b.chain(chained, &[padding(8), padding(4)]);
        let a = b.text(chained, "a");
        let horizontal = b.add(column, NodeKind::Box);
        b.chain(horizontal, &[padding_each(8, 0, 8, 0)]);
        let c = b.text(horizontal, "a");
        lay_out(&mut b);
        assert_eq!(bounds(&b, chained), "0,0 32x44");
        assert_eq!(bounds(&b, a), "12,12 8x20");
        assert_eq!(bounds(&b, horizontal), "0,44 24x20");
        assert_eq!(bounds(&b, c), "8,44 8x20");
    }

    #[test]
    fn a_size_larger_than_the_window_is_clamped() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let boxed = b.add(root, NodeKind::Box);
        b.chain(boxed, &[size(500, 500)]);
        lay_out(&mut b);
        assert_eq!(bounds(&b, boxed), "0,0 400x300");
    }

    #[test]
    fn padding_before_or_after_a_size() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        let before = b.add(column, NodeKind::Box);
        b.chain(before, &[padding(8), size(40, 40)]);
        let a = b.text(before, "a");
        let after = b.add(column, NodeKind::Box);
        b.chain(after, &[size(40, 40), padding(8)]);
        let text_b = b.text(after, "b");
        lay_out(&mut b);
        assert_eq!(bounds(&b, before), "0,0 56x56");
        assert_eq!(bounds(&b, a), "8,8 8x20");
        assert_eq!(bounds(&b, after), "0,56 40x40");
        assert_eq!(bounds(&b, text_b), "8,64 8x20");
    }

    #[test]
    fn minimum_and_fractional_sizes() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        let minimum = b.add(column, NodeKind::Box);
        b.chain(minimum, &[default_min_size(30, -1)]);
        b.text(minimum, "a");
        let half = b.add(column, NodeKind::Box);
        b.chain(half, &[fill_max(true, false, 0.5)]);
        let tall = b.add(column, NodeKind::Box);
        b.chain(tall, &[fill_max(false, true, 0.25)]);
        lay_out(&mut b);
        assert_eq!(bounds(&b, minimum), "0,0 30x20");
        assert_eq!(bounds(&b, half), "0,20 200x0");
        // A quarter of what remains of the column's height: (300 - 20) / 4.
        assert_eq!(bounds(&b, tall), "0,20 0x70");
    }

    #[test]
    fn a_spacer_takes_its_size_from_its_modifier() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        b.text(column, "a");
        let spacer = b.add(column, NodeKind::Spacer);
        b.chain(spacer, &[height(8)]);
        let text_b = b.text(column, "b");
        lay_out(&mut b);
        assert_eq!(bounds(&b, spacer), "0,20 0x8");
        assert_eq!(bounds(&b, text_b), "0,28 8x20");
    }

    #[test]
    fn wrapped_text_and_a_larger_font() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        let wrapped = b.text(column, "aa bb cc");
        b.chain(wrapped, &[width(40)]);
        let large = b.text(column, "abc");
        b.style(large, 0x1_0000_0000, 24);
        lay_out(&mut b);
        assert_eq!(bounds(&b, wrapped), "0,0 40x40");
        assert_eq!(bounds(&b, large), "0,40 36x30");
    }

    #[test]
    fn a_text_field_is_one_line_of_its_value() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        let field = b.text_field(column, "ac");
        b.chain(
            field,
            &[
                default_min_size(200, -1),
                Element::TextFieldFrame {
                    unfocused: crate::properties::Color(0xFF79_747E),
                    focused: crate::properties::Color(0xFF67_50A4),
                },
                padding(8),
            ],
        );
        let empty = b.text_field(column, "");
        b.text(column, "Hello ac");
        lay_out(&mut b);
        assert_eq!(bounds(&b, field), "0,0 200x36");
        let node = b.tree.node(field).unwrap();
        assert_eq!(node.chain[1].bounds, Rect::new(0, 0, 200, 36));
        assert_eq!(node.content, Rect::new(8, 8, 184, 20));
        assert_eq!(bounds(&b, empty), "0,36 0x20");
        assert_eq!(bounds(&b, column), "0,0 200x76");
    }

    #[test]
    fn a_buttons_size_and_label_position() {
        // `Button(onClick = { }) { Text("+") }`, as material3 builds it (D24).
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let button = b.add(root, NodeKind::Box);
        let id = b.register(crate::handlers::HandlerKind::Click, inline_handler(0));
        b.chain(
            button,
            &[
                background(0xFF67_50A4, Shape::new(0, 50)),
                clickable(id, true, Some(crate::properties::Role::Button)),
            ],
        );
        let row = b.add(button, NodeKind::Row);
        b.chain(row, &[default_min_size(58, 40), padding_each(24, 8, 24, 8)])
            .arrangement(row, Arrangement::Center)
            .alignment(row, Alignment::CENTER_VERTICALLY);
        let label = b.text(row, "+");
        lay_out(&mut b);
        assert_eq!(bounds(&b, button), "0,0 58x40");
        assert_eq!(bounds(&b, row), "0,0 58x40");
        assert_eq!(bounds(&b, label), "25,10 8x20");
        assert_eq!(
            b.tree.node(button).unwrap().chain[0].bounds,
            Rect::new(0, 0, 58, 40)
        );
    }

    #[test]
    fn elements_record_the_bounds_at_their_position() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let boxed = b.add(root, NodeKind::Box);
        let id = b.register(crate::handlers::HandlerKind::Click, inline_handler(0));
        b.chain(
            boxed,
            &[
                background(0xFFFF_0000, Shape::RECTANGLE),
                padding(20),
                clickable(id, true, None),
                size(10, 10),
            ],
        );
        lay_out(&mut b);
        let node = b.tree.node(boxed).unwrap();
        assert_eq!(node.chain[0].bounds, Rect::new(0, 0, 50, 50));
        assert_eq!(node.chain[2].bounds, Rect::new(20, 20, 10, 10));
        assert_eq!(node.size, Size::new(50, 50));
    }

    /// Counts the characters it measures.
    struct CountingMeasurer(Cell<usize>);

    impl TextMeasurer for CountingMeasurer {
        fn advance(&self, character: char, font_size: i64) -> i64 {
            self.0.set(self.0.get() + 1);
            FixedAdvanceMetrics.advance(character, font_size)
        }

        fn line_height(&self, font_size: i64) -> i64 {
            FixedAdvanceMetrics.line_height(font_size)
        }
    }

    /// The scenario "One changed text is measured alone".
    #[test]
    fn one_changed_text_is_measured_alone() {
        let mut b = TreeBuilder::new(400, 100_000);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        let texts: Vec<NodeId> = (0..100)
            .map(|index| b.text(column, &format!("text {:03}", index)))
            .collect();
        let measurer = CountingMeasurer(Cell::new(0));
        let mut layout = Layout::new();
        assert!(b.lay_out_with(&mut layout, &measurer));
        assert_eq!(measurer.0.get(), 100 * 8);
        assert_eq!(layout.measured, 102);
        measurer.0.set(0);
        assert!(
            !b.lay_out_with(&mut layout, &measurer),
            "a clean tree is not laid out"
        );
        b.tree.set_text(texts[50], "changed!");
        assert!(b.lay_out_with(&mut layout, &measurer));
        assert_eq!(measurer.0.get(), 8, "only the changed text's characters");
        assert_eq!(
            layout.measured, 1,
            "the column and the root keep their sizes"
        );
        // A text that changes its size re-measures its column, whose other children are
        // cached.
        measurer.0.set(0);
        b.tree.set_text(texts[20], "a longer text");
        assert!(b.lay_out_with(&mut layout, &measurer));
        assert_eq!(measurer.0.get(), 13);
        assert_eq!(layout.measured, 3, "the text, its column and the root");
        assert_eq!(
            crate::dump::bounds_text(&b.tree, column),
            "0,0 104x2000",
            "the column is as wide as the new text"
        );
        assert_eq!(crate::dump::bounds_text(&b.tree, texts[21]), "0,420 64x20");
    }

    /// Incremental relayouts give the layout a full one gives: random edits of texts,
    /// chains and structure, each followed by a relayout whose dump is compared with the dump
    /// after every node is marked and laid out again.
    #[test]
    fn incremental_relayouts_match_full_layouts() {
        for seed in 0..20 {
            incremental_relayouts_match_full_layouts_from(seed);
        }
    }

    fn incremental_relayouts_match_full_layouts_from(mut seed: u64) {
        let initial = seed;
        let mut random = move |bound: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % bound
        };
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        let mut nodes = vec![column];
        for _ in 0..30 {
            let parent = nodes[random(nodes.len() as u64) as usize];
            let kind = [
                NodeKind::Row,
                NodeKind::Column,
                NodeKind::Box,
                NodeKind::Text,
            ][random(4) as usize];
            let parent = if b.tree.node(parent).unwrap().kind == NodeKind::Text {
                column
            } else {
                parent
            };
            let node = b.add(parent, kind);
            if kind == NodeKind::Text {
                b.tree
                    .set_text(node, &"word ".repeat(random(4) as usize + 1));
            }
            nodes.push(node);
        }
        let dump = |tree: &crate::tree::UiTree| {
            let mut out = String::new();
            crate::dump::dump_tree(tree, None, &mut out);
            out
        };
        let mut layout = Layout::new();
        b.lay_out_with(&mut layout, &FixedAdvanceMetrics);
        for step in 0..300 {
            let node = nodes[1 + random(nodes.len() as u64 - 1) as usize];
            if b.tree.node(node).is_none() {
                continue;
            }
            match random(7) {
                5 => {
                    // Remove and release a subtree.
                    let parent = b.tree.node(node).unwrap().parent.unwrap();
                    let mut drops = Vec::new();
                    b.tree.remove(parent, node);
                    b.tree.release(node, &mut b.handlers, &mut drops);
                    crate::handlers::release_functions(&mut drops);
                }
                6 => {
                    // Insert a text after the node.
                    let parent = b.tree.node(node).unwrap().parent.unwrap();
                    let text = b.tree.create_kind(NodeKind::Text);
                    b.tree.set_text(text, "new");
                    b.tree.place_after(parent, Some(node), text);
                    nodes.push(text);
                }
                0 => {
                    if b.tree.node(node).unwrap().kind == NodeKind::Text {
                        let text = "ab ".repeat(random(6) as usize);
                        b.tree.set_text(node, &text);
                    }
                }
                1 => {
                    let chains: [&[Element]; 6] = [
                        &[],
                        &[padding(random(10) as i64)],
                        &[
                            Element::Offset {
                                x: random(9) as i64 - 4,
                                y: random(9) as i64 - 4,
                            },
                            padding(random(4) as i64),
                        ],
                        &[size(random(60) as i64, random(60) as i64)],
                        &[weight(1.0 + random(3) as f64, random(2) == 0)],
                        &[
                            align(Alignment::from_code(random(15) as i64).unwrap()),
                            width(50),
                        ],
                    ];
                    b.chain(node, chains[random(6) as usize]);
                }
                2 => {
                    // Move the node first among its parent's children.
                    let parent = b.tree.node(node).unwrap().parent.unwrap();
                    b.tree.place_after(parent, None, node);
                }
                3 => {
                    let arrangement = crate::properties::Arrangement::from_words(
                        random(9) as i64,
                        random(10) as i64,
                    )
                    .unwrap();
                    b.arrangement(node, arrangement);
                }
                _ => b
                    .tree
                    .resize(300 + random(200) as i64, 200 + random(200) as i64),
            }
            b.lay_out_with(&mut layout, &FixedAdvanceMetrics);
            let incremental = dump(&b.tree);
            let mut stack = vec![b.tree.root()];
            while let Some(id) = stack.pop() {
                stack.extend(b.tree.children(id));
                b.tree.mark_needs_measure(id);
            }
            b.lay_out_with(&mut layout, &FixedAdvanceMetrics);
            assert_eq!(
                incremental,
                dump(&b.tree),
                "after step {} of seed {}",
                step,
                initial
            );
            assert!(!b.tree.needs_layout());
        }
    }

    /// The scenario "A text keeping its size calls no policy".
    #[test]
    fn a_text_keeping_its_size_calls_no_policy() {
        let mut b = TreeBuilder::new(400, 100_000);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        b.set_policy(column, counting_policy());
        let texts: Vec<NodeId> = (0..100)
            .map(|index| b.text(column, &format!("text {:03}", index)))
            .collect();
        let mut layout = Layout::new();
        b.lay_out_with(&mut layout, &FixedAdvanceMetrics);
        let calls = policy_calls();
        b.tree.set_text(texts[50], "changed!");
        assert!(b.lay_out_with(&mut layout, &FixedAdvanceMetrics));
        assert_eq!(policy_calls(), calls, "a text of the same width");
        b.tree.set_text(texts[20], "a longer text");
        assert!(b.lay_out_with(&mut layout, &FixedAdvanceMetrics));
        assert_eq!(policy_calls(), calls + 1, "a wider text");
    }

    /// The misuse details of D13 that Rust code can reach in process, from a policy and from
    /// outside any layout.
    #[test]
    fn measure_policy_misuse_details() {
        use crate::error::UiError;
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let node = b.add(root, NodeKind::Layout);
        let text = b.text(node, "a");
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = seen.clone();
        b.set_policy(
            node,
            probe_policy(move |scope, measurables, c| {
                let mut seen = record.borrow_mut();
                let mut note = |result: Result<(), UiError>| {
                    seen.push(result.map_err(|error| error.message()).err());
                };
                let child = NodeId(measurables[0].node);
                let placeable = Position::child(child);
                note(place(scope, placeable, 0, 0));
                note(measured(child).map(|_| ()));
                let invalid = Constraints {
                    min_width: 10,
                    max_width: 5,
                    min_height: 0,
                    max_height: 0,
                };
                note(measure_child(child, invalid).map(|_| ()));
                let negative = Constraints { min_width: -1, ..c };
                note(measure_child(child, negative).map(|_| ()));
                note(measure_child(child, c).map(|_| ()));
                note(measure_child(child, c).map(|_| ()));
                note(begin_placement(placeable, 0, 0));
                note(place(scope, placeable, 0, 0));
                note(begin_placement(scope, 8, 20));
                note(begin_placement(scope, 1, 1));
                note(place(scope, placeable, 0, 0));
                note(end_placement(scope));
                note(place(scope, placeable, 0, 0));
                Size::new(8, 20)
            }),
        );
        lay_out(&mut b);
        let placed_outside =
            Some("ui misuse: a placeable was placed outside its parent's placement".to_string());
        let invalid = Some(
            "ui misuse: invalid constraints: a minimum is negative or above its maximum"
                .to_string(),
        );
        assert_eq!(
            *seen.borrow(),
            [
                placed_outside.clone(),
                placed_outside.clone(),
                invalid.clone(),
                invalid,
                None,
                Some("ui misuse: a measurable was measured twice in one measurement".to_string()),
                Some("ui misuse: layout was called outside its node's measurement".to_string()),
                placed_outside.clone(),
                None,
                Some("ui misuse: layout was called twice in one measurement".to_string()),
                None,
                None,
                placed_outside,
            ]
        );
        assert_eq!(bounds(&b, node), "0,0 8x20");
        assert_eq!(bounds(&b, text), "0,0 8x20");
        // Outside any layout.
        b.installed(|| {
            assert_eq!(
                measure_child(text, Constraints::up_to(10, 10)),
                Err(crate::error::MEASURED_OUTSIDE)
            );
            let scope = Position { node, element: 0 };
            assert_eq!(
                begin_placement(scope, 0, 0),
                Err(crate::error::LAYOUT_OUTSIDE)
            );
            assert_eq!(
                place(scope, Position::child(text), 0, 0),
                Err(crate::error::PLACED_OUTSIDE)
            );
        });
    }

    /// Nested policies three levels deep, each lent its own buffer: the middle one copies its
    /// measurables before measuring and finds its lent list unchanged after its children's
    /// policies ran, as the reviewer of D4 should probe.
    #[test]
    fn nested_policies_keep_their_lent_measurables() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let outer = b.add(root, NodeKind::Layout);
        let middle = b.add(outer, NodeKind::Layout);
        let first = b.text(outer, "abc");
        let inner = b.add(middle, NodeKind::Layout);
        let second = b.text(middle, "ab");
        let leaves = [b.text(inner, "a"), b.text(inner, "abcd")];
        let checks = std::rc::Rc::new(Cell::new(0));
        let counted = checks.clone();
        // Every level stacks its children vertically; the middle copies its list first.
        let stack = move |scope: Position, measurables: &[MeasurableMirror], c: Constraints| {
            let copy = measurables.to_vec();
            let mut sizes = Vec::new();
            for child in &copy {
                sizes.push(measure_child(NodeId(child.node), c.loosened()).unwrap());
            }
            if scope.node == middle {
                assert_eq!(measurables, copy.as_slice(), "the lent list stays");
                counted.set(counted.get() + 1);
            }
            let width = sizes.iter().map(|size| size.width).max().unwrap_or(0);
            let height = sizes.iter().map(|size| size.height).sum();
            begin_placement(scope, width, height).unwrap();
            let mut y = 0;
            for (child, size) in copy.iter().zip(&sizes) {
                place(scope, Position::child(NodeId(child.node)), 0, y).unwrap();
                y += size.height;
            }
            end_placement(scope).unwrap();
            Size::new(width, height)
        };
        let policy = probe_policy(stack);
        b.set_policy(outer, policy);
        let duplicate = crate::test_support::probe_policy_again();
        b.set_policy(middle, duplicate);
        b.set_policy(inner, crate::test_support::probe_policy_again());
        lay_out(&mut b);
        assert_eq!(checks.get(), 1);
        assert_eq!(bounds(&b, outer), "0,0 32x80");
        assert_eq!(bounds(&b, middle), "0,0 32x60");
        assert_eq!(bounds(&b, inner), "0,0 32x40");
        assert_eq!(bounds(&b, leaves[0]), "0,0 8x20");
        assert_eq!(bounds(&b, leaves[1]), "0,20 32x20");
        assert_eq!(bounds(&b, second), "0,40 16x20");
        assert_eq!(bounds(&b, first), "0,60 24x20");
    }

    /// An element measurer whose steps measure the rest of their chain and call `layout`
    /// without placing it.
    fn hiding_steps() -> FunctionValue {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static SLOT: AtomicUsize = AtomicUsize::new(0);
        static DESCRIPTOR: prism_runtime::Descriptor = prism_runtime::Descriptor {
            slot: &SLOT,
            drop_environment: None,
            equals_environment: None,
            duplicate_environment: None,
        };
        unsafe extern "C" fn hide(
            _: *mut MeasureResultMirror,
            _: *mut u8,
            scope: *const MeasureScopeMirror,
            _: *const ElementMirror,
            measurable: *const MeasurableMirror,
            constraints: *const ConstraintsMirror,
        ) {
            let scope = Position::of_words((*scope).node, (*scope).element);
            let rest = Position::of_words((*measurable).node, (*measurable).element);
            let size = measure_position(rest, (*constraints).into()).unwrap();
            begin_placement(scope, size.width, size.height).unwrap();
            end_placement(scope).unwrap();
        }
        SLOT.store(hide as *const () as usize, Ordering::Release);
        FunctionValue {
            descriptor: &DESCRIPTOR,
            environment: std::ptr::null_mut(),
        }
    }

    /// The scenario "A layout modifier that hides its content", from Rust: a step that does not
    /// place the rest of its chain ends the program.
    fn hidden_content_in_child() -> ! {
        let mut b = TreeBuilder::new(400, 300);
        b.steps = hiding_steps;
        let root = b.root();
        let text = b.text(root, "a");
        b.chain(text, &[padding(4)]);
        lay_out(&mut b);
        unreachable!("the layout pass ends the program")
    }

    #[test]
    fn a_step_that_does_not_place_its_content() {
        let output = run_in_child(
            "layout::tests::a_step_that_does_not_place_its_content",
            hidden_content_in_child,
        );
        assert_eq!(output.status.code(), Some(101));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "panic: ui misuse: a layout modifier did not place its content\n"
        );
    }

    /// D1: steps place the rest of their chain, so an offset moves the elements after it, the
    /// content and the children, and leaves the node's size; positions sum along the chain.
    #[test]
    fn offsets_and_paddings_move_the_rest_of_the_chain() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        b.chain(
            column,
            &[
                padding(4),
                Element::Offset { x: 10, y: 5 },
                background(0xFFFF_0000, Shape::RECTANGLE),
                padding(2),
            ],
        );
        let text = b.text(column, "a");
        lay_out(&mut b);
        assert_eq!(bounds(&b, column), "0,0 20x32");
        assert_eq!(bounds(&b, text), "16,11 8x20");
        let node = b.tree.node(column).unwrap();
        assert_eq!(node.chain[2].bounds, Rect::new(14, 9, 12, 24));
        assert_eq!(node.content, Rect::new(16, 11, 8, 20));
        // Moving the offset moves the child it placed, through a relayout of the column only.
        b.chain(
            column,
            &[
                padding(4),
                Element::Offset { x: 0, y: 0 },
                background(0xFFFF_0000, Shape::RECTANGLE),
                padding(2),
            ],
        );
        lay_out(&mut b);
        assert_eq!(bounds(&b, text), "6,6 8x20");
    }

    /// D7: a child its parent does not place keeps its last bounds, and is printed unplaced.
    #[test]
    fn an_unplaced_child_is_dumped_unplaced() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let node = b.add(root, NodeKind::Layout);
        b.set_policy(node, first_only_policy());
        b.text(node, "shown");
        b.text(node, "hidden");
        lay_out(&mut b);
        let mut out = String::new();
        crate::dump::dump_tree(&b.tree, None, &mut out);
        assert_eq!(
            out,
            "window 400x300\n  layout #1 0,0 40x20\n    text #2 0,0 40x20 \"shown\"\n    text #3 0,0 48x20 \"hidden\" unplaced\n"
        );
    }
}
