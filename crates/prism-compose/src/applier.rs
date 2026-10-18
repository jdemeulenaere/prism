//! Nodes and the applier that owns them (D9 of `compose-runtime`, implementing
//! `compose-architecture` D7).
//!
//! A composition has an [`Applier`], which creates, places, removes and releases the nodes of
//! its node groups. Placement is relative to the previous sibling rather than by index, because
//! removals are deferred to the end of the pass, so during a pass a parent's children still
//! include nodes about to be removed and indexes would be ambiguous. A `place` that changes
//! nothing must be cheap, since every node group places its node on every pass that composes
//! it.
//!
//! [`NodeTree`] is the built-in applier: it keeps each node's kind and children and nothing
//! else, and numbers nodes from 1 in creation order. Compositions started through the entry
//! points use it; `prism-ui` supplies its own applier. An applier's methods are Rust code and
//! must not call back into the composition runtime.

use crate::handle::{Arena, Handle};

/// A node of an applier, named by a handle the applier gives out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeHandle(pub u64);

/// What owns a composition's nodes.
pub trait Applier {
    /// The root node, the parent of the nodes of top-level node groups.
    fn root(&self) -> NodeHandle;
    /// Creates a node of `kind`, not yet placed.
    fn create(&mut self, kind: i64) -> NodeHandle;
    /// Puts `node` among `parent`'s children right after `previous` (first when `None`),
    /// inserting it or moving it there from wherever it is.
    fn place(&mut self, parent: NodeHandle, previous: Option<NodeHandle>, node: NodeHandle);
    /// Takes `node` out of `parent`'s children, at the end of a pass.
    fn remove(&mut self, parent: NodeHandle, node: NodeHandle);
    /// Releases `node`, after its descendants.
    fn release(&mut self, node: NodeHandle);
    /// The number a composition dump shows for `node`.
    fn number(&self, node: NodeHandle) -> u64;
    /// Appends the applier's own dump.
    fn dump(&self, out: &mut String);
}

#[derive(Debug)]
struct TreeNode {
    kind: i64,
    number: u64,
    parent: Option<Handle>,
    first_child: Option<Handle>,
    last_child: Option<Handle>,
    previous: Option<Handle>,
    next: Option<Handle>,
}

/// The built-in applier: an arena of linked nodes under a root.
#[derive(Debug)]
pub struct NodeTree {
    nodes: Arena<TreeNode>,
    root: Handle,
    numbers: u64,
}

impl Default for NodeTree {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeTree {
    pub fn new() -> NodeTree {
        let mut nodes = Arena::new();
        let root = nodes.insert(TreeNode {
            kind: 0,
            number: 0,
            parent: None,
            first_child: None,
            last_child: None,
            previous: None,
            next: None,
        });
        NodeTree {
            nodes,
            root,
            numbers: 0,
        }
    }

    /// How many nodes are live, the root not counted.
    pub fn len(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The children of `node`, in order.
    pub fn children(&self, node: NodeHandle) -> Vec<NodeHandle> {
        let mut children = Vec::new();
        let mut child = self
            .nodes
            .get(Handle(node.0))
            .and_then(|node| node.first_child);
        while let Some(handle) = child {
            children.push(NodeHandle(handle.0));
            child = self.nodes.get(handle).and_then(|node| node.next);
        }
        children
    }

    /// The kind of `node`.
    pub fn kind(&self, node: NodeHandle) -> Option<i64> {
        self.nodes.get(Handle(node.0)).map(|node| node.kind)
    }

    /// Takes `node` out of its parent's list, if it is in one.
    fn unlink(&mut self, node: Handle) {
        let Some(record) = self.nodes.get(node) else {
            return;
        };
        let (parent, previous, next) = (record.parent, record.previous, record.next);
        let Some(parent) = parent else {
            return;
        };
        match previous {
            Some(previous) => self.nodes.get_mut(previous).unwrap().next = next,
            None => self.nodes.get_mut(parent).unwrap().first_child = next,
        }
        match next {
            Some(next) => self.nodes.get_mut(next).unwrap().previous = previous,
            None => self.nodes.get_mut(parent).unwrap().last_child = previous,
        }
        let record = self.nodes.get_mut(node).unwrap();
        record.parent = None;
        record.previous = None;
        record.next = None;
    }
}

impl Applier for NodeTree {
    fn root(&self) -> NodeHandle {
        NodeHandle(self.root.0)
    }

    fn create(&mut self, kind: i64) -> NodeHandle {
        self.numbers += 1;
        NodeHandle(
            self.nodes
                .insert(TreeNode {
                    kind,
                    number: self.numbers,
                    parent: None,
                    first_child: None,
                    last_child: None,
                    previous: None,
                    next: None,
                })
                .0,
        )
    }

