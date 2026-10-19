//! The headless host (D16 of `ui-toolkit`): it gives the frame loop an event script's lines in
//! order, presents nothing, and ends the app after the last line.
//!
//! A text target `"<text>" [<n>]` names the n-th node in tree order, from 1, whose text is
//! exactly the string: a text's text or a text field's value. The frame loop resolves it
//! against the tree at the time of the command ([`resolve_text_target`]), at the centre of the
//! node's bounds, rounded down, and hit-tests normally from there, so `click "+"` clicks the
//! button around the `+`. A target no node shows and a `type` or `key` without a focused text
//! field end the program with `event script line <n>: <detail>` ([`missing_target`],
//! [`no_focus`]). Dumps are printed through the program's output, in order with what it
//! prints.

use crate::draw::DisplayList;
use crate::dump::write_quoted;
use crate::error::UiError;
use crate::host::{Host, HostEvent};
use crate::layout::absolute_bounds;
use crate::node::NodeKind;
use crate::script::ScriptLine;
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

/// The point a text target acts at: the centre, rounded down, of the `occurrence`-th node in
/// tree order whose text or value is `text`.
pub fn resolve_text_target(tree: &UiTree, text: &str, occurrence: usize) -> Option<(i64, i64)> {
    let mut seen = 0;
    let mut stack = Vec::new();
    let mut child = tree.node(tree.root())?.last_child;
    while let Some(current) = child {
        stack.push(current);
        child = tree.node(current).unwrap().previous;
    }
    while let Some(id) = stack.pop() {
        let node = tree.node(id).unwrap();
        if matches!(node.kind, NodeKind::Text | NodeKind::TextField) && node.text.as_str() == text {
            seen += 1;
            if seen == occurrence {
                return Some(absolute_bounds(tree, id).centre());
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
    use crate::layout::Layout;
    use crate::script::parse_script;
    use crate::test_support::*;
    use crate::text::FixedAdvanceMetrics;

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
        Layout::new().run(&mut b.tree, &FixedAdvanceMetrics);
        assert_eq!(resolve_text_target(&b.tree, "item", 1), Some((16, 10)));
        assert_eq!(resolve_text_target(&b.tree, "item", 2), Some((16, 50)));
        assert_eq!(resolve_text_target(&b.tree, "item", 3), None);
        // The odd box is at 0,60 9x9: its centre rounds down to 4,64.
        assert_eq!(resolve_text_target(&b.tree, "odd", 1), Some((4, 64)));
        assert_eq!(resolve_text_target(&b.tree, "Minus", 1), None);
    }

    #[test]
    fn the_messages_of_both_errors() {
        assert_eq!(
            missing_target(1, "Minus").message(),
            "event script line 1: no node shows the text \"Minus\""
        );
        assert_eq!(
            no_focus(3).message(),
            "event script line 3: no text field is focused"
        );
    }
}
