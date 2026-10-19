//! Constraint layout (D11 of `ui-toolkit`): one measure-and-place pass over the tree, in
//! whole logical pixels.
//!
//! Measuring a node under constraints folds its modifier chain from the first element,
//! Compose's outermost, each layout element changing the constraints it passes on and the
//! size it gets back, then measures the content with the kind's policy under what remains:
//!
//! - `padding` deflates the constraints, floored at 0, offsets the rest, and returns the
//!   rest's size plus the padding, clamped into its constraints;
//! - `size` makes each set axis exact, clamped into the constraints;
//! - `defaultMinSize` raises a minimum of 0 to a specified value, at most the maximum;
//! - `fillMax*` makes each bounded axis exactly the maximum times the fraction, rounded down;
//! - `weight` and `align` are parent data, read by the parent, the first in a chain counting;
//! - drawing and input elements record the bounds at their position, the offset of the
//!   elements before them and the size of the elements after them.
//!
//! The fold is a loop over at most sixteen elements with its state on the stack; recursion is
//! only from a node to its children. Children are placed relative to their parent, so a clean
//! subtree needs no placement.
//!
//! **The cache.** A node that is neither `needs_measure` nor `child_needs_measure` and is
//! measured with its last constraints returns its last size without visiting its subtree. A
//! node that is only `child_needs_measure` re-measures its dirty children with their last
//! constraints, and when each keeps its size, keeps its own size and its children's positions
//! without running its policy; otherwise its policy runs. So a relayout is linear in the dirty
//! nodes, their ancestors and those ancestors' children, and a change that keeps a node's size
//! costs a flag check per sibling on its path.

use crate::geometry::{Constraints, Rect, Size, UNBOUNDED};
use crate::handle::NodeId;
use crate::modifier::{Element, CAPACITY};
use crate::node::NodeKind;
use crate::properties::Alignment;
use crate::text::{break_lines, line_width, Line, TextMeasurer};
use crate::tree::UiTree;

/// The layout pass's reused buffers and counters.
#[derive(Debug, Default)]
pub struct Layout {
    lines: Vec<Line>,
    /// How many nodes the last pass measured, cached ones not counted.
    pub measured: usize,
}

impl Layout {
    pub fn new() -> Layout {
        Layout::default()
    }

    /// Lays out the tree when anything in it needs measuring, and gives whether it did, which
    /// makes the display list stale.
    pub fn run(&mut self, tree: &mut UiTree, measurer: &dyn TextMeasurer) -> bool {
        self.measured = 0;
        if !tree.needs_layout() {
            return false;
        }
        let root = tree.root();
        let window = tree.window_size();
        let constraints = Constraints::up_to(window.width, window.height);
        let root_node = tree.node(root).unwrap();
        let from_root = root_node.needs_measure || root_node.last_constraints != Some(constraints);
        let mut dirty = std::mem::take(&mut tree.dirty);
        let mut context = Context {
            tree,
            measurer,
            lines: &mut self.lines,
            measured: 0,
        };
        if from_root {
            context.measure(root, constraints);
        } else {
            for &id in &dirty {
                context.relayout_from(id);
            }
            for &id in &dirty {
                context.clear_ancestors(id);
            }
        }
        self.measured = context.measured;
        dirty.clear();
        tree.dirty = dirty;
        tree.redraw = true;
        true
    }
}

struct Context<'a> {
    tree: &'a mut UiTree,
    measurer: &'a dyn TextMeasurer,
    lines: &'a mut Vec<Line>,
    measured: usize,
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

/// The share of the maximum `fraction` gives, rounded down.
fn fraction_of(maximum: i64, fraction: f64) -> i64 {
    let value = (maximum as f64 * fraction).floor();
    if value.is_nan() {
        0
    } else {
        value as i64
    }
}