    fn place(&mut self, parent: NodeHandle, previous: Option<NodeHandle>, node: NodeHandle) {
        let (parent, node) = (Handle(parent.0), Handle(node.0));
        let previous = previous.map(|previous| Handle(previous.0));
        let Some(record) = self.nodes.get(node) else {
            return;
        };
        if !self.nodes.contains(parent) || previous == Some(node) {
            return;
        }
        if let Some(previous) = previous {
            if self.nodes.get(previous).and_then(|p| p.parent) != Some(parent) {
                return;
            }
        }
        if record.parent == Some(parent) && record.previous == previous {
            return;
        }
        self.unlink(node);
        let next = match previous {
            Some(previous) => self.nodes.get(previous).unwrap().next,
            None => self.nodes.get(parent).unwrap().first_child,
        };
        {
            let record = self.nodes.get_mut(node).unwrap();
            record.parent = Some(parent);
            record.previous = previous;
            record.next = next;
        }
        match previous {
            Some(previous) => self.nodes.get_mut(previous).unwrap().next = Some(node),
            None => self.nodes.get_mut(parent).unwrap().first_child = Some(node),
        }
        match next {
            Some(next) => self.nodes.get_mut(next).unwrap().previous = Some(node),
            None => self.nodes.get_mut(parent).unwrap().last_child = Some(node),
        }
    }

    fn remove(&mut self, parent: NodeHandle, node: NodeHandle) {
        let node = Handle(node.0);
        if self.nodes.get(node).and_then(|record| record.parent) == Some(Handle(parent.0)) {
            self.unlink(node);
        }
    }

    fn release(&mut self, node: NodeHandle) {
        let node = Handle(node.0);
        if node == self.root {
            return;
        }
        self.unlink(node);
        // Children still linked to it, which only a misbehaving caller leaves, become roots of
        // their own.
        let mut child = self.nodes.get(node).and_then(|record| record.first_child);
        while let Some(handle) = child {
            let record = self.nodes.get_mut(handle).unwrap();
            child = record.next;
            record.parent = None;
            record.previous = None;
            record.next = None;
        }
        self.nodes.remove(node);
    }

    fn number(&self, node: NodeHandle) -> u64 {
        self.nodes
            .get(Handle(node.0))
            .map_or(0, |record| record.number)
    }

    fn dump(&self, out: &mut String) {
        use std::fmt::Write;
        out.push_str("nodes\n");
        // Depth-first, without recursion: (node, depth).
        let mut stack: Vec<(Handle, usize)> = Vec::new();
        let push_children = |stack: &mut Vec<(Handle, usize)>, node: Handle, depth: usize| {
            let start = stack.len();
            let mut child = self.nodes.get(node).and_then(|record| record.first_child);
            while let Some(handle) = child {
                stack.push((handle, depth));
                child = self.nodes.get(handle).and_then(|record| record.next);
            }
            stack[start..].reverse();
        };
        push_children(&mut stack, self.root, 1);
        while let Some((node, depth)) = stack.pop() {
            let record = self.nodes.get(node).unwrap();
            let _ = writeln!(
                out,
                "{}#{} kind {}",
                "  ".repeat(depth),
                record.number,
                record.kind
            );
            push_children(&mut stack, node, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(tree: &NodeTree, parent: NodeHandle) -> Vec<i64> {
        tree.children(parent)
            .into_iter()
            .map(|node| tree.kind(node).unwrap())
            .collect()
    }

    #[test]
    fn place_inserts_moves_and_keeps() {
        let mut tree = NodeTree::new();
        let root = tree.root();
        let a = tree.create(1);
        let b = tree.create(2);
        let c = tree.create(3);
        tree.place(root, None, a);
        tree.place(root, Some(a), b);
        tree.place(root, Some(b), c);
        assert_eq!(kinds(&tree, root), [1, 2, 3]);
        // Moving c first.
        tree.place(root, None, c);
        assert_eq!(kinds(&tree, root), [3, 1, 2]);
        // A place that changes nothing.
        tree.place(root, Some(c), a);
        assert_eq!(kinds(&tree, root), [3, 1, 2]);
        // Moving into another parent.
        tree.place(a, None, b);
        assert_eq!(kinds(&tree, root), [3, 1]);
        assert_eq!(kinds(&tree, a), [2]);
    }

    #[test]
    fn remove_and_release() {
        let mut tree = NodeTree::new();
        let root = tree.root();
        let a = tree.create(1);
        let b = tree.create(2);
        tree.place(root, None, a);
        tree.place(a, None, b);
        tree.release(b);
        tree.remove(root, a);
        tree.release(a);
        assert!(tree.is_empty());
        assert!(tree.children(root).is_empty());
        // Numbers are never reused.
        let c = tree.create(3);
        assert_eq!(tree.number(c), 3);
    }

    #[test]
    fn the_dump_indents_nested_nodes() {
        let mut tree = NodeTree::new();
        let root = tree.root();
        let a = tree.create(1);
        let b = tree.create(2);
        let c = tree.create(3);
        tree.place(root, None, a);
        tree.place(a, None, b);
        tree.place(a, Some(b), c);
        let mut out = String::new();
        tree.dump(&mut out);
        assert_eq!(out, "nodes\n  #1 kind 1\n    #2 kind 2\n    #3 kind 3\n");
    }
}
