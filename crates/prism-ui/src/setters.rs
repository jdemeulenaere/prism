//! The node property setters and handler registration of the toolkit's composables (D3, D10,
//! D17 and D23 of `ui-toolkit`), as Rust functions; task 9.1's `prism_ui_*` entry points
//! decode their C arguments, call these and raise what they return.
//!
//! Each setter checks, in this order, that a pass is running, that it is the running app's
//! (the first pass of an app is adopted as the app's), and that the node is live; then it
//! compares the new value with the node's and invalidates only what changed (the tree's
//! updates). Released handler entries go to the thread's `drops`, which the app drops after
//! the pass.

use crate::error::{
    UiError, UiResult, HANDLER_WITHOUT_APP, OUTSIDE_COMPOSITION, OUTSIDE_RUN_APP, STALE_NODE,
};
use crate::handle::{HandlerId, NodeId};
use crate::handlers::{release_functions, Comparable, HandlerKind};
use crate::modifier::ModifierMirror;
use crate::properties::{Alignment, Arrangement, Color, TextStyle};
use crate::runtime;
use crate::thread::{with_state, State};
use crate::tree::UiTree;
use prism_runtime::FunctionValue;
use std::borrow::Cow;

/// Runs `update` on the tree once the pass and the node are checked.
fn with_node<R>(node: NodeId, update: impl FnOnce(&mut State) -> UiResult<R>) -> UiResult<R> {
    let active = runtime::active_composition().ok_or(OUTSIDE_COMPOSITION)?;
    with_state(|state| {
        let app = state.app.as_mut().ok_or(OUTSIDE_RUN_APP)?;
        match app.composition {
            Some(composition) if composition != active => return Err(OUTSIDE_RUN_APP),
            Some(_) => {}
            None => app.composition = Some(active),
        }
        if state.tree.node(node).is_none() || node == state.tree.root() {
            return Err(STALE_NODE);
        }
        update(state)
    })
}

fn tree_update(node: NodeId, update: impl FnOnce(&mut UiTree)) -> UiResult<()> {
    with_node(node, |state| {
        update(&mut state.tree);
        Ok(())
    })
}

/// `prism_ui_set_text`: a text's text or a text field's value.
pub fn set_text(node: NodeId, text: &str) -> UiResult<()> {
    tree_update(node, |tree| {
        tree.set_text(node, text);
    })
}

/// `prism_ui_set_text_style`: the `argb` word of a colour and a font size, either
/// unspecified.
pub fn set_text_style(node: NodeId, color: i64, font_size: i64) -> UiResult<()> {
    tree_update(node, |tree| {
        tree.set_text_style(
            node,
            TextStyle {
                color: Color(color),
                font_size,
            },
        );
    })
}

/// `prism_ui_set_arrangement`: an arrangement's code and spacing.
pub fn set_arrangement(node: NodeId, code: i64, space: i64) -> UiResult<()> {
    let arrangement = Arrangement::from_words(code, space)
        .ok_or_else(|| UiError::Misuse(Cow::Owned(format!("unknown arrangement {}", code))))?;
    tree_update(node, |tree| {
        tree.set_arrangement(node, arrangement);
    })
}

/// `prism_ui_set_alignment`: an alignment's code.
pub fn set_alignment(node: NodeId, code: i64) -> UiResult<()> {
    let alignment = Alignment::from_code(code)
        .ok_or_else(|| UiError::Misuse(Cow::Owned(format!("unknown alignment {}", code))))?;
    tree_update(node, |tree| {
        tree.set_alignment(node, alignment);
    })
}

/// `prism_ui_set_content_color`.
pub fn set_content_color(node: NodeId, color: i64) -> UiResult<()> {
    tree_update(node, |tree| {
        tree.set_content_color(node, Color(color));
    })
}

/// `prism_ui_set_enabled`.
pub fn set_enabled(node: NodeId, enabled: bool) -> UiResult<()> {
    tree_update(node, |tree| {
        tree.set_enabled(node, enabled);
    })
}

/// Copies of the handlers of `pair`, to compare once the state is let go.
fn comparables(
    state: &State,
    pair: Option<(HandlerId, HandlerId)>,
) -> Option<(Option<Comparable>, Option<Comparable>)> {
    let (a, b) = pair?;
    Some((state.handlers.comparable(a), state.handlers.comparable(b)))
}

