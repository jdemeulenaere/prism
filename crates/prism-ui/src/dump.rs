//! Text dumps of the laid-out tree and the display list (D16 of `ui-toolkit`), exact and
//! platform-independent so that fixtures can pin them.
//!
//! - **`dump`** prints `window <width>x<height>`, then a line per node in tree order, indented
//!   two spaces per depth: `<kind> #<number> <x>,<y> <width>x<height>` with absolute bounds,
//!   then the node's non-default properties in a fixed order (the quoted text, `color`,
//!   `fontSize`, `contentColor`, `disabled`, `focused`, `arrangement`, `alignment`,
//!   `unplaced`), then its
//!   modifier elements in chain order with their arguments.
//! - **`dump draw`** prints one display list command per line: `clear`, `rect`, `border` and
//!   `text`, with `radius <r>` after a rounded rectangle or border.
//!
//! Colours print as `#AARRGGBB`, and texts are quoted with `\"`, `\\` and `\n` escaped.

use crate::draw::{DisplayList, DrawCommand};
use crate::handle::NodeId;
use crate::layout::absolute_bounds;
use crate::modifier::Element;
use crate::node::{Node, NodeKind};
use crate::properties::{Shape, DEFAULT_FONT_SIZE};
use crate::tree::UiTree;
use prism_runtime::format_double;
use std::fmt::Write;

/// Appends `text` quoted, with `"`, `\` and newlines escaped.
pub fn write_quoted(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
}

/// Appends the dump of the tree; `focused` is the focused text field.
pub fn dump_tree(tree: &UiTree, focused: Option<NodeId>, out: &mut String) {
    let window = tree.window_size();
    let _ = writeln!(out, "window {}x{}", window.width, window.height);
    // Depth-first without recursion: (node, depth, absolute origin of its parent).
    let mut stack: Vec<(NodeId, usize, i64, i64)> = Vec::new();
    let push_children =
        |stack: &mut Vec<(NodeId, usize, i64, i64)>, parent: NodeId, depth, x, y| {
            let mut child = tree.node(parent).unwrap().last_child;
            while let Some(current) = child {
                stack.push((current, depth, x, y));
                child = tree.node(current).unwrap().previous;
            }
        };
    push_children(&mut stack, tree.root(), 1, 0, 0);
    while let Some((id, depth, parent_x, parent_y)) = stack.pop() {
        let node = tree.node(id).unwrap();
        let (x, y) = (parent_x + node.x, parent_y + node.y);
        for _ in 0..depth {
            out.push_str("  ");
        }
        let _ = write!(
            out,
            "{} #{} {},{} {}x{}",
            node.kind.name(),
            node.number,
            x,
            y,
            node.size.width,
            node.size.height
        );
        write_properties(out, node, focused == Some(id));
        for entry in &node.chain {
            out.push(' ');
            write_element(out, &entry.element);
        }
        out.push('\n');
        push_children(&mut stack, id, depth + 1, x, y);
    }
}

fn write_properties(out: &mut String, node: &Node, focused: bool) {
    if !node.text.as_str().is_empty() {
        out.push(' ');
        write_quoted(out, node.text.as_str());
    }
    if node.style.color.is_specified() {
        let _ = write!(out, " color {}", node.style.color);
    }
    if node.style.resolved_font_size() != DEFAULT_FONT_SIZE {
        let _ = write!(out, " fontSize {}", node.style.resolved_font_size());
    }
    if node.content_color.is_specified() {
        let _ = write!(out, " contentColor {}", node.content_color);
    }
    if !node.enabled {
        out.push_str(" disabled");
    }
    if focused {
        out.push_str(" focused");
    }
    let lays_out_children = matches!(node.kind, NodeKind::Column | NodeKind::Row | NodeKind::Box);
    if matches!(node.kind, NodeKind::Column | NodeKind::Row)
        && node.arrangement != node.kind.default_arrangement()
    {
        let _ = write!(out, " arrangement {}", node.arrangement);
    }
    if lays_out_children && node.alignment != node.kind.default_alignment() {
        let _ = write!(out, " alignment {}", node.alignment);
    }
    if !node.placed {
        out.push_str(" unplaced");
    }
}

fn write_shape(out: &mut String, shape: Shape) {
    if !shape.is_rectangle() {
        let _ = write!(out, " {}", shape);
    }
}

