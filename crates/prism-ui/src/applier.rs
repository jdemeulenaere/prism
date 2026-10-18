//! The applier the compose runtime calls for the nodes of an app's node groups (D10 of
//! `ui-toolkit`, `compose-runtime` D9).
//!
//! [`UiApplier`] holds nothing: it acts on the thread's [`UiTree`](crate::tree::UiTree). Node
//! handles are the tree's [`NodeId`] words, which `start_node_group` returns to the toolkit's
//! composables. `prism-compose` calls these methods while it holds its own state, so they must
//! run no program code and never call back into it (the compose spec's "An applier's methods
//! are Rust code"): releasing a node frees its text, a runtime string, and moves its handlers'
//! function values to the thread's `drops`, which the app drops after the pass or the disposal
//! returns ([`crate::thread::release_dropped`]).

use crate::error::raise;
use crate::handle::NodeId;
use crate::thread::{with_state, State};
use prism_compose::{Applier, NodeHandle};

/// The applier of an app's composition.
#[derive(Debug, Default, Clone, Copy)]
pub struct UiApplier;

fn id(node: NodeHandle) -> NodeId {
    NodeId(node.0)
}

impl Applier for UiApplier {
    fn root(&self) -> NodeHandle {
        with_state(|state| NodeHandle(state.tree.root().0))
    }

    /// Creates a node; an unknown kind ends the program (`ui misuse: unknown node kind <k>`),
    /// since only the toolkit's own code passes kinds.
    fn create(&mut self, kind: i64) -> NodeHandle {
        match with_state(|state| state.tree.create(kind)) {
            Ok(node) => NodeHandle(node.0),
            Err(error) => raise(&error),
        }
    }

    fn place(&mut self, parent: NodeHandle, previous: Option<NodeHandle>, node: NodeHandle) {
        with_state(|state| {
            state
                .tree
                .place_after(id(parent), previous.map(id), id(node))
        })
    }

    fn remove(&mut self, parent: NodeHandle, node: NodeHandle) {
        with_state(|state| state.tree.remove(id(parent), id(node)))
    }

    fn release(&mut self, node: NodeHandle) {
        with_state(|state| {
            let State {
                tree,
                handlers,
                drops,
                ..
            } = state;
            tree.release(id(node), handlers, drops);
        })
    }

    fn number(&self, node: NodeHandle) -> u64 {
        with_state(|state| state.tree.node(id(node)).map_or(0, |node| node.number))
    }

    /// The tree dump of `dump` (D16), as of the last layout.
    fn dump(&self, out: &mut String) {
        with_state(|state| crate::dump::dump_tree(&state.tree, None, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread::{install_tree, release_dropped};
    use prism_compose::test_support::content;
    use prism_compose::{invalidate, Composition};
    use std::cell::Cell;
    use std::rc::Rc;

    fn kinds_and_numbers(parent: NodeId) -> Vec<(String, u64)> {
        with_state(|state| {
            state
                .tree
                .children(parent)
                .map(|child| {
                    let node = state.tree.node(child).unwrap();
                    (node.kind.name().to_string(), node.number)
                })
                .collect()
        })
    }

    #[test]
    fn node_groups_create_move_and_remove_ui_nodes() {
        install_tree(400, 300);
        // Each pass emits a column holding texts in the order of `order`, keyed by their ids.
        let order = Rc::new(Cell::new([1i64, 2, 3]));
        let shown = order.clone();
        let column = Rc::new(Cell::new(None));
        let seen_column = column.clone();
        let c = Composition::start(
            Box::new(UiApplier),
            content(move |k| {
                let node = k.start_node_group(10, 1).unwrap();
                seen_column.set(Some(NodeId(node.0)));
                for id in shown.get() {
                    if id != 0 {
                        k.start_group(prism_compose::GroupKind::Movable, 20, id)
                            .unwrap();
                        k.start_node_group(30, 4).unwrap();
                        k.end_group().unwrap();
                        k.end_group().unwrap();
                    }
                }
                k.end_group().unwrap();
            }),
        )
        .unwrap();
        let root = with_state(|state| state.tree.root());
        let column = column.get().unwrap();
        assert_eq!(kinds_and_numbers(root), [("column".to_string(), 1)]);
        let texts = |numbers: &[u64]| -> Vec<(String, u64)> {
            numbers.iter().map(|n| ("text".to_string(), *n)).collect()
        };
        assert_eq!(kinds_and_numbers(column), texts(&[2, 3, 4]));
        // Reordering moves the nodes; none is created.
        order.set([3, 1, 2]);
        invalidate(Composition::root_scope(c).unwrap());
        assert!(Composition::recompose(c).unwrap());
        assert_eq!(kinds_and_numbers(column), texts(&[4, 2, 3]));
        // Removing a group removes and releases its node after the pass; a new one gets a new
        // number.
        order.set([3, 0, 2]);
        invalidate(Composition::root_scope(c).unwrap());
        Composition::recompose(c).unwrap();
        assert_eq!(kinds_and_numbers(column), texts(&[4, 3]));
        order.set([3, 1, 2]);
        invalidate(Composition::root_scope(c).unwrap());
        Composition::recompose(c).unwrap();
        assert_eq!(kinds_and_numbers(column), texts(&[4, 5, 3]));
        assert_eq!(with_state(|state| state.tree.len()), 4);
        // Disposing releases every node.
        Composition::dispose(c).unwrap();
        release_dropped();
        assert!(with_state(|state| state.tree.is_empty()));
    }
}
