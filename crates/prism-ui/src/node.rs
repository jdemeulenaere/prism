//! Node records (D10 of `ui-toolkit`): a node's kind, links, properties, decoded modifier
//! chain with the handler entries it holds, and layout state.

use crate::geometry::{Constraints, Rect, Size};
use crate::handle::{HandlerId, NodeId};
use crate::modifier::Element;
use crate::properties::{Alignment, Arrangement, Color, TextStyle};
use prism_runtime::PrismString;

/// The kinds of nodes, by the codes of `/prism/compose/ui` (D3). The window root, 0, is no
/// composable's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Window = 0,
    Column = 1,
    Row = 2,
    Box = 3,
    Text = 4,
    Spacer = 5,
    TextField = 6,
    /// A user layout of `Layout` (D10 of `layout-primitive`).
    Layout = 7,
}

impl NodeKind {
    /// The kind a composable creates with `code`; the window's code is not one.
    pub fn from_code(code: i64) -> Option<NodeKind> {
        Some(match code {
            1 => NodeKind::Column,
            2 => NodeKind::Row,
            3 => NodeKind::Box,
            4 => NodeKind::Text,
            5 => NodeKind::Spacer,
            6 => NodeKind::TextField,
            7 => NodeKind::Layout,
            _ => return None,
        })
    }

    /// The name dumps print.
    pub fn name(self) -> &'static str {
        match self {
            NodeKind::Window => "window",
            NodeKind::Column => "column",
            NodeKind::Row => "row",
            NodeKind::Box => "box",
            NodeKind::Text => "text",
            NodeKind::Spacer => "spacer",
            NodeKind::TextField => "textfield",
            NodeKind::Layout => "layout",
        }
    }

    /// Whether a node of this kind has its content measured by its measure policy: columns,
    /// rows, boxes, spacers and user layouts (D10 of `layout-primitive`). Texts, text fields
    /// and the window are measured natively.
    pub fn has_policy(self) -> bool {
        matches!(
            self,
            NodeKind::Column | NodeKind::Row | NodeKind::Box | NodeKind::Spacer | NodeKind::Layout
        )
    }

    /// The arrangement a node of this kind has until its composable sets one.
    pub fn default_arrangement(self) -> Arrangement {
        match self {
            NodeKind::Row => Arrangement::Start,
            _ => Arrangement::Top,
        }
    }

    /// The alignment a node of this kind has until its composable sets one.
    pub fn default_alignment(self) -> Alignment {
        match self {
            NodeKind::Column => Alignment::START,
            NodeKind::Row => Alignment::TOP,
            _ => Alignment::TOP_START,
        }
    }
}

/// A string a node owns, allocated by the runtime's allocator so the allocation check covers
/// it, and freed when it is replaced or the node is released.
#[derive(Debug)]
pub struct OwnedText(PrismString);

impl OwnedText {
    pub fn empty() -> OwnedText {
        OwnedText(PrismString::empty())
    }

    pub fn new(text: &str) -> OwnedText {
        OwnedText(PrismString::owned(text))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// The string, which the caller owns from then on.
    pub fn into_string(self) -> PrismString {
        let text = std::mem::ManuallyDrop::new(self);
        // SAFETY: the value is not dropped, so the string is moved out exactly once.
        unsafe { std::ptr::read(&text.0) }
    }
}

impl Drop for OwnedText {
    fn drop(&mut self) {
        // SAFETY: the string is owned by this value and not used again.
        unsafe { prism_runtime::prism_rt_string_drop(&mut self.0) }
    }
}

/// One element of a node's chain, with the bounds at its position, relative to the node,
/// which layout records for drawing and input elements (D11). A layout element records no
/// bounds, so it keeps there what its measurement step last gave instead (D1 and D6 of
/// `layout-in-prism`): where it placed the rest of the chain, from its own top-left corner, as
/// `x` and `y`, and the size of the chain from it as `width` and `height`.
#[derive(Debug, Clone, Copy)]
pub struct ChainEntry {
    pub element: Element,
    pub bounds: Rect,
}

impl ChainEntry {
    pub fn new(element: Element) -> ChainEntry {
        ChainEntry {
            element,
            bounds: Rect::ZERO,
        }
    }
}

/// A node of the tree.
#[derive(Debug)]
pub struct Node {
    pub kind: NodeKind,
    /// Its creation number, from 1 per app, never reused.
    pub number: u64,
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub last_child: Option<NodeId>,
    pub previous: Option<NodeId>,
    pub next: Option<NodeId>,