/// Whether the copied handlers are equal. Their environment equality entries are program
/// code, which may compare handler ids again, so no state is borrowed here (D2); the entries
/// stay live meanwhile, since comparing releases nothing.
fn compare(copies: &Option<(Option<Comparable>, Option<Comparable>)>) -> bool {
    copies
        .as_ref()
        .is_some_and(|(a, b)| Comparable::equal(a.as_ref(), b.as_ref()))
}

/// `prism_ui_set_modifier`: decodes the chain and sets it, claiming its handlers.
pub fn set_modifier(node: NodeId, modifier: &ModifierMirror) -> UiResult<()> {
    let chain = modifier.decode();
    let copies = with_node(node, |state| {
        let pairs = state.tree.handler_pairs(node, chain.as_slice());
        Ok(pairs.map(|pair| comparables(state, pair)))
    })?;
    let equal = copies.each_ref().map(compare);
    with_node(node, |state| {
        let State {
            tree,
            handlers,
            drops,
            ..
        } = state;
        tree.set_chain(node, chain.as_slice(), &equal, handlers, drops)
            .map(|_| ())
    })
}

/// `prism_ui_set_on_value_change`: a text field's handler.
pub fn set_on_value_change(node: NodeId, handler: HandlerId) -> UiResult<()> {
    let copies = with_node(node, |state| {
        Ok(comparables(
            state,
            state.tree.value_change_pair(node, handler),
        ))
    })?;
    let equal = compare(&copies);
    with_node(node, |state| {
        let State {
            tree,
            handlers,
            drops,
            ..
        } = state;
        tree.set_on_value_change(node, handler, equal, handlers, drops)
            .map(|_| ())
    })
}

/// `prism_ui_set_measure_policy`: a layout node's measure policy (D8 of `layout-primitive`),
/// compared with the one it holds outside the state's borrow, as handlers are.
pub fn set_measure_policy(node: NodeId, policy: HandlerId) -> UiResult<()> {
    let copies = with_node(node, |state| {
        Ok(comparables(
            state,
            state.tree.measure_policy_pair(node, policy),
        ))
    })?;
    let equal = compare(&copies);
    with_node(node, |state| {
        let State {
            tree,
            handlers,
            drops,
            ..
        } = state;
        tree.set_measure_policy(node, policy, equal, handlers, drops)
            .map(|_| ())
    })
}

/// `prism_ui_register_measure_policy`: moves a toolkit layout's policy into a pending entry,
/// during a pass of the running app only, as the toolkit's setters check (D8 of
/// `layout-primitive`); otherwise `function` is dropped and the result is the setters' misuse.
pub fn register_measure_policy(function: FunctionValue) -> UiResult<HandlerId> {
    let checked = match runtime::active_composition() {
        None => Err(OUTSIDE_COMPOSITION),
        Some(active) => with_state(|state| {
            let app = state.app.as_mut().ok_or(OUTSIDE_RUN_APP)?;
            match app.composition {
                Some(composition) if composition != active => Err(OUTSIDE_RUN_APP),
                Some(_) => Ok(()),
                None => {
                    app.composition = Some(active);
                    Ok(())
                }
            }
        }),
    };
    if let Err(error) = checked {
        release_functions(&mut vec![function]);
        return Err(error);
    }
    register(HandlerKind::Measure, function)
}

/// `prism_ui_register_click`, `_toggle` and `_text`: moves `function` into a pending entry of
/// the handler table. With no app on the thread, `function` is dropped and the result is
/// `ui misuse: modifier handlers are made only while runApp runs`.
///
/// A handler whose environment is a word owning nothing gives the id of a live entry holding
/// an equal handler, when there is one, and is dropped (which runs nothing): so the handler
/// that `Modifier.clickable { count.value += 1 }` builds on every pass is one entry, and the
/// chains naming it name a live entry wherever they are kept.
pub fn register(kind: HandlerKind, function: FunctionValue) -> UiResult<HandlerId> {
    let interned = with_state(|state| state.app.map(|_| state.handlers.interned(kind, &function)));
    let Some(interned) = interned else {
        release_functions(&mut vec![function]);
        return Err(HANDLER_WITHOUT_APP);
    };
    if let Some((id, existing)) = interned {
        if existing.equals_function(kind, &function) {
            // SAFETY: an interned handler's environment owns nothing: it has no drop entry.
            unsafe { function.release() };
            return Ok(id);
        }
    }
    Ok(with_state(|state| state.handlers.register(kind, function)))
}

