//! The UI tree (D10 and D11 of `ui-toolkit`): a generational arena of linked nodes under a
//! window root, with the property updates that compare and invalidate, and the release of a
//! node's values.
//!
//! - **Placement.** Children are linked lists, so placing a node after a sibling is O(1) and
//!   traversal is in order; a placement that changes nothing does nothing.
//! - **Dirty flags.** A change that affects layout marks its node `needs_measure` and its
//!   ancestors `child_needs_measure`, stopping at the first ancestor already marked; a change
//!   that affects only drawing sets the tree's `redraw` flag, as layout does when it measures.
//! - **Ownership.** A node owns its text, which is freed when replaced or released, and the
//!   handler entries it claimed (D23). Releasing a node moves its handlers' function values to
//!   the caller's `drops`, which the caller drops once it borrows no state (D2), since a drop
//!   entry is program code.

use crate::error::{unknown_kind, UiResult};
use crate::geometry::Size;
use crate::handle::{Arena, HandlerId, NodeId};
use crate::handlers::{HandlerKind, HandlerTable};
use crate::modifier::{compare_elements, ChainChange, Element, CAPACITY};
use crate::node::{ChainEntry, Node, NodeKind, OwnedText};
use crate::properties::{Alignment, Arrangement, Color, TextStyle};
use prism_runtime::FunctionValue;

/// For each element of a chain, the handler it names and the other one its node holds there.
pub type HandlerPairs = [Option<(HandlerId, HandlerId)>; CAPACITY];

/// For each element of a chain, whether the two handlers of its pair are equal.
pub type HandlerEqualities = [bool; CAPACITY];

/// What a property update changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Update {
    pub layout: bool,
    pub drawing: bool,
}

/// The nodes of one app.
#[derive(Debug)]
pub struct UiTree {
    nodes: Arena<Node>,
    root: NodeId,
    numbers: u64,
    /// Whether the display list is stale.
    pub redraw: bool,
    /// Scratch for releasing subtrees.
    release_stack: Vec<NodeId>,
    /// The nodes marked `needs_measure` since the last layout, from which a relayout climbs.
    pub(crate) dirty: Vec<NodeId>,
}

impl UiTree {
    /// A tree whose window root is `width` × `height`.
    pub fn new(width: i64, height: i64) -> UiTree {
        let mut nodes = Arena::new();
        let mut root = Node::new(NodeKind::Window, 0);
        root.size = Size::new(width.max(0), height.max(0));
        let root = NodeId(nodes.insert(root));
        UiTree {
            nodes,
            root,
            numbers: 0,
            redraw: true,
            release_stack: Vec::new(),
            // Room for the first relayouts, so that one after the first frame (a resize marks
            // the root) allocates nothing.
            dirty: Vec::with_capacity(16),
        }
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    /// How many nodes are live, the root not counted.
    pub fn len(&self) -> usize {
        self.nodes.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.0)
    }

    /// The window's size.
    pub fn window_size(&self) -> Size {
        self.node(self.root).unwrap().size
    }

    /// Resizes the window, marking the root for measuring.
    pub fn resize(&mut self, width: i64, height: i64) {
        let size = Size::new(width.max(0), height.max(0));
        let root = self.root;
        if self.node(root).unwrap().size != size {
            self.node_mut(root).unwrap().size = size;
            self.mark_needs_measure(root);
        }
    }

    /// Whether layout has anything to do.
    pub fn needs_layout(&self) -> bool {
        let root = self.node(self.root).unwrap();
        root.needs_measure || root.child_needs_measure
    }

    /// Creates a node of the kind of `code`, not yet placed.
    pub fn create(&mut self, code: i64) -> UiResult<NodeId> {
        let kind = NodeKind::from_code(code).ok_or_else(|| unknown_kind(code))?;
        Ok(self.create_kind(kind))
    }

    pub fn create_kind(&mut self, kind: NodeKind) -> NodeId {
        self.numbers += 1;
        NodeId(self.nodes.insert(Node::new(kind, self.numbers)))
    }