    /// A text's text or a text field's value.
    pub text: OwnedText,
    pub style: TextStyle,
    pub arrangement: Arrangement,
    pub alignment: Alignment,
    pub enabled: bool,
    pub content_color: Color,
    /// The chain, each handler element naming the entry this node holds.
    pub chain: Vec<ChainEntry>,
    /// A text field's value-change handler.
    pub value_change: Option<HandlerId>,
    /// The measure policy of a node of a policy kind (D8 of `layout-primitive`).
    pub policy: Option<HandlerId>,
    /// Its first `weight` (0.0 for none) with its `fill`, and its first `align`, as its
    /// parent's policy reads them: [`Node::weight`] and [`Node::own_alignment`], kept when the
    /// chain is set so that a policy's read is one load.
    pub parent_data: (f64, bool, Option<Alignment>),

    /// The constraints of the last measurement, if any.
    pub last_constraints: Option<Constraints>,
    pub size: Size,
    /// The position relative to the parent's origin.
    pub x: i64,
    pub y: i64,
    /// Where the content (text or children) starts, relative to the node, and its size.
    pub content: Rect,
    /// The maximum width a text was broken at.
    pub break_width: i64,
    pub needs_measure: bool,
    pub child_needs_measure: bool,
    /// Whether its parent placed it in its last measurement: an unplaced node and its subtree
    /// are not drawn, hit or found by script targets (D7 of `layout-primitive`).
    pub placed: bool,
    /// The stamp of the measurement frame that last measured it, and of the one that last
    /// placed it (D6).
    pub stamp: u64,
    pub placed_stamp: u64,
}

impl Node {
    pub fn new(kind: NodeKind, number: u64) -> Node {
        Node {
            kind,
            number,
            parent: None,
            first_child: None,
            last_child: None,
            previous: None,
            next: None,
            text: OwnedText::empty(),
            style: TextStyle::DEFAULT,
            arrangement: kind.default_arrangement(),
            alignment: kind.default_alignment(),
            enabled: true,
            content_color: Color::UNSPECIFIED,
            chain: Vec::new(),
            value_change: None,
            policy: None,
            parent_data: (0.0, false, None),
            last_constraints: None,
            size: Size::ZERO,
            x: 0,
            y: 0,
            content: Rect::ZERO,
            break_width: 0,
            needs_measure: true,
            child_needs_measure: false,
            placed: true,
            stamp: 0,
            placed_stamp: 0,
        }
    }

    /// Every handler entry the node holds, into `out`.
    pub fn held_handlers(&self, out: &mut impl FnMut(HandlerId)) {
        for entry in &self.chain {
            if let Some(handler) = entry.element.handler() {
                out(handler);
            }
        }
        if let Some(handler) = self.value_change {
            out(handler);
        }
        if let Some(handler) = self.policy {
            out(handler);
        }
    }

    /// The node's weight in a row or column: the first `weight` of its chain.
    pub fn weight(&self) -> Option<(f64, bool)> {
        self.chain.iter().find_map(|entry| match entry.element {
            Element::Weight { weight, fill } => Some((weight, fill)),
            _ => None,
        })
    }

    /// The parent data its chain gives, to keep in [`Node::parent_data`].
    pub fn chain_parent_data(&self) -> (f64, bool, Option<Alignment>) {
        let (weight, fill) = self.weight().unwrap_or((0.0, false));
        (weight, fill, self.own_alignment())
    }

    /// The node's own alignment in its parent: the first `align` of its chain.
    pub fn own_alignment(&self) -> Option<Alignment> {
        self.chain.iter().find_map(|entry| match entry.element {
            Element::Align(alignment) => Some(alignment),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_decode_from_their_codes() {
        let names: Vec<_> = (1..=7)
            .map(|code| NodeKind::from_code(code).unwrap().name())
            .collect();
        assert_eq!(
            names,
            [
                "column",
                "row",
                "box",
                "text",
                "spacer",
                "textfield",
                "layout"
            ]
        );
        assert_eq!(
            NodeKind::from_code(0),
            None,
            "the window is no composable's"
        );
        assert_eq!(NodeKind::from_code(8), None);
        assert_eq!(NodeKind::from_code(-1), None);
    }

    #[test]
    fn defaults_follow_the_kind() {
        let row = Node::new(NodeKind::Row, 1);
        assert_eq!(row.arrangement, Arrangement::Start);
        assert_eq!(row.alignment, Alignment::TOP);
        let column = Node::new(NodeKind::Column, 2);
        assert_eq!(column.arrangement, Arrangement::Top);
        assert_eq!(column.alignment, Alignment::START);
        assert!(column.needs_measure);
        assert_eq!(column.text.as_str(), "");
    }

    #[test]
    fn parent_data_is_the_first_in_the_chain() {
        let mut node = Node::new(NodeKind::Text, 1);
        for element in [
            Element::Weight {
                weight: 2.0,
                fill: false,
            },
            Element::Align(Alignment::END),
            Element::Weight {
                weight: 1.0,
                fill: true,
            },
            Element::Align(Alignment::CENTER),
        ] {
            node.chain.push(ChainEntry::new(element));
        }
        assert_eq!(node.weight(), Some((2.0, false)));
        assert_eq!(node.own_alignment(), Some(Alignment::END));
    }
}