/// Appends one element as dumps write it.
pub fn write_element(out: &mut String, element: &Element) {
    let axis = |value: Option<i64>| value.map_or("-".to_string(), |value| value.to_string());
    let fraction = |fraction: f64| {
        if fraction == 1.0 {
            String::new()
        } else {
            format!("({})", format_double(fraction))
        }
    };
    let _ = match *element {
        Element::Padding {
            start,
            top,
            end,
            bottom,
        } => {
            if start == top && top == end && end == bottom {
                write!(out, "padding({})", start)
            } else {
                write!(out, "padding({},{},{},{})", start, top, end, bottom)
            }
        }
        Element::Size {
            width: Some(width),
            height: None,
        } => write!(out, "width({})", width),
        Element::Size {
            width: None,
            height: Some(height),
        } => write!(out, "height({})", height),
        Element::Size { width, height } => write!(out, "size({}x{})", axis(width), axis(height)),
        Element::DefaultMinSize { width, height } => {
            write!(out, "defaultMinSize({}x{})", axis(width), axis(height))
        }
        Element::FillMax {
            width,
            height,
            fraction: value,
        } => {
            let name = match (width, height) {
                (true, true) => "fillMaxSize",
                (true, false) => "fillMaxWidth",
                (false, true) => "fillMaxHeight",
                (false, false) => "fillMaxNone",
            };
            write!(out, "{}{}", name, fraction(value))
        }
        Element::Weight { weight, fill } => {
            let fill = if fill { "" } else { " nofill" };
            write!(out, "weight({}{})", format_double(weight), fill)
        }
        Element::Align(alignment) => write!(out, "align({})", alignment),
        Element::Background { color, shape } => {
            let _ = write!(out, "background({}", color);
            write_shape(out, shape);
            write!(out, ")")
        }
        Element::Border {
            width,
            color,
            shape,
        } => {
            let _ = write!(out, "border({} {}", width, color);
            write_shape(out, shape);
            write!(out, ")")
        }
        Element::Clickable { enabled, role, .. } => {
            let mut arguments: Vec<&str> = Vec::new();
            arguments.extend(role.map(|role| role.name()));
            if !enabled {
                arguments.push("disabled");
            }
            if arguments.is_empty() {
                write!(out, "clickable")
            } else {
                write!(out, "clickable({})", arguments.join(" "))
            }
        }
        Element::Toggleable {
            value,
            enabled,
            role,
            ..
        } => {
            let mut arguments: Vec<&str> = Vec::new();
            arguments.extend(role.map(|role| role.name()));
            arguments.push(if value { "on" } else { "off" });
            if !enabled {
                arguments.push("disabled");
            }
            write!(out, "toggleable({})", arguments.join(" "))
        }
        Element::TextFieldFrame { unfocused, focused } => {
            write!(out, "textFieldFrame({} {})", unfocused, focused)
        }
        Element::Offset { x, y } => write!(out, "offset({},{})", x, y),
    };
}

/// Appends the dump of the display list.
pub fn dump_display_list(list: &DisplayList, out: &mut String) {
    let radius = |out: &mut String, radius: i64| {
        if radius > 0 {
            let _ = write!(out, " radius {}", radius);
        }
    };
    for command in &list.commands {
        match command {
            DrawCommand::Clear {
                width,
                height,
                color,
            } => {
                let _ = write!(out, "clear {}x{} {}", width, height, color);
            }
            DrawCommand::Rectangle {
                bounds,
                color,
                corner_radius,
            } => {
                let _ = write!(
                    out,
                    "rect {},{} {}x{} {}",
                    bounds.x, bounds.y, bounds.width, bounds.height, color
                );
                radius(out, *corner_radius);
            }
            DrawCommand::Border {
                bounds,
                width,
                color,
                corner_radius,
            } => {
                let _ = write!(
                    out,
                    "border {},{} {}x{} width {} {}",
                    bounds.x, bounds.y, bounds.width, bounds.height, width, color
                );
                radius(out, *corner_radius);
            }
            DrawCommand::Text {
                x,
                y,
                font_size,
                color,
                text,
            } => {
                let _ = write!(out, "text {},{} size {} {} ", x, y, font_size, color);
                write_quoted(out, list.text_of(text));
            }
        }
        out.push('\n');
    }
}

