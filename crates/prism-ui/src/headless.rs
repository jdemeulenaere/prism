//! The headless host (D16 of `ui-toolkit`): it gives the frame loop an event script's lines in
//! order, presents nothing, and ends the app after the last line.
//!
//! A text target `"<text>" [<n>]` names the n-th node in tree order, from 1, whose text is
//! exactly the string: a text's text or a text field's value. The frame loop resolves it
//! against the tree at the time of the command ([`resolve_text_target`]), at the centre of the
//! node's content, where its steps placed it, rounded down, and hit-tests normally from there, so `click "+"` clicks the
//! button around the `+`. A target no node shows and a `type` or `key` without a focused text
//! field end the program with `event script line <n>: <detail>` ([`missing_target`],
//! [`no_focus`]). Dumps are printed through the program's output, in order with what it
//! prints.

use crate::draw::DisplayList;
use crate::dump::write_quoted;
use crate::error::UiError;
use crate::geometry::Rect;
use crate::host::{Host, HostEvent};
use crate::layout::absolute_bounds;
use crate::modifier::Element;
use crate::node::{Node, NodeKind};
use crate::script::{ScriptLine, TargetKind};
use crate::tree::UiTree;

/// A host that runs an event script.
#[derive(Debug)]
pub struct HeadlessHost {
    size: (i64, i64),
    lines: std::vec::IntoIter<ScriptLine>,
}

impl HeadlessHost {
    pub fn new(size: (i64, i64), lines: Vec<ScriptLine>) -> HeadlessHost {
        HeadlessHost {
            size,
            lines: lines.into_iter(),
        }
    }
}

impl Host for HeadlessHost {
    fn size(&self) -> (i64, i64) {
        self.size
    }

    fn next_event(&mut self) -> Option<HostEvent> {
        self.lines.next().map(HostEvent::Script)
    }

    fn present(&mut self, _list: &DisplayList) {}
}

/// The point a text target acts at: the centre, rounded down, of the content of the
/// `occurrence`-th node in tree order whose text or value is `text`, where it is drawn.
pub fn resolve_text_target(tree: &UiTree, text: &str, occurrence: usize) -> Option<(i64, i64)> {
    resolve(tree, occurrence, |node| {
        (matches!(node.kind, NodeKind::Text | NodeKind::TextField) && node.text.as_str() == text)
            .then_some(node.content)
    })
}

/// The point a kind target acts at: the centre, rounded down, of the `occurrence`-th node in
/// tree order of `kind`, or whose `clickable` or `toggleable` element has its role.
pub fn resolve_kind_target(
    tree: &UiTree,
    kind: TargetKind,
    occurrence: usize,
) -> Option<(i64, i64)> {
    resolve(tree, occurrence, |node| match kind {
        TargetKind::Node(node_kind) => (node.kind == node_kind).then_some(node.content),
        // Where the element with the role is hit.
        TargetKind::Role(role) => node.chain.iter().find_map(|entry| {
            matches!(
                entry.element,
                Element::Clickable { role: Some(r), .. } | Element::Toggleable { role: Some(r), .. }
                    if r == role
            )
            .then_some(entry.bounds)
        }),
    })
}

/// The centre, rounded down, of the bounds that `matches` gives, relative to its node, for the
/// `occurrence`-th node in tree order it gives bounds for: where the node is drawn or hit,
/// which its steps may have moved from the node's own bounds (an `offset`, for instance).
fn resolve(
    tree: &UiTree,
    occurrence: usize,
    matches: impl Fn(&Node) -> Option<Rect>,
) -> Option<(i64, i64)> {
    let mut seen = 0;
    let mut stack = Vec::new();
    let mut child = tree.node(tree.root())?.last_child;
    while let Some(current) = child {
        stack.push(current);
        child = tree.node(current).unwrap().previous;
    }
    while let Some(id) = stack.pop() {
        let node = tree.node(id).unwrap();
        // An unplaced node and its subtree are no target (D7 of `layout-primitive`).
        if !node.placed {
            continue;
        }
        if let Some(bounds) = matches(node) {
            seen += 1;
            if seen == occurrence {
                let node_bounds = absolute_bounds(tree, id);
                return Some(bounds.offset(node_bounds.x, node_bounds.y).centre());
            }
        }
        let mut child = node.last_child;
        while let Some(current) = child {
            stack.push(current);
            child = tree.node(current).unwrap().previous;
        }
    }
    None
}