    /// The children of `id`, in order.
    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            tree: self,
            next: self.node(id).and_then(|node| node.first_child),
        }
    }

    /// Marks `id` for measuring and its ancestors as having a child to measure, stopping at
    /// the first ancestor already marked either way, and records it for the next relayout.
    pub fn mark_needs_measure(&mut self, id: NodeId) {
        let Some(node) = self.node_mut(id) else {
            return;
        };
        if !node.needs_measure {
            node.needs_measure = true;
            let parent = node.parent;
            self.dirty.push(id);
            self.mark_ancestors(parent);
        }
    }

    /// Marks the ancestors from `parent` up as having a child to measure, stopping at the
    /// first one already marked either way.
    fn mark_ancestors(&mut self, mut parent: Option<NodeId>) {
        while let Some(ancestor) = parent {
            let node = self.node_mut(ancestor).unwrap();
            if node.needs_measure || node.child_needs_measure {
                break;
            }
            node.child_needs_measure = true;
            parent = node.parent;
        }
    }

    /// Takes `id` out of its parent's children, if it is among them.
    fn unlink(&mut self, id: NodeId) {
        let Some(node) = self.node(id) else {
            return;
        };
        let (parent, previous, next) = (node.parent, node.previous, node.next);
        let Some(parent) = parent else {
            return;
        };
        match previous {
            Some(previous) => self.node_mut(previous).unwrap().next = next,
            None => self.node_mut(parent).unwrap().first_child = next,
        }
        match next {
            Some(next) => self.node_mut(next).unwrap().previous = previous,
            None => self.node_mut(parent).unwrap().last_child = previous,
        }
        let node = self.node_mut(id).unwrap();
        node.parent = None;
        node.previous = None;
        node.next = None;
    }

    /// Puts `id` among `parent`'s children right after `previous` (first when `None`),
    /// inserting it or moving it there. Does nothing when it is already there, or when a
    /// handle is stale or `previous` is not a child of `parent`.
    pub fn place_after(&mut self, parent: NodeId, previous: Option<NodeId>, id: NodeId) {
        let Some(node) = self.node(id) else {
            return;
        };
        if !self.nodes.contains(parent.0) || previous == Some(id) || id == self.root {
            return;
        }
        if let Some(previous) = previous {
            if self.node(previous).and_then(|p| p.parent) != Some(parent) {
                return;
            }
        }
        if node.parent == Some(parent) && node.previous == previous {
            return;
        }
        let old_parent = node.parent;
        self.unlink(id);
        let next = match previous {
            Some(previous) => self.node(previous).unwrap().next,
            None => self.node(parent).unwrap().first_child,
        };
        {
            let node = self.node_mut(id).unwrap();
            node.parent = Some(parent);
            node.previous = previous;
            node.next = next;
        }
        match previous {
            Some(previous) => self.node_mut(previous).unwrap().next = Some(id),
            None => self.node_mut(parent).unwrap().first_child = Some(id),
        }
        match next {
            Some(next) => self.node_mut(next).unwrap().previous = Some(id),
            None => self.node_mut(parent).unwrap().last_child = Some(id),
        }
        if let Some(old_parent) = old_parent.filter(|old| *old != parent) {
            self.mark_needs_measure(old_parent);
        }
        self.mark_needs_measure(parent);
    }

    /// Takes `id` out of `parent`'s children and marks the parent for measuring.
    pub fn remove(&mut self, parent: NodeId, id: NodeId) {
        if self.node(id).and_then(|node| node.parent) == Some(parent) {
            self.unlink(id);
            self.mark_needs_measure(parent);
        }
    }

    /// Releases `id` and every node still linked under it, children first and the last child
    /// first: frees their texts and moves the function values of the handler entries they
    /// hold to `drops`.
    pub fn release(
        &mut self,
        id: NodeId,
        handlers: &mut HandlerTable,
        drops: &mut Vec<FunctionValue>,
    ) {
        if id == self.root || !self.nodes.contains(id.0) {
            return;
        }
        if let Some(parent) = self.node(id).unwrap().parent {
            self.remove(parent, id);
        }
        // A post-order walk without recursion: a node is released once it has no children.
        let mut stack = std::mem::take(&mut self.release_stack);
        stack.push(id);
        while let Some(&top) = stack.last() {
            match self.node(top).unwrap().last_child {
                Some(child) => {
                    self.unlink(child);
                    stack.push(child);
                }
                None => {
                    stack.pop();
                    let node = self.nodes.remove(top.0).unwrap();
                    node.held_handlers(&mut |handler| handlers.release(handler, drops));
                }
            }
        }
        self.release_stack = stack;
    }

    /// Releases every node: what the end of an app does after disposing of its composition,
    /// for nodes a misbehaving composition left.
    pub fn release_all(&mut self, handlers: &mut HandlerTable, drops: &mut Vec<FunctionValue>) {
        while let Some(child) = self.node(self.root).unwrap().first_child {
            self.release(child, handlers, drops);
        }
    }

    fn apply(&mut self, id: NodeId, update: Update) {
        if update.layout {
            self.mark_needs_measure(id);
        }
        if update.drawing {
            self.redraw = true;
        }
    }

    /// Sets a text's text or a text field's value, copying it only when it differs.
    pub fn set_text(&mut self, id: NodeId, text: &str) -> Option<Update> {
        let node = self.node_mut(id)?;
        if node.text.as_str() == text {
            return Some(Update::default());
        }
        node.text = OwnedText::new(text);
        let update = Update {
            layout: true,
            drawing: true,
        };
        self.apply(id, update);
        Some(update)
    }

    pub fn set_text_style(&mut self, id: NodeId, style: TextStyle) -> Option<Update> {
        let node = self.node_mut(id)?;
        let mut update = Update::default();
        if node.style != style {
            update.drawing = true;
            update.layout = node.style.resolved_font_size() != style.resolved_font_size();
            node.style = style;
        }
        self.apply(id, update);
        Some(update)
    }

    pub fn set_arrangement(&mut self, id: NodeId, arrangement: Arrangement) -> Option<Update> {
        let node = self.node_mut(id)?;
        let changed = node.arrangement != arrangement;
        node.arrangement = arrangement;
        let update = Update {
            layout: changed,
            drawing: changed,
        };
        self.apply(id, update);
        Some(update)
    }

    pub fn set_alignment(&mut self, id: NodeId, alignment: Alignment) -> Option<Update> {
        let node = self.node_mut(id)?;
        let changed = node.alignment != alignment;
        node.alignment = alignment;
        let update = Update {
            layout: changed,
            drawing: changed,
        };
        self.apply(id, update);
        Some(update)
    }

    pub fn set_content_color(&mut self, id: NodeId, color: Color) -> Option<Update> {
        let node = self.node_mut(id)?;
        let changed = node.content_color != color;
        node.content_color = color;
        let update = Update {
            layout: false,
            drawing: changed,
        };
        self.apply(id, update);
        Some(update)
    }

    pub fn set_enabled(&mut self, id: NodeId, enabled: bool) -> Option<Update> {
        let node = self.node_mut(id)?;
        let changed = node.enabled != enabled;
        node.enabled = enabled;
        let update = Update {
            layout: false,
            drawing: changed,
        };
        self.apply(id, update);
        Some(update)
    }

    /// For each element of `new` naming a handler, the other entry the node holds at that
    /// position in an element of the same variant: the pairs whose equality
    /// [`UiTree::set_chain`] needs, which the caller compares outside the thread's state,
    /// since an environment equality entry is program code (D2).
    pub fn handler_pairs(&self, id: NodeId, new: &[Element]) -> HandlerPairs {
        let mut pairs = [None; CAPACITY];
        let Some(node) = self.node(id) else {
            return pairs;
        };
        for (index, element) in new.iter().enumerate() {
            let (Some(named), Some(old)) = (element.handler(), node.chain.get(index)) else {
                continue;
            };
            if let Some(current) = old.element.handler() {
                if current != named && old.element.same_variant(element) {
                    pairs[index] = Some((named, current));
                }
            }
        }
        pairs
    }

    /// Sets a node's chain (D6, D23), given for each pair of [`UiTree::handler_pairs`]
    /// whether its two handlers are equal. When the new chain equals the node's, element by
    /// element with handlers compared by equality, the node keeps every entry it holds and the
    /// entries the new chain names stay pending: the composable that passed the chain keeps its
    /// old argument then, so the entries it names are the node's. Otherwise the node takes the
    /// entries the new chain names, claiming those it does not hold, as that composable keeps
    /// the new argument; and it releases those it no longer names, moving their function values
    /// to `drops`.
    pub fn set_chain(
        &mut self,
        id: NodeId,
        new: &[Element],
        equal: &HandlerEqualities,
        handlers: &mut HandlerTable,
        drops: &mut Vec<FunctionValue>,
    ) -> UiResult<Option<ChainChange>> {
        let Some(node) = self.node(id) else {
            return Ok(None);
        };
        let old_parent_data = node.parent_data;
        let old = &node.chain;
        let chain_equal = new.len() == old.len()
            && new.iter().zip(old).enumerate().all(|(index, (new, old))| {
                let old = &old.element;
                old.same_variant(new)
                    && old.with_handler(HandlerId(0)) == new.with_handler(HandlerId(0))
                    && match (new.handler(), old.handler()) {
                        (Some(named), Some(current)) => named == current || equal[index],
                        _ => true,
                    }
            });
        let mut reused = [false; CAPACITY];
        let mut held: [Option<HandlerId>; CAPACITY] = [None; CAPACITY];
        let mut change = ChainChange::default();
        if new.len() != old.len() {
            change.layout = true;
            change.drawing = true;
        }
        for (index, element) in new.iter().enumerate() {
            let Some(named) = element.handler() else {
                continue;
            };
            let kind = match element {
                Element::Toggleable { .. } => HandlerKind::Toggle,
                Element::Layout { .. } => HandlerKind::LayoutModifier,
                _ => HandlerKind::Click,
            };
            // An equal chain keeps the entry held at this position.
            if chain_equal {
                reused[index] = true;
                held[index] = old[index].element.handler();
                continue;
            }
            // The entry named, held at this position or another one.
            let same_position = old
                .get(index)
                .is_some_and(|entry| entry.element.handler() == Some(named));
            let position = if same_position && !reused[index] {
                Some(index)
            } else {
                old.iter().enumerate().position(|(position, entry)| {
                    !reused[position] && entry.element.handler() == Some(named)
                })
            };
            if let Some(position) = position {
                reused[position] = true;
                held[index] = Some(named);
                continue;
            }
            held[index] = Some(handlers.claim(named, kind, id)?);
        }
        for (position, entry) in old.iter().enumerate() {
            if let (Some(handler), false) = (entry.element.handler(), reused[position]) {
                handlers.release(handler, drops);
            }
        }
        let node = self.node_mut(id).unwrap();
        if new.len() == node.chain.len() {
            for (index, element) in new.iter().enumerate() {
                let element = match held[index] {
                    Some(handler) => element.with_handler(handler),
                    None => *element,
                };
                let entry = &mut node.chain[index];
                let handlers_equal = entry.element.handler() == held[index];
                compare_elements(&entry.element, &element, handlers_equal, &mut change);
                entry.element = element;
            }
        } else {
            node.chain.clear();
            for (index, element) in new.iter().enumerate() {
                let element = match held[index] {
                    Some(handler) => element.with_handler(handler),
                    None => *element,
                };
                node.chain.push(ChainEntry::new(element));
            }
        }
        node.parent_data = node.chain_parent_data();
        // A weight or alignment is read by the parent, whose policy must run again.
        if node.parent_data != old_parent_data {
            if let Some(parent) = node.parent {
                self.mark_needs_measure(parent);
            }
        }
        self.apply(
            id,
            Update {
                layout: change.layout,
                drawing: change.drawing,
            },
        );
        Ok(Some(change))
    }

    /// [`UiTree::set_chain`], comparing handlers in the table: for callers that hold no
    /// thread state, whose handlers are Rust's (tests and [`crate::test_support`]).
    pub fn set_chain_comparing(
        &mut self,
        id: NodeId,
        new: &[Element],
        handlers: &mut HandlerTable,
        drops: &mut Vec<FunctionValue>,
    ) -> UiResult<Option<ChainChange>> {
        let equal = self
            .handler_pairs(id, new)
            .map(|pair| pair.is_some_and(|(named, current)| handlers.equal(named, current)));
        self.set_chain(id, new, &equal, handlers, drops)
    }

    /// The handler a text field holds, when it is not `named`: the pair whose equality
    /// [`UiTree::set_on_value_change`] needs.
    pub fn value_change_pair(
        &self,
        id: NodeId,
        named: HandlerId,
    ) -> Option<(HandlerId, HandlerId)> {
        let current = self.node(id)?.value_change?;
        (current != named).then_some((named, current))
    }

    /// Gives a text field its value-change handler: keeps the one it holds when `named` is it
    /// or, as `equal` says, equal to it, and otherwise claims `named` and releases the old one.
    pub fn set_on_value_change(
        &mut self,
        id: NodeId,
        named: HandlerId,
        equal: bool,
        handlers: &mut HandlerTable,
        drops: &mut Vec<FunctionValue>,
    ) -> UiResult<Option<()>> {
        let Some(node) = self.node(id) else {
            return Ok(None);
        };
        if let Some(current) = node.value_change {
            if current == named || equal {
                return Ok(Some(()));
            }
        }
        let claimed = handlers.claim(named, HandlerKind::Text, id)?;
        let node = self.node_mut(id).unwrap();
        if let Some(old) = node.value_change.replace(claimed) {
            handlers.release(old, drops);
        }
        Ok(Some(()))
    }

    /// The policy a node holds, when it is not `named`: the pair whose equality
    /// [`UiTree::set_measure_policy`] needs.
    pub fn measure_policy_pair(
        &self,
        id: NodeId,
        named: HandlerId,
    ) -> Option<(HandlerId, HandlerId)> {
        let current = self.node(id)?.policy?;
        (current != named).then_some((named, current))
    }

    /// Gives a node its measure policy (D8 of `layout-primitive`): keeps the one it holds when
    /// `named` is it or, as `equal` says, equal to it, and otherwise claims `named`, releases
    /// the old one and marks the node for measuring.
    pub fn set_measure_policy(
        &mut self,
        id: NodeId,
        named: HandlerId,
        equal: bool,
        handlers: &mut HandlerTable,
        drops: &mut Vec<FunctionValue>,
    ) -> UiResult<Option<()>> {
        let Some(node) = self.node(id) else {
            return Ok(None);
        };
        if let Some(current) = node.policy {
            if current == named || equal {
                return Ok(Some(()));
            }
        }
        let claimed = handlers.claim(named, HandlerKind::Measure, id)?;
        let node = self.node_mut(id).unwrap();
        if let Some(old) = node.policy.replace(claimed) {
            handlers.release(old, drops);
        }
        self.mark_needs_measure(id);
        Ok(Some(()))
    }

    /// Marks every node measured by a measure policy for measuring, and the root, so that the
    /// next layout runs every policy again from the root: what a reload does, since it may
    /// have changed the code of any policy (D12 of `layout-primitive`).
    pub fn mark_policies_for_measure(&mut self) {
        let root = self.root;
        let mut stack = std::mem::take(&mut self.release_stack);
        stack.push(root);
        while let Some(id) = stack.pop() {
            let node = self.node_mut(id).unwrap();
            // A `Modifier.layout`'s step is program code too (D8 of `layout-in-prism`).
            let steps = || {
                node.chain
                    .iter()
                    .any(|entry| matches!(entry.element, Element::Layout { .. }))
            };
            if node.kind.has_policy() || steps() {
                node.needs_measure = true;
            }
            let mut child = node.first_child;
            while let Some(current) = child {
                stack.push(current);
                child = self.node(current).unwrap().next;
            }
        }
        self.release_stack = stack;
        self.node_mut(root).unwrap().needs_measure = true;
    }
}

