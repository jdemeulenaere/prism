//! Text dumps (D16 of `ui-toolkit`).

use crate::handle::NodeId;
use crate::tree::UiTree;
use std::fmt::Write;

/// Appends the dump of the tree: `window <width>x<height>`, then a line per node in tree
/// order, indented two spaces per depth.
pub fn dump_tree(tree: &UiTree, _focused: Option<NodeId>, out: &mut String) {
    let size = tree.window_size();
    let _ = writeln!(out, "window {}x{}", size.width, size.height);
    let mut stack: Vec<(NodeId, usize)> = tree.children(tree.root()).map(|c| (c, 1)).collect();
    stack.reverse();
    while let Some((id, depth)) = stack.pop() {
        let node = tree.node(id).unwrap();
        let _ = writeln!(
            out,
            "{}{} #{}",
            "  ".repeat(depth),
            node.kind.name(),
            node.number
        );
        let start = stack.len();
        stack.extend(tree.children(id).map(|c| (c, depth + 1)));
        stack[start..].reverse();
    }
}
