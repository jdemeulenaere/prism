//! The display list (D13 of `ui-toolkit`): what a frame draws, built from the laid-out tree in
//! one walk, parents before children.
//!
//! The list starts with a clear of the window. Each node then adds its drawing elements in
//! chain order, each in the bounds recorded at its position: `background` a rectangle,
//! `border` a border, and a text field's frame a border of 1 in its first colour, or of 2 in
//! its second when the node is focused. Then come the kind's own commands: a text run per
//! line of a text, and a text field's value with, when focused, its caret. An unspecified text
//! colour is the content colour of the nearest node that sets one, the node itself included,
//! or black (D9). The vectors are cleared and reused, so once they have grown a build
//! allocates nothing.

use crate::geometry::Rect;
use crate::handle::NodeId;
use crate::modifier::Element;
use crate::node::NodeKind;
use crate::properties::Color;
use crate::text::{break_lines, line_width, Line, TextMeasurer};
use crate::tree::UiTree;
use std::ops::Range;

/// One drawing command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawCommand {
    Clear {
        width: i64,
        height: i64,
        color: Color,
    },
    Rectangle {
        bounds: Rect,
        color: Color,
        corner_radius: i64,
    },
    Border {
        bounds: Rect,
        width: i64,
        color: Color,
        corner_radius: i64,
    },
    /// A run of text whose line box starts at `(x, y)`; `text` is a range of the list's text.
    Text {
        x: i64,
        y: i64,
        font_size: i64,
        color: Color,
        text: Range<usize>,
    },
}

/// The window's background (D9).
pub const WINDOW_COLOR: Color = Color::WHITE;

/// A pending node of the walk: the node, its parent's absolute origin and the content colour
/// it inherits.
#[derive(Debug, Clone, Copy)]
struct Pending {
    node: NodeId,
    x: i64,
    y: i64,
    content_color: Color,
}

/// The commands of a frame, and the text their runs show.
#[derive(Debug, Default)]
pub struct DisplayList {
    pub commands: Vec<DrawCommand>,
    pub text: String,
    stack: Vec<Pending>,
    lines: Vec<Line>,
}

impl DisplayList {
    pub fn new() -> DisplayList {
        DisplayList::default()
    }

    /// The text of a run.
    pub fn text_of(&self, range: &Range<usize>) -> &str {
        &self.text[range.clone()]
    }

    /// Rebuilds the list from the laid-out tree; `focused` is the focused text field.
    pub fn build(&mut self, tree: &UiTree, measurer: &dyn TextMeasurer, focused: Option<NodeId>) {
        self.commands.clear();
        self.text.clear();
        self.stack.clear();
        let window = tree.window_size();
        self.commands.push(DrawCommand::Clear {
            width: window.width,
            height: window.height,
            color: WINDOW_COLOR,
        });
        self.push_children(tree, tree.root(), 0, 0, Color::UNSPECIFIED);
        while let Some(pending) = self.stack.pop() {
            let node = tree.node(pending.node).unwrap();
            let (x, y) = (pending.x + node.x, pending.y + node.y);
            let content_color = node.content_color.or(pending.content_color);
            let is_focused = focused == Some(pending.node);
            let mut caret_color = None;
            for entry in &node.chain {
                let bounds = entry.bounds.offset(x, y);
                match entry.element {
                    Element::Background { color, shape } => {
                        self.commands.push(DrawCommand::Rectangle {
                            bounds,
                            color,
                            corner_radius: shape.radius(bounds.width, bounds.height),
                        })
                    }
                    Element::Border {
                        width,
                        color,
                        shape,
                    } => self.commands.push(DrawCommand::Border {
                        bounds,
                        width,
                        color,
                        corner_radius: shape.radius(bounds.width, bounds.height),
                    }),
                    Element::TextFieldFrame { unfocused, focused } => {
                        let (width, color) = if is_focused {
                            (2, focused)
                        } else {
                            (1, unfocused)
                        };
                        caret_color = Some(focused);
                        self.commands.push(DrawCommand::Border {
                            bounds,
                            width,
                            color,
                            corner_radius: 0,
                        });
                    }
                    _ => {}
                }
            }
            let text_x = x + node.content.x;
            let text_y = y + node.content.y;
            let font_size = node.style.resolved_font_size();
            let color = node.style.color.or(content_color).or(Color::BLACK);
            match node.kind {
                NodeKind::Text => {
                    break_lines(
                        node.text.as_str(),
                        node.break_width,
                        font_size,
                        measurer,
                        &mut self.lines,
                    );
                    let line_height = measurer.line_height(font_size);
                    for index in 0..self.lines.len() {
                        let line = self.lines[index];
                        self.run(
                            &node.text.as_str()[line.start..line.end],
                            text_x,
                            text_y + index as i64 * line_height,
                            font_size,
                            color,
                        );
                    }
                }
                NodeKind::TextField => {
                    let value = node.text.as_str();
                    self.run(value, text_x, text_y, font_size, color);
                    if is_focused {
                        self.commands.push(DrawCommand::Rectangle {
                            bounds: Rect::new(
                                text_x + line_width(value, font_size, measurer),
                                text_y,
                                1,
                                measurer.line_height(font_size),
                            ),
                            color: caret_color.unwrap_or(color),
                            corner_radius: 0,
                        });
                    }
                }
                _ => {}
            }
            self.push_children(tree, pending.node, x, y, content_color);
        }
    }