/// The children of a node, in order.
pub struct Children<'a> {
    tree: &'a UiTree,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let current = self.next?;
        self.next = self.tree.node(current).and_then(|node| node.next);
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::release_functions;
    use crate::properties::Role;
    use crate::test_support::{drops, handler, inline_handler, reset_drops};

    fn numbers(tree: &UiTree, parent: NodeId) -> Vec<u64> {
        tree.children(parent)
            .map(|child| tree.node(child).unwrap().number)
            .collect()
    }

    fn clean(tree: &mut UiTree) {
        let mut stack = vec![tree.root()];
        while let Some(id) = stack.pop() {
            stack.extend(tree.children(id));
            let node = tree.node_mut(id).unwrap();
            node.needs_measure = false;
            node.child_needs_measure = false;
        }
    }

    #[test]
    fn insert_move_and_unchanged_placement() {
        let mut tree = UiTree::new(400, 300);
        let root = tree.root();
        let a = tree.create(4).unwrap();
        let b = tree.create(4).unwrap();
        let c = tree.create(1).unwrap();
        tree.place_after(root, None, a);
        tree.place_after(root, Some(a), b);
        tree.place_after(root, Some(b), c);
        assert_eq!(numbers(&tree, root), [1, 2, 3]);
        clean(&mut tree);
        // A placement that changes nothing marks nothing.
        tree.place_after(root, Some(a), b);
        assert!(!tree.needs_layout());
        tree.place_after(root, None, c);
        assert_eq!(numbers(&tree, root), [3, 1, 2]);
        assert!(tree.node(root).unwrap().needs_measure);
        // Into another parent.
        tree.place_after(c, None, b);
        assert_eq!(numbers(&tree, root), [3, 1]);
        assert_eq!(numbers(&tree, c), [2]);
        assert_eq!(tree.len(), 3);
    }

    #[test]
    fn unknown_kinds_are_rejected() {
        let mut tree = UiTree::new(1, 1);
        assert_eq!(tree.create(9), Err(unknown_kind(9)));
        assert_eq!(tree.create(0), Err(unknown_kind(0)));
    }

    #[test]
    fn marking_stops_at_a_marked_ancestor() {
        let mut tree = UiTree::new(400, 300);
        let root = tree.root();
        let column = tree.create_kind(NodeKind::Column);
        let row = tree.create_kind(NodeKind::Row);
        let text = tree.create_kind(NodeKind::Text);
        tree.place_after(root, None, column);
        tree.place_after(column, None, row);
        tree.place_after(row, None, text);
        clean(&mut tree);
        tree.mark_needs_measure(text);
        assert!(tree.node(text).unwrap().needs_measure);
        assert!(tree.node(row).unwrap().child_needs_measure);
        assert!(tree.node(column).unwrap().child_needs_measure);
        assert!(tree.node(root).unwrap().child_needs_measure);
        assert!(!tree.node(row).unwrap().needs_measure);
        // The column's flag stops the walk: the root is not visited again.
        tree.node_mut(root).unwrap().child_needs_measure = false;
        tree.mark_needs_measure(row);
        assert!(!tree.node(root).unwrap().child_needs_measure);
    }

    #[test]
    fn removal_marks_the_parent_and_release_goes_children_first() {
        reset_drops();
        let mut tree = UiTree::new(400, 300);
        let mut handlers = HandlerTable::new();
        let mut out = Vec::new();
        let root = tree.root();
        let column = tree.create_kind(NodeKind::Column);
        let first = tree.create_kind(NodeKind::Box);
        let second = tree.create_kind(NodeKind::Box);
        tree.place_after(root, None, column);
        tree.place_after(column, None, first);
        tree.place_after(column, Some(first), second);
        for (node, name) in [(column, "column"), (first, "first"), (second, "second")] {
            let id = handlers.register(HandlerKind::Click, handler(name, || {}));
            let chain = [Element::Clickable {
                handler: id,
                enabled: true,
                role: None,
            }];
            tree.set_chain_comparing(node, &chain, &mut handlers, &mut out)
                .unwrap();
        }
        tree.set_text(first, "owned text");
        clean(&mut tree);
        tree.remove(root, column);
        assert!(tree.node(root).unwrap().needs_measure);
        assert!(tree.children(root).next().is_none());
        tree.release(column, &mut handlers, &mut out);
        release_functions(&mut out);
        assert_eq!(drops(), ["drop second", "drop first", "drop column"]);
        assert!(tree.is_empty());
        assert!(handlers.is_empty());
        // Numbers are never reused.
        let next = tree.create_kind(NodeKind::Text);
        assert_eq!(tree.node(next).unwrap().number, 4);
        assert!(tree.node(first).is_none(), "a released handle is stale");
    }

    #[test]
    fn setters_compare_before_invalidating() {
        let mut tree = UiTree::new(400, 300);
        let root = tree.root();
        let text = tree.create_kind(NodeKind::Text);
        tree.place_after(root, None, text);
        clean(&mut tree);
        tree.redraw = false;
        assert_eq!(tree.set_text(text, ""), Some(Update::default()));
        assert!(!tree.needs_layout());
        assert_eq!(
            tree.set_text(text, "a"),
            Some(Update {
                layout: true,
                drawing: true
            })
        );
        clean(&mut tree);
        tree.redraw = false;
        let update = tree.set_content_color(text, Color::WHITE).unwrap();
        assert!(update.drawing && !update.layout && tree.redraw && !tree.needs_layout());
        let style = TextStyle {
            color: Color::BLACK,
            font_size: -1,
        };
        let update = tree.set_text_style(text, style).unwrap();
        assert!(
            update.drawing && !update.layout,
            "the same resolved font size"
        );
        let update = tree
            .set_text_style(
                text,
                TextStyle {
                    font_size: 24,
                    ..style
                },
            )
            .unwrap();
        assert!(update.layout);
        assert_eq!(tree.set_text(NodeId(12345), "x"), None);
    }

    #[test]
    fn chains_claim_keep_and_release_handlers() {
        reset_drops();
        let mut tree = UiTree::new(400, 300);
        let mut handlers = HandlerTable::new();
        let mut out = Vec::new();
        let node = tree.create_kind(NodeKind::Box);
        let clickable = |handler| Element::Clickable {
            handler,
            enabled: true,
            role: Some(Role::Button),
        };
        let first = handlers.register(HandlerKind::Click, handler("first", || {}));
        let change = tree
            .set_chain_comparing(node, &[clickable(first)], &mut handlers, &mut out)
            .unwrap()
            .unwrap();
        assert!(change.layout, "a new chain");
        handlers.release_unclaimed(&mut out);
        // The same entry again changes nothing.
        let change = tree
            .set_chain_comparing(node, &[clickable(first)], &mut handlers, &mut out)
            .unwrap()
            .unwrap();
        assert!(!change.any());
        // A handler owning captures is never equal: the new one is claimed, the old released.
        let second = handlers.register(HandlerKind::Click, handler("second", || {}));
        let change = tree
            .set_chain_comparing(node, &[clickable(second)], &mut handlers, &mut out)
            .unwrap()
            .unwrap();
        assert!(change.input && !change.layout);
        release_functions(&mut out);
        assert_eq!(drops(), ["drop first"]);
        // An equal keepable handler keeps the held entry; the new one stays pending.
        let kept = handlers.register(HandlerKind::Click, inline_handler(3));
        tree.set_chain_comparing(node, &[clickable(kept)], &mut handlers, &mut out)
            .unwrap();
        release_functions(&mut out);
        assert_eq!(drops(), ["drop first", "drop second"]);
        let equal = handlers.register(HandlerKind::Click, inline_handler(3));
        let change = tree
            .set_chain_comparing(node, &[clickable(equal)], &mut handlers, &mut out)
            .unwrap()
            .unwrap();
        assert!(!change.any());
        assert_eq!(
            tree.node(node).unwrap().chain[0].element.handler(),
            Some(kept)
        );
        handlers.release_unclaimed(&mut out);
        assert_eq!(handlers.len(), 1);
        // A stale entry is misuse.
        assert_eq!(
            tree.set_chain_comparing(node, &[clickable(equal)], &mut handlers, &mut out),
            Err(crate::error::STALE_HANDLER)
        );
        // Dropping the element releases its entry.
        tree.set_chain_comparing(node, &[], &mut handlers, &mut out)
            .unwrap();
        assert!(handlers.is_empty());
        release_functions(&mut out);
    }

    #[test]
    fn a_changed_chain_takes_the_entries_it_names() {
        let mut tree = UiTree::new(400, 300);
        let mut handlers = HandlerTable::new();
        let mut out = Vec::new();
        let node = tree.create_kind(NodeKind::Box);
        let clickable = |handler, enabled| Element::Clickable {
            handler,
            enabled,
            role: None,
        };
        let first = handlers.register(HandlerKind::Click, inline_handler(3));
        tree.set_chain_comparing(node, &[clickable(first, true)], &mut handlers, &mut out)
            .unwrap();
        // An equal handler in a changed chain: the node takes the new entry, which the
        // composable passing the chain now keeps, and releases the old one.
        let second = handlers.register(HandlerKind::Click, inline_handler(3));
        tree.set_chain_comparing(node, &[clickable(second, false)], &mut handlers, &mut out)
            .unwrap();
        assert_eq!(
            tree.node(node).unwrap().chain[0].element.handler(),
            Some(second)
        );
        assert_eq!(handlers.owner(first), None);
        // An equal handler in an equal chain: the node keeps its entry, as that composable
        // keeps its old argument, and the new one stays pending.
        let third = handlers.register(HandlerKind::Click, inline_handler(3));
        tree.set_chain_comparing(node, &[clickable(third, false)], &mut handlers, &mut out)
            .unwrap();
        assert_eq!(
            tree.node(node).unwrap().chain[0].element.handler(),
            Some(second)
        );
        handlers.release_unclaimed(&mut out);
        assert_eq!(handlers.len(), 1);
        tree.set_chain_comparing(node, &[], &mut handlers, &mut out)
            .unwrap();
        release_functions(&mut out);
    }

    #[test]
    fn a_shared_handler_is_duplicated_or_rejected() {
        reset_drops();
        let mut tree = UiTree::new(400, 300);
        let mut handlers = HandlerTable::new();
        let mut out = Vec::new();
        let a = tree.create_kind(NodeKind::Box);
        let b = tree.create_kind(NodeKind::Box);
        let clickable = |handler| Element::Clickable {
            handler,
            enabled: true,
            role: None,
        };
        let copy = handlers.register(HandlerKind::Click, inline_handler(1));
        tree.set_chain_comparing(a, &[clickable(copy)], &mut handlers, &mut out)
            .unwrap();
        tree.set_chain_comparing(b, &[clickable(copy)], &mut handlers, &mut out)
            .unwrap();
        let held_by_b = tree.node(b).unwrap().chain[0].element.handler().unwrap();
        assert_ne!(held_by_b, copy);
        assert_eq!(handlers.len(), 2);
        let owning = handlers.register(HandlerKind::Click, handler("owning", || {}));
        tree.set_chain_comparing(a, &[clickable(owning)], &mut handlers, &mut out)
            .unwrap();
        assert_eq!(
            tree.set_chain_comparing(b, &[clickable(owning)], &mut handlers, &mut out),
            Err(crate::error::SHARED_HANDLER)
        );
        tree.release(a, &mut handlers, &mut out);
        tree.release(b, &mut handlers, &mut out);
        release_functions(&mut out);
        assert!(handlers.is_empty());
        assert_eq!(drops(), ["drop owning"]);
    }

    #[test]
    fn a_text_field_holds_its_value_change_handler() {
        reset_drops();
        let mut tree = UiTree::new(400, 300);
        let mut handlers = HandlerTable::new();
        let mut out = Vec::new();
        let field = tree.create_kind(NodeKind::TextField);
        let first = handlers.register(HandlerKind::Text, handler("first", || {}));
        tree.set_on_value_change(field, first, false, &mut handlers, &mut out)
            .unwrap();
        tree.set_on_value_change(field, first, false, &mut handlers, &mut out)
            .unwrap();
        let second = handlers.register(HandlerKind::Text, handler("second", || {}));
        tree.set_on_value_change(field, second, false, &mut handlers, &mut out)
            .unwrap();
        release_functions(&mut out);
        assert_eq!(drops(), ["drop first"]);
        let click = handlers.register(HandlerKind::Click, inline_handler(0));
        assert_eq!(
            tree.set_on_value_change(field, click, false, &mut handlers, &mut out),
            Err(crate::error::HANDLER_OF_ANOTHER_TYPE)
        );
        tree.release(field, &mut handlers, &mut out);
        handlers.release_all(&mut out);
        release_functions(&mut out);
        assert_eq!(drops(), ["drop first", "drop second"]);
    }

    /// D8 of `layout-primitive`: an equal policy is kept, another replaces the old one and
    /// marks the node, and releasing the node drops its policy.
    #[test]
    fn a_node_holds_its_measure_policy() {
        use crate::test_support::{counting_policy, policy};
        let mut tree = UiTree::new(400, 300);
        let mut handlers = HandlerTable::new();
        let mut drops = Vec::new();
        let root = tree.root();
        let column = tree.create_kind(NodeKind::Column);
        tree.place_after(root, None, column);
        let first = handlers.register(HandlerKind::Measure, policy(NodeKind::Column).unwrap());
        tree.set_measure_policy(column, first, false, &mut handlers, &mut drops)
            .unwrap();
        assert_eq!(tree.node(column).unwrap().policy, Some(first));
        clean(&mut tree);
        // An equal policy of a later pass is kept, and its own entry stays pending.
        let equal = handlers.register(HandlerKind::Measure, policy(NodeKind::Column).unwrap());
        let pair = tree.measure_policy_pair(column, equal);
        assert_eq!(pair, Some((equal, first)));
        assert!(handlers.equal(equal, first));
        tree.set_measure_policy(column, equal, true, &mut handlers, &mut drops)
            .unwrap();
        assert_eq!(tree.node(column).unwrap().policy, Some(first));
        assert!(!tree.needs_layout(), "an equal policy changes nothing");
        handlers.release_unclaimed(&mut drops);
        // Another policy replaces it, releases the old one and marks the node.
        let other = handlers.register(HandlerKind::Measure, counting_policy());
        assert!(!handlers.equal(other, first));
        tree.set_measure_policy(column, other, false, &mut handlers, &mut drops)
            .unwrap();
        assert_eq!(tree.node(column).unwrap().policy, Some(other));
        assert_eq!(handlers.owner(first), None, "the old policy is released");
        assert!(tree.node(column).unwrap().needs_measure);
        assert_eq!(handlers.len(), 1);
        // Releasing the node drops its policy.
        tree.release(column, &mut handlers, &mut drops);
        assert!(handlers.is_empty());
        crate::handlers::release_functions(&mut drops);
    }
}