impl Context<'_> {
    /// Measures `id` under `constraints`, records its layout and gives its size.
    fn measure(&mut self, id: NodeId, constraints: Constraints) -> Size {
        let node = self.tree.node(id).unwrap();
        if !node.needs_measure
            && node.last_constraints == Some(constraints)
            && (!node.child_needs_measure || self.remeasure_dirty_children(id))
        {
            let node = self.tree.node_mut(id).unwrap();
            node.child_needs_measure = false;
            return node.size;
        }
        let node = self.tree.node(id).unwrap();
        self.measured += 1;
        let count = node.chain.len().min(CAPACITY);
        // The constraints each element receives and the offset of the rest at its position;
        // index `count` is what the kind's policy receives.
        let mut received = [constraints; CAPACITY + 1];
        let mut offsets = [(0i64, 0i64); CAPACITY + 1];
        for index in 0..count {
            let c = received[index];
            let (x, y) = offsets[index];
            let (next, offset) = match node.chain[index].element {
                Element::Padding {
                    start,
                    top,
                    end,
                    bottom,
                } => (c.deflated(start + end, top + bottom), (x + start, y + top)),
                Element::Size { width, height } => {
                    let mut next = c;
                    if let Some(width) = width {
                        next = next.with_exact_width(width);
                    }
                    if let Some(height) = height {
                        next = next.with_exact_height(height);
                    }
                    (next, (x, y))
                }
                Element::DefaultMinSize { width, height } => {
                    let mut next = c;
                    if let (Some(width), 0) = (width, c.min_width) {
                        next.min_width = width.min(c.max_width);
                    }
                    if let (Some(height), 0) = (height, c.min_height) {
                        next.min_height = height.min(c.max_height);
                    }
                    (next, (x, y))
                }
                Element::FillMax {
                    width,
                    height,
                    fraction,
                } => {
                    let mut next = c;
                    if width && c.max_width != UNBOUNDED {
                        next = next.with_exact_width(fraction_of(c.max_width, fraction));
                    }
                    if height && c.max_height != UNBOUNDED {
                        next = next.with_exact_height(fraction_of(c.max_height, fraction));
                    }
                    (next, (x, y))
                }
                _ => (c, (x, y)),
            };
            received[index + 1] = next;
            offsets[index + 1] = offset;
        }
        let (content_x, content_y) = offsets[count];
        let content = self.measure_content(id, received[count], content_x, content_y);
        // Back from the content to the outermost element.
        let node = self.tree.node_mut(id).unwrap();
        let mut size = content;
        for index in (0..count).rev() {
            let entry = &mut node.chain[index];
            match entry.element {
                Element::Padding {
                    start,
                    top,
                    end,
                    bottom,
                } => {
                    size = received[index].constrain(Size::new(
                        size.width + start + end,
                        size.height + top + bottom,
                    ));
                }
                element if element.records_bounds() => {
                    let (x, y) = offsets[index];
                    entry.bounds = Rect::new(x, y, size.width, size.height);
                }
                _ => {}
            }
        }
        if node.kind == NodeKind::Window {
            size = node.size;
        }
        node.content = Rect::new(content_x, content_y, content.width, content.height);
        node.size = size;
        node.last_constraints = Some(constraints);
        node.needs_measure = false;
        node.child_needs_measure = false;
        size
    }

    /// A relayout from a node marked for measuring: re-measures it with its last
    /// constraints, which stay its own while its siblings and ancestors are unchanged, and,
    /// when its size changed, its parent with its policy, and so on up, stopping at the first
    /// node whose size stays. A node never measured is measured by its parent's policy.
    fn relayout_from(&mut self, id: NodeId) {
        let mut current = id;
        loop {
            let Some(node) = self.tree.node(current) else {
                return;
            };
            if !node.needs_measure && !node.child_needs_measure {
                return;
            }
            let parent = node.parent;
            let measured = match node.last_constraints {
                Some(last) => {
                    let old = node.size;
                    self.measure(current, last) != old
                }
                None => true,
            };
            let Some(parent) = parent.filter(|_| measured) else {
                return;
            };
            self.tree.node_mut(parent).unwrap().needs_measure = true;
            current = parent;
        }
    }

    /// Clears `child_needs_measure` on the ancestors of `id` once every dirty node has been
    /// measured.
    fn clear_ancestors(&mut self, id: NodeId) {
        let mut parent = self.tree.node(id).and_then(|node| node.parent);
        while let Some(ancestor) = parent {
            let node = self.tree.node_mut(ancestor).unwrap();
            node.child_needs_measure = false;
            parent = node.parent;
        }
    }

    /// For a node that is clean itself and measured with its last constraints: re-measures
    /// its dirty children with their last constraints, which are still theirs while every
    /// sibling keeps its size, and gives whether every one kept its size, in which case the
    /// node's size and its children's positions stand. Otherwise the caller measures the node
    /// again with its policy.
    fn remeasure_dirty_children(&mut self, id: NodeId) -> bool {
        let mut child = self.tree.node(id).unwrap().first_child;
        while let Some(current) = child {
            let node = self.tree.node(current).unwrap();
            child = node.next;
            if !node.needs_measure && !node.child_needs_measure {
                continue;
            }
            let (Some(last), size) = (node.last_constraints, node.size) else {
                return false;
            };
            if self.measure(current, last) != size {
                return false;
            }
        }
        true
    }

    /// Measures a node's content with its kind's policy, placing its children at
    /// `(x, y)` plus their position in the content.
    fn measure_content(&mut self, id: NodeId, c: Constraints, x: i64, y: i64) -> Size {
        let node = self.tree.node(id).unwrap();
        match node.kind {
            NodeKind::Text => {
                let font_size = node.style.resolved_font_size();
                break_lines(
                    node.text.as_str(),
                    c.max_width,
                    font_size,
                    self.measurer,
                    self.lines,
                );
                let width = self.lines.iter().map(|line| line.width).max().unwrap_or(0);
                let height = self.lines.len() as i64 * self.measurer.line_height(font_size);
                self.tree.node_mut(id).unwrap().break_width = c.max_width;
                c.constrain(Size::new(width, height))
            }
            NodeKind::TextField => {
                let font_size = node.style.resolved_font_size();
                let width = line_width(node.text.as_str(), font_size, self.measurer);
                c.constrain(Size::new(width, self.measurer.line_height(font_size)))
            }
            NodeKind::Spacer => c.constrain(Size::ZERO),
            NodeKind::Window => {
                let window = node.size;
                self.measure_box(
                    id,
                    Constraints::up_to(window.width, window.height),
                    Alignment::TOP_START,
                    x,
                    y,
                );
                window
            }
            NodeKind::Box => {
                let alignment = node.alignment;
                self.measure_box(id, c, alignment, x, y)
            }
            NodeKind::Column => self.measure_line(id, c, true, x, y),
            NodeKind::Row => self.measure_line(id, c, false, x, y),
        }
    }

    /// A box: children under the constraints loosened, the size of the largest at least the
    /// minimum, each child placed by its own alignment or the box's.
    fn measure_box(
        &mut self,
        id: NodeId,
        c: Constraints,
        alignment: Alignment,
        x: i64,
        y: i64,
    ) -> Size {
        let loose = c.loosened();
        let mut largest = Size::ZERO;
        let mut child = self.tree.node(id).unwrap().first_child;
        while let Some(current) = child {
            let size = self.measure(current, loose);
            largest.width = largest.width.max(size.width);
            largest.height = largest.height.max(size.height);
            child = self.tree.node(current).unwrap().next;
        }
        let size = c.constrain(largest);
        let mut child = self.tree.node(id).unwrap().first_child;
        while let Some(current) = child {
            let node = self.tree.node_mut(current).unwrap();
            let own = node.own_alignment().unwrap_or(alignment);
            node.x = x + own.horizontal().offset(size.width - node.size.width);
            node.y = y + own.vertical().offset(size.height - node.size.height);
            child = node.next;
        }
        size
    }

    /// A column (`vertical`) or a row: unweighted children in order under what remains of the
    /// main axis, then weighted children sharing what is left of a bounded main axis, then
    /// placement by the arrangement and each child's alignment.
    fn measure_line(&mut self, id: NodeId, c: Constraints, vertical: bool, x: i64, y: i64) -> Size {
        let node = self.tree.node(id).unwrap();
        let arrangement = node.arrangement;
        let alignment = node.alignment;
        let spacing = arrangement.spacing();
        let (main_max, cross_max) = if vertical {
            (c.max_height, c.max_width)
        } else {
            (c.max_width, c.max_height)
        };
        let bounded = main_max != UNBOUNDED;
        let mut count: i64 = 0;
        let mut measured: i64 = 0;
        let mut used: i64 = 0;
        let mut cross: i64 = 0;
        let mut total_weight = 0.0;
        let mut weighted: i64 = 0;
        let mut child = node.first_child;
        while let Some(current) = child {
            let node = self.tree.node(current).unwrap();
            let next = node.next;
            count += 1;
            match node.weight().filter(|(weight, _)| bounded && *weight > 0.0) {
                Some((weight, _)) => {
                    total_weight += weight;
                    weighted += 1;
                }
                None => {
                    let remaining = if bounded {
                        (main_max - used - spacing * measured).max(0)
                    } else {
                        UNBOUNDED
                    };
                    let size =
                        self.measure(current, oriented(vertical, 0, remaining, 0, cross_max));
                    used += main_of(vertical, size);
                    cross = cross.max(cross_of(vertical, size));
                    measured += 1;
                }
            }
            child = next;
        }
        let gaps = spacing * (count - 1).max(0);
        if weighted > 0 {
            let remaining = (main_max - used - gaps).max(0);
            let mut given = 0;
            let mut seen = 0;
            let mut child = self.tree.node(id).unwrap().first_child;
            while let Some(current) = child {
                let node = self.tree.node(current).unwrap();
                let next = node.next;
                if let Some((weight, fill)) = node.weight().filter(|(weight, _)| *weight > 0.0) {
                    seen += 1;
                    let share = if seen == weighted {
                        remaining - given
                    } else {
                        fraction_of(remaining, weight / total_weight)
                    };
                    given += share;
                    let main_min = if fill { share } else { 0 };
                    let size =
                        self.measure(current, oriented(vertical, main_min, share, 0, cross_max));
                    used += main_of(vertical, size);
                    cross = cross.max(cross_of(vertical, size));
                }
                child = next;
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
        let mut position = start;
        let mut child = self.tree.node(id).unwrap().first_child;
        while let Some(current) = child {
            let node = self.tree.node_mut(current).unwrap();
            let own = node.own_alignment().unwrap_or(alignment);
            let part = if vertical {
                own.horizontal()
            } else {
                own.vertical()
            };
            let along = part.offset(cross_size - cross_of(vertical, node.size));
            if vertical {
                node.x = x + along;
                node.y = y + position;
            } else {
                node.x = x + position;
                node.y = y + along;
            }
            position += main_of(vertical, node.size) + spacing + gap;
            child = node.next;
        }
        size
    }
}

/// The absolute bounds of `id`, from the positions relative to each parent.
pub fn absolute_bounds(tree: &UiTree, id: NodeId) -> Rect {
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
    use crate::properties::{Arrangement, Shape};
    use crate::test_support::*;
    use crate::text::FixedAdvanceMetrics;
    use std::cell::Cell;

    fn lay_out(builder: &mut TreeBuilder) {
        Layout::new().run(&mut builder.tree, &FixedAdvanceMetrics);
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
        assert!(layout.run(&mut b.tree, &measurer));
        assert_eq!(measurer.0.get(), 100 * 8);
        assert_eq!(layout.measured, 102);
        measurer.0.set(0);
        assert!(
            !layout.run(&mut b.tree, &measurer),
            "a clean tree is not laid out"
        );
        b.tree.set_text(texts[50], "changed!");
        assert!(layout.run(&mut b.tree, &measurer));
        assert_eq!(measurer.0.get(), 8, "only the changed text's characters");
        assert_eq!(
            layout.measured, 1,
            "the column and the root keep their sizes"
        );
        // A text that changes its size re-measures its column, whose other children are
        // cached.
        measurer.0.set(0);
        b.tree.set_text(texts[20], "a longer text");
        assert!(layout.run(&mut b.tree, &measurer));
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
        let dump = |tree: &UiTree| {
            let mut out = String::new();
            crate::dump::dump_tree(tree, None, &mut out);
            out
        };
        let mut layout = Layout::new();
        layout.run(&mut b.tree, &FixedAdvanceMetrics);
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
                    let chains: [&[Element]; 5] = [
                        &[],
                        &[padding(random(10) as i64)],
                        &[size(random(60) as i64, random(60) as i64)],
                        &[weight(1.0 + random(3) as f64, random(2) == 0)],
                        &[
                            align(Alignment::from_code(random(15) as i64).unwrap()),
                            width(50),
                        ],
                    ];
                    b.chain(node, chains[random(5) as usize]);
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
            layout.run(&mut b.tree, &FixedAdvanceMetrics);
            let incremental = dump(&b.tree);
            let mut stack = vec![b.tree.root()];
            while let Some(id) = stack.pop() {
                stack.extend(b.tree.children(id));
                b.tree.mark_needs_measure(id);
            }
            layout.run(&mut b.tree, &FixedAdvanceMetrics);
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
}