/// `prism_ui_handlers_equal`, comparing copies of the two handlers once the state is let go.
pub fn handlers_equal(a: HandlerId, b: HandlerId) -> bool {
    if a == b {
        return true;
    }
    let copies = with_state(|state| comparables(state, Some((a, b))));
    compare(&copies)
}

/// `prism_ui_modifier_overflow`: the misuse of a seventeenth element.
pub fn modifier_overflow() -> UiError {
    crate::error::CHAIN_OVERFLOW
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{drops, handler, inline_handler, node_group, reset_drops};
    use crate::thread::{install_tree, release_dropped, RunningApp};
    use prism_compose::test_support::content;
    use prism_compose::{Composition, NodeTree};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn a_setter_outside_any_pass() {
        install_tree(10, 10);
        assert_eq!(set_text(NodeId(1), "a"), Err(OUTSIDE_COMPOSITION));
    }

    #[test]
    fn a_setter_in_a_plain_composition() {
        install_tree(10, 10);
        let seen = Rc::new(RefCell::new(None));
        let result = seen.clone();
        let c = Composition::start(
            Box::new(NodeTree::new()),
            content(move |k| {
                let node = k.start_node_group(1, 4).unwrap();
                *result.borrow_mut() = Some(set_text(NodeId(node.0), "a"));
                k.end_group().unwrap();
            }),
        )
        .unwrap();
        assert_eq!(seen.borrow_mut().take(), Some(Err(OUTSIDE_RUN_APP)));
        // With an app running on the thread, a pass of another composition is still not its.
        with_state(|state| {
            state.app = Some(RunningApp {
                composition: Some(prism_compose::Handle(12345)),
            })
        });
        prism_compose::invalidate(Composition::root_scope(c).unwrap());
        Composition::recompose(c).unwrap();
        assert_eq!(seen.borrow_mut().take(), Some(Err(OUTSIDE_RUN_APP)));
        with_state(|state| state.app = None);
        Composition::dispose(c).unwrap();
    }

    #[test]
    fn a_stale_node_handle_and_unknown_codes() {
        install_tree(10, 10);
        with_state(|state| state.app = Some(RunningApp { composition: None }));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let result = seen.clone();
        let c = Composition::start(
            Box::new(crate::UiApplier),
            content(move |k| {
                let node = node_group(k, 1, 3);
                let mut results = result.borrow_mut();
                results.push(set_text(NodeId(node.0 + (1 << 32)), "a"));
                results.push(set_alignment(node, 99));
                results.push(set_arrangement(node, -1, 0));
                results.push(set_alignment(node, 4));
                k.end_group().unwrap();
            }),
        )
        .unwrap();
        assert_eq!(
            seen.borrow()
                .iter()
                .map(|result| result.as_ref().map_err(UiError::message))
                .collect::<Vec<_>>(),
            [
                Err("ui misuse: stale node handle".to_string()),
                Err("ui misuse: unknown alignment 99".to_string()),
                Err("ui misuse: unknown arrangement -1".to_string()),
                Ok(&()),
            ]
        );
        assert_eq!(
            with_state(|state| state.app.unwrap().composition),
            Some(c),
            "the first pass is adopted as the app's"
        );
        Composition::dispose(c).unwrap();
        release_dropped();
        with_state(|state| state.app = None);
    }

    #[test]
    fn registering_needs_an_app() {
        reset_drops();
        install_tree(10, 10);
        assert_eq!(
            register(HandlerKind::Click, handler("lost", || {})),
            Err(HANDLER_WITHOUT_APP)
        );
        assert_eq!(drops(), ["drop lost"], "the handler is dropped");
        with_state(|state| state.app = Some(RunningApp { composition: None }));
        let a = register(HandlerKind::Click, inline_handler(1)).unwrap();
        let b = register(HandlerKind::Click, inline_handler(1)).unwrap();
        assert!(handlers_equal(a, b));
        with_state(|state| {
            let State {
                handlers, drops, ..
            } = state;
            handlers.release_all(drops);
            state.app = None;
        });
        release_dropped();
    }
}