    /// A text run, unless the text is empty.
    fn run(&mut self, text: &str, x: i64, y: i64, font_size: i64, color: Color) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        self.commands.push(DrawCommand::Text {
            x,
            y,
            font_size,
            color,
            text: start..self.text.len(),
        });
    }

    /// Pushes `parent`'s children so that the first is popped first.
    fn push_children(
        &mut self,
        tree: &UiTree,
        parent: NodeId,
        x: i64,
        y: i64,
        content_color: Color,
    ) {
        let mut child = tree.node(parent).unwrap().last_child;
        while let Some(current) = child {
            self.stack.push(Pending {
                node: current,
                x,
                y,
                content_color,
            });
            child = tree.node(current).unwrap().previous;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump::dump_display_list;
    use crate::layout::Layout;
    use crate::properties::{Shape, TextStyle};
    use crate::test_support::*;
    use crate::text::FixedAdvanceMetrics;

    fn drawn(builder: &mut TreeBuilder, focused: Option<NodeId>) -> String {
        Layout::new().run(&mut builder.tree, &FixedAdvanceMetrics);
        let mut list = DisplayList::new();
        list.build(&builder.tree, &FixedAdvanceMetrics, focused);
        let mut out = String::new();
        dump_display_list(&list, &mut out);
        out
    }

    #[test]
    fn a_border_and_a_text() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let boxed = b.add(root, NodeKind::Box);
        b.chain(boxed, &[border(1, 0xFF00_0000, Shape::RECTANGLE)]);
        b.text(boxed, "ab");
        assert_eq!(
            drawn(&mut b, None),
            "clear 400x300 #FFFFFFFF\nborder 0,0 16x20 width 1 #FF000000\ntext 0,0 size 16 #FF000000 \"ab\"\n"
        );
    }

    #[test]
    fn order_decides_what_a_background_covers() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        let inside = b.add(column, NodeKind::Box);
        b.chain(
            inside,
            &[padding(8), background(0xFFFF_0000, Shape::RECTANGLE)],
        );
        b.text(inside, "a");
        let under = b.add(column, NodeKind::Box);
        b.chain(
            under,
            &[background(0xFF00_00FF, Shape::RECTANGLE), padding(8)],
        );
        b.text(under, "b");
        assert_eq!(
            drawn(&mut b, None),
            "clear 400x300 #FFFFFFFF\n\
             rect 8,8 8x20 #FFFF0000\n\
             text 8,8 size 16 #FF000000 \"a\"\n\
             rect 0,36 24x36 #FF0000FF\n\
             text 8,44 size 16 #FF000000 \"b\"\n"
        );
    }

    #[test]
    fn rounded_shapes_resolve_against_their_bounds() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        let rounded = b.add(column, NodeKind::Box);
        b.chain(
            rounded,
            &[background(0xFFFF_0000, Shape::new(4, 0)), padding(8)],
        );
        b.text(rounded, "a");
        let circle = b.add(column, NodeKind::Box);
        b.chain(
            circle,
            &[border(2, 0xFF00_00FF, Shape::new(0, 50)), size(40, 40)],
        );
        assert_eq!(
            drawn(&mut b, None),
            "clear 400x300 #FFFFFFFF\n\
             rect 0,0 24x36 #FFFF0000 radius 4\n\
             text 8,8 size 16 #FF000000 \"a\"\n\
             border 0,36 40x40 width 2 #FF0000FF radius 20\n"
        );
    }

    #[test]
    fn a_later_element_draws_over_an_earlier_one() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let boxed = b.add(root, NodeKind::Box);
        b.chain(
            boxed,
            &[
                size(20, 20),
                border(2, 0xFF00_00FF, Shape::RECTANGLE),
                background(0xFFFF_0000, Shape::RECTANGLE),
            ],
        );
        assert_eq!(
            drawn(&mut b, None),
            "clear 400x300 #FFFFFFFF\nborder 0,0 20x20 width 2 #FF0000FF\nrect 0,0 20x20 #FFFF0000\n"
        );
    }

    #[test]
    fn a_button_label_is_white() {
        let mut b = TreeBuilder::new(400, 300);
        counter_screen(&mut b, "Count: 0", inline_handler(1));
        assert_eq!(
            drawn(&mut b, None),
            "clear 400x300 #FFFFFFFF\n\
             text 16,16 size 16 #FF000000 \"Count: 0\"\n\
             rect 16,44 58x40 #FF6750A4 radius 20\n\
             text 41,54 size 16 #FFFFFFFF \"+\"\n"
        );
    }

    #[test]
    fn an_unspecified_text_colour_inside_and_outside_a_content_colour() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        b.text(column, "a");
        let surface = b.add(column, NodeKind::Box);
        b.chain(surface, &[background(0xFF00_0000, Shape::RECTANGLE)])
            .content_color(surface, 0xFFFF_FFFF);
        let inner = b.add(surface, NodeKind::Column);
        b.text(inner, "b");
        let explicit = b.text(inner, "c");
        b.style(explicit, 0xFF00_00FF, -1);
        assert_eq!(
            drawn(&mut b, None),
            "clear 400x300 #FFFFFFFF\n\
             text 0,0 size 16 #FF000000 \"a\"\n\
             rect 0,20 8x40 #FF000000\n\
             text 0,20 size 16 #FFFFFFFF \"b\"\n\
             text 0,40 size 16 #FF0000FF \"c\"\n"
        );
    }

    #[test]
    fn wrapped_text_draws_a_run_per_line() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let text = b.text(root, "aa bb cc");
        b.chain(text, &[width(40)]);
        assert_eq!(
            drawn(&mut b, None),
            "clear 400x300 #FFFFFFFF\n\
             text 0,0 size 16 #FF000000 \"aa bb\"\n\
             text 0,20 size 16 #FF000000 \"cc\"\n"
        );
    }

    #[test]
    fn a_focused_text_fields_frame_and_caret() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let field = b.text_field(root, "ac");
        b.chain(
            field,
            &[
                default_min_size(200, -1),
                Element::TextFieldFrame {
                    unfocused: Color(0xFF79_747E),
                    focused: Color(0xFF67_50A4),
                },
                padding(8),
            ],
        );
        b.tree.set_text_style(field, TextStyle::DEFAULT);
        assert_eq!(
            drawn(&mut b, None),
            "clear 400x300 #FFFFFFFF\n\
             border 0,0 200x36 width 1 #FF79747E\n\
             text 8,8 size 16 #FF000000 \"ac\"\n"
        );
        assert_eq!(
            drawn(&mut b, Some(field)),
            "clear 400x300 #FFFFFFFF\n\
             border 0,0 200x36 width 2 #FF6750A4\n\
             text 8,8 size 16 #FF000000 \"ac\"\n\
             rect 24,8 1x20 #FF6750A4\n"
        );
    }

    #[test]
    fn a_rebuild_reuses_its_buffers() {
        let mut b = TreeBuilder::new(400, 300);
        counter_screen(&mut b, "Count: 0", inline_handler(1));
        Layout::new().run(&mut b.tree, &FixedAdvanceMetrics);
        let mut list = DisplayList::new();
        list.build(&b.tree, &FixedAdvanceMetrics, None);
        let (commands, text) = (list.commands.capacity(), list.text.capacity());
        list.build(&b.tree, &FixedAdvanceMetrics, None);
        assert_eq!(list.commands.len(), 4);
        assert_eq!(list.commands.capacity(), commands);
        assert_eq!(list.text.capacity(), text);
    }
}