/// `event script line <line>: the tree has no <kind> <occurrence>`.
pub fn missing_kind(line: usize, kind: TargetKind, occurrence: usize) -> UiError {
    UiError::Script {
        line,
        detail: format!("the tree has no {} {}", kind.word(), occurrence),
    }
}

/// `event script line <line>: no node shows the text "<text>"`.
pub fn missing_target(line: usize, text: &str) -> UiError {
    let mut detail = String::from("no node shows the text ");
    write_quoted(&mut detail, text);
    UiError::Script { line, detail }
}

/// `event script line <line>: no text field is focused`.
pub fn no_focus(line: usize) -> UiError {
    UiError::Script {
        line,
        detail: "no text field is focused".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::parse_script;
    use crate::test_support::*;

    #[test]
    fn the_script_is_given_in_order_and_then_ends() {
        let mut host = HeadlessHost::new((400, 300), parse_script("frame\n\ndump").unwrap());
        assert_eq!(host.size(), (400, 300));
        let lines: Vec<usize> = std::iter::from_fn(|| host.next_event())
            .map(|event| match event {
                HostEvent::Script(line) => line.line,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(lines, [1, 3]);
        assert!(host.next_event().is_none());
    }

    #[test]
    fn targets_by_occurrence_at_the_centre_rounded_down() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let column = b.add(root, NodeKind::Column);
        b.text(column, "item");
        b.text(column, "other");
        b.text_field(column, "item");
        let odd = b.text(column, "odd");
        b.chain(odd, &[size(9, 9)]);
        b.lay_out();
        assert_eq!(resolve_text_target(&b.tree, "item", 1), Some((16, 10)));
        assert_eq!(resolve_text_target(&b.tree, "item", 2), Some((16, 50)));
        assert_eq!(resolve_text_target(&b.tree, "item", 3), None);
        // The odd box is at 0,60 9x9: its centre rounds down to 4,64.
        assert_eq!(resolve_text_target(&b.tree, "odd", 1), Some((4, 64)));
        assert_eq!(resolve_text_target(&b.tree, "Minus", 1), None);
        let field = TargetKind::Node(NodeKind::TextField);
        assert_eq!(resolve_kind_target(&b.tree, field, 1), Some((16, 50)));
        assert_eq!(resolve_kind_target(&b.tree, field, 2), None);
        let text = TargetKind::Node(NodeKind::Text);
        assert_eq!(resolve_kind_target(&b.tree, text, 3), Some((4, 64)));
    }

    #[test]
    fn the_messages_of_both_errors() {
        assert_eq!(
            missing_target(1, "Minus").message(),
            "event script line 1: no node shows the text \"Minus\""
        );
        assert_eq!(
            missing_kind(1, TargetKind::from_word("checkbox").unwrap(), 1).message(),
            "event script line 1: the tree has no checkbox 1"
        );
        assert_eq!(
            no_focus(3).message(),
            "event script line 3: no text field is focused"
        );
    }

    /// D7 of `layout-primitive`: a node its parent does not place, and its subtree, are no
    /// text or kind target; `layout` is a kind target.
    #[test]
    fn unplaced_nodes_are_no_targets() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let node = b.add(root, NodeKind::Layout);
        b.set_policy(node, first_only_policy());
        b.text(node, "shown");
        let hidden = b.add(node, NodeKind::Box);
        b.text(hidden, "inside");
        b.lay_out();
        assert_eq!(resolve_text_target(&b.tree, "shown", 1), Some((20, 10)));
        assert_eq!(resolve_text_target(&b.tree, "inside", 1), None);
        let boxes = TargetKind::Node(NodeKind::Box);
        assert_eq!(resolve_kind_target(&b.tree, boxes, 1), None);
        let layouts = TargetKind::from_word("layout").unwrap();
        assert_eq!(resolve_kind_target(&b.tree, layouts, 1), Some((20, 10)));
    }
}