/// The absolute bounds of a node, as `dump` prints them, for tests.
pub fn bounds_text(tree: &UiTree, id: NodeId) -> String {
    let bounds = absolute_bounds(tree, id);
    format!(
        "{},{} {}x{}",
        bounds.x, bounds.y, bounds.width, bounds.height
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::HandlerKind;
    use crate::properties::{Alignment, Arrangement, Color, Role};
    use crate::test_support::*;

    fn dumped(builder: &mut TreeBuilder, focused: Option<NodeId>) -> String {
        builder.lay_out();
        let mut out = String::new();
        dump_tree(&builder.tree, focused, &mut out);
        out
    }

    #[test]
    fn a_dump_of_the_counter_screen() {
        let mut b = TreeBuilder::new(400, 300);
        counter_screen(&mut b, "Count: 0", inline_handler(1));
        assert_eq!(
            dumped(&mut b, None),
            "window 400x300\n\
             \x20 column #1 0,0 96x100 arrangement spacedBy(8) padding(16)\n\
             \x20   text #2 16,16 64x20 \"Count: 0\"\n\
             \x20   box #3 16,44 58x40 contentColor #FFFFFFFF background(#FF6750A4 RoundedCorner(50%)) clickable(Button)\n\
             \x20     row #4 16,44 58x40 arrangement Center alignment CenterVertically defaultMinSize(58x40) padding(24,8,24,8)\n\
             \x20       text #5 41,54 8x20 \"+\"\n"
        );
    }

    #[test]
    fn a_checkbox_and_its_label() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let row = b.add(root, NodeKind::Row);
        let checkbox = b.add(row, NodeKind::Box);
        let id = b.register(HandlerKind::Toggle, bool_handler("toggle", |_| {}));
        let primary = 0xFF67_50A4;
        b.chain(
            checkbox,
            &[
                toggleable(id, true, true, Some(Role::Checkbox)),
                size(20, 20),
                background(primary, crate::properties::Shape::new(2, 0)),
                border(2, primary, crate::properties::Shape::new(2, 0)),
            ],
        );
        b.text(row, "milk");
        assert_eq!(
            dumped(&mut b, None),
            "window 400x300\n\
             \x20 row #1 0,0 52x20\n\
             \x20   box #2 0,0 20x20 toggleable(Checkbox on) size(20x20) background(#FF6750A4 RoundedCorner(2)) border(2 #FF6750A4 RoundedCorner(2))\n\
             \x20   text #3 20,0 32x20 \"milk\"\n"
        );
    }

    #[test]
    fn a_focused_text_field_and_a_text_below() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        let field = b.text_field(column, "ac");
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
        b.text(column, "Hello ac");
        assert_eq!(
            dumped(&mut b, Some(field)),
            "window 400x300\n\
             \x20 column #1 0,0 200x56\n\
             \x20   textfield #2 0,0 200x36 \"ac\" focused defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)\n\
             \x20   text #3 0,36 64x20 \"Hello ac\"\n"
        );
    }

    #[test]
    fn every_property_and_element_argument() {
        let mut b = TreeBuilder::new(800, 600);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        b.arrangement(column, Arrangement::SpaceEvenly)
            .alignment(column, Alignment::CENTER_HORIZONTALLY);
        let text = b.text(column, "say \"hi\"\\\n");
        b.style(text, 0xFF00_00FF, 24)
            .content_color(text, 0xFFFF_FFFF)
            .enabled(text, false);
        let click = b.register(HandlerKind::Click, inline_handler(1));
        let toggle = b.register(HandlerKind::Toggle, bool_handler("toggle", |_| {}));
        b.chain(
            text,
            &[
                padding(1),
                padding_each(1, 2, 3, 4),
                width(5),
                height(6),
                size(7, 8),
                default_min_size(-1, 9),
                fill_max(true, false, 1.0),
                fill_max(false, true, 0.5),
                fill_max(true, true, 0.25),
                weight(1.0, true),
                weight(2.5, false),
                align(Alignment::BOTTOM_END),
                background(0xFFFF_0000, crate::properties::Shape::new(4, 0)),
                border(1, 0xFF00_0000, crate::properties::Shape::RECTANGLE),
                clickable(click, false, None),
                toggleable(toggle, false, false, None),
            ],
        );
        let out = dumped(&mut b, None);
        let line = out.lines().nth(2).unwrap();
        assert_eq!(
            line.split_once('"').unwrap().1,
            "say \\\"hi\\\"\\\\\\n\" color #FF0000FF fontSize 24 contentColor #FFFFFFFF disabled \
             padding(1) padding(1,2,3,4) width(5) height(6) size(7x8) defaultMinSize(-x9) \
             fillMaxWidth fillMaxHeight(0.5) fillMaxSize(0.25) weight(1.0) weight(2.5 nofill) \
             align(BottomEnd) background(#FFFF0000 RoundedCorner(4)) border(1 #FF000000) \
             clickable(disabled) toggleable(off disabled)"
        );
        assert!(out
            .lines()
            .nth(1)
            .unwrap()
            .ends_with("arrangement SpaceEvenly alignment CenterHorizontally"));
    }

    #[test]
    fn the_applier_dumps_the_tree() {
        use prism_compose::Applier;
        crate::thread::install_tree(200, 100);
        let mut applier = crate::applier::UiApplier;
        let text = applier.create(4);
        let root = applier.root();
        applier.place(root, None, text);
        crate::thread::with_state(|state| {
            state.tree.set_text(NodeId(text.0), "hi");
        });
        let mut out = String::new();
        applier.dump(&mut out);
        assert_eq!(out, "window 200x100\n  text #1 0,0 0x0 \"hi\"\n");
        assert_eq!(applier.number(text), 1);
    }
}
