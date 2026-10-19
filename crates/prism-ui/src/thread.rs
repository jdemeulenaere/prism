//! The per-thread state of the UI toolkit (D2 of `ui-toolkit`).
//!
//! Each thread has its own node tree, handler table and at most one running app, in a
//! `thread_local!`, as `prism-compose`'s state is per thread. Prism runs an app on its program
//! thread, and Rust tests run on many threads, each with its own state.
//!
//! **The re-entrancy rule.** Every operation borrows the state through [`with_state`], does its
//! work and lets go of it before program code runs. Program code runs in three ways here: a
//! composition's content, a handler, and the drop entry of a released handler. A handler's
//! function value is copied out of its entry before it is called, and a released handler's
//! value is moved to [`State::drops`] and dropped by [`release_dropped`] once nothing is
//! borrowed. This matters most in the applier, which `prism-compose` calls while it holds its
//! own state: releasing a node there only moves its handlers to `drops`, and the app drops them
//! after the pass or the disposal returns, so no handler's drop entry runs inside the
//! composition runtime.

use crate::handlers::{release_functions, HandlerTable};
use crate::tree::UiTree;
use prism_compose::Handle;
use prism_runtime::FunctionValue;
use std::cell::RefCell;

/// The app running on the thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunningApp {
    /// Its composition, known once its first pass has started (D17's checks adopt the pass's
    /// composition while it is `None`).
    pub composition: Option<Handle>,
}

/// What one thread's toolkit holds.
#[derive(Debug)]
pub struct State {
    pub tree: UiTree,
    pub handlers: HandlerTable,
    pub app: Option<RunningApp>,
    /// Released handlers' function values, dropped by [`release_dropped`].
    pub drops: Vec<FunctionValue>,
    /// The nodes left in the trees [`install_tree`] replaced, which a release path forgot: the
    /// allocation check reports them with the current tree's ([`live_nodes_report`]).
    pub replaced_nodes: usize,
}

impl State {
    fn new() -> State {
        State {
            tree: UiTree::new(0, 0),
            handlers: HandlerTable::new(),
            app: None,
            drops: Vec::new(),
            replaced_nodes: 0,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new());
}

/// Runs `f` with this thread's state. `f` must not run program code (the re-entrancy rule).
pub fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|state| f(&mut state.borrow_mut()))
}

/// Gives the thread a new, empty tree with a window of `width` × `height`.
pub fn install_tree(width: i64, height: i64) {
    let old = with_state(|state| {
        let old = std::mem::replace(&mut state.tree, UiTree::new(width, height));
        state.replaced_nodes += old.len();
        old
    });
    drop(old);
}

/// The line the allocation check writes for this thread's nodes still live when `main` returns,
/// after `prism-compose`'s groups, slots and cells (D6 of `reactive-app-verification`):
/// `allocation check: nodes still live at exit: <n>`, counting the current tree's nodes, the
/// window root excepted, and those left in the trees [`install_tree`] replaced. `None` when a
/// composition is still live, since its nodes are expected to be live then, or when none is.
pub fn live_nodes_report() -> Option<String> {
    if prism_compose::live_compositions_report().is_some() {
        return None;
    }
    let live = with_state(|state| state.tree.len() + state.replaced_nodes);
    (live > 0).then(|| format!("allocation check: nodes still live at exit: {}", live))
}

/// Drops the function values released handlers left in [`State::drops`], outside the state,
/// keeping the vector's capacity. A drop entry that releases more handlers is followed.
pub fn release_dropped() {
    loop {
        let mut drops = with_state(|state| std::mem::take(&mut state.drops));
        if drops.is_empty() {
            with_state(|state| {
                if state.drops.capacity() == 0 {
                    state.drops = drops;
                }
            });
            return;
        }
        release_functions(&mut drops);
        with_state(|state| {
            if state.drops.is_empty() {
                std::mem::swap(&mut state.drops, &mut drops);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::HandlerKind;
    use crate::test_support::{drops, handler, reset_drops};

    #[test]
    fn two_threads_have_independent_states() {
        install_tree(400, 300);
        with_state(|state| {
            state.app = Some(RunningApp { composition: None });
        });
        let other = std::thread::spawn(|| {
            with_state(|state| {
                assert!(state.app.is_none(), "the other thread runs no app");
                assert_eq!(state.tree.window_size().width, 0);
            });
        });
        other.join().unwrap();
        with_state(|state| {
            assert_eq!(state.tree.window_size().width, 400);
            state.app = None;
        });
    }

    /// execution: "A leaked group, cell and node are reported", for this crate's part: a node
    /// left in the tree, or in a tree that a later app replaced, is reported.
    #[test]
    fn a_leaked_node_is_reported_even_after_its_tree_is_replaced() {
        std::thread::spawn(|| {
            assert_eq!(live_nodes_report(), None);
            install_tree(400, 300);
            crate::test_support::leak_node();
            assert_eq!(
                live_nodes_report().as_deref(),
                Some("allocation check: nodes still live at exit: 1")
            );
            install_tree(0, 0);
            crate::test_support::leak_node();
            assert_eq!(
                live_nodes_report().as_deref(),
                Some("allocation check: nodes still live at exit: 2")
            );
        })
        .join()
        .unwrap();
    }

    #[test]
    fn released_handlers_are_dropped_outside_the_state() {
        reset_drops();
        // A drop entry that uses the state would panic on a nested borrow.
        let id = with_state(|state| {
            state
                .handlers
                .register(HandlerKind::Click, handler("probe", || {}))
        });
        crate::test_support::on_drop(Box::new(|| {
            with_state(|state| assert!(state.handlers.is_empty()));
        }));
        with_state(|state| {
            let State {
                handlers, drops, ..
            } = state;
            handlers.release(id, drops);
        });
        release_dropped();
        assert_eq!(drops(), ["drop probe"]);
        assert!(with_state(|state| state.drops.is_empty()));
    }
}
