//! Input dispatch (D14 of `ui-toolkit`): hit testing, clicks on release, focus, and the
//! keys a focused text field takes.
//!
//! - **Hit testing** walks the laid-out tree in reverse drawing order: the last child first,
//!   children before their parent, and within a node its own text field content before its
//!   input elements, last to first. The first target whose bounds hold the point wins: a
//!   `clickable` or `toggleable` element in the bounds at its position, or a text field at its
//!   frame (its content without one), where its steps placed it, enabled or not.
//! - **Clicks.** A press records its target; a release over the same, enabled target clicks
//!   it: a `clickable` calls `onClick()`, a `toggleable` `onValueChange(!value)`, and a text
//!   field takes the focus. A disabled target absorbs both and calls nothing, so clicks never
//!   fall through. A press that hits no text field clears the focus.
//! - **Keys** go to the focused, enabled text field, which calls its `onValueChange` with a
//!   new value built from the one it shows: a character appended, or the last character
//!   removed by backspace (nothing when empty); enter does nothing yet.
//!
//! The pressed target and the focus are this dispatcher's state, never the composition's
//! (D25). Dispatch returns an [`Action`] rather than calling the handler, so the caller can
//! let go of every borrow first and copy the function value out of its entry (D2).

use crate::handle::{HandlerId, NodeId};
use crate::modifier::Element;
use crate::node::{NodeKind, OwnedText};
use crate::runtime;
use crate::tree::UiTree;
use prism_runtime::FunctionValue;

/// What a pointer hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The input element at `index` of `node`'s chain.
    Element { node: NodeId, index: usize },
    /// A text field.
    TextField(NodeId),
}

/// The keys a text field handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Backspace,
    Enter,
}

/// A handler call that dispatch asks for.
#[derive(Debug)]
pub enum Action {
    /// `onClick()`.
    Click(HandlerId),
    /// `onValueChange(value)` of a `toggleable`.
    Toggle(HandlerId, bool),
    /// `onValueChange(value)` of a text field, the new value moved into the call.
    Text(HandlerId, OwnedText),
}

impl Action {
    pub fn handler(&self) -> HandlerId {
        match self {
            Action::Click(handler) | Action::Toggle(handler, _) | Action::Text(handler, _) => {
                *handler
            }
        }
    }

    /// Calls `function`, the action's handler copied out of its entry, with the action's
    /// arguments. It runs program code, so the caller must borrow no state.
    ///
    /// # Safety
    ///
    /// `function` must be the live function value of the action's handler, whose type the
    /// handler table checked when the node claimed it.
    pub unsafe fn perform(self, function: &FunctionValue) {
        match self {
            Action::Click(_) => runtime::call_unit(function),
            Action::Toggle(_, value) => runtime::call_with_bool(function, value),
            Action::Text(_, value) => runtime::call_with_string(function, value.into_string()),
        }
    }
}

/// The topmost target at `(x, y)`.
pub fn hit_test(tree: &UiTree, x: i64, y: i64) -> Option<Target> {
    visit(tree, tree.root(), 0, 0, x, y)
}

fn visit(
    tree: &UiTree,
    id: NodeId,
    parent_x: i64,
    parent_y: i64,
    x: i64,
    y: i64,
) -> Option<Target> {
    let node = tree.node(id).unwrap();
    let (origin_x, origin_y) = (parent_x + node.x, parent_y + node.y);
    let mut child = node.last_child;
    while let Some(current) = child {
        // An unplaced node and its subtree are not hit (D7 of `layout-primitive`).
        if tree.node(current).unwrap().placed {
            if let Some(target) = visit(tree, current, origin_x, origin_y, x, y) {
                return Some(target);
            }
        }
        child = tree.node(current).unwrap().previous;
    }
    // A text field is hit where it takes pointer input, which follows its steps.
    if node.kind == NodeKind::TextField
        && node
            .text_field_hit_area()
            .offset(origin_x, origin_y)
            .contains(x, y)
    {
        return Some(Target::TextField(id));
    }
    for (index, entry) in node.chain.iter().enumerate().rev() {
        let is_input = matches!(
            entry.element,
            Element::Clickable { .. } | Element::Toggleable { .. }
        );
        if is_input && entry.bounds.offset(origin_x, origin_y).contains(x, y) {
            return Some(Target::Element { node: id, index });
        }
    }
    None
}

/// The pressed target and the focused text field.
#[derive(Debug, Default)]
pub struct Dispatcher {
    pub pressed: Option<Target>,
    pub focused: Option<NodeId>,
}

impl Dispatcher {
    pub fn new() -> Dispatcher {
        Dispatcher::default()
    }

    /// Forgets the targets whose nodes were released.
    fn forget_released(&mut self, tree: &UiTree) {
        if self.focused.is_some_and(|node| tree.node(node).is_none()) {
            self.focused = None;
        }
        let pressed_node = match self.pressed {
            Some(Target::Element { node, .. } | Target::TextField(node)) => Some(node),
            None => None,
        };
        if pressed_node.is_some_and(|node| tree.node(node).is_none()) {
            self.pressed = None;
        }
    }

    /// The focused text field, if it still exists.
    pub fn focused(&mut self, tree: &UiTree) -> Option<NodeId> {
        self.forget_released(tree);
        self.focused
    }

    /// A press at `(x, y)`; gives whether the focus changed.
    pub fn press(&mut self, tree: &UiTree, x: i64, y: i64) -> bool {
        self.forget_released(tree);
        let target = hit_test(tree, x, y);
        self.pressed = target;
        !matches!(target, Some(Target::TextField(_))) && self.focused.take().is_some()
    }

    /// A release at `(x, y)`: the action of a click on the pressed target, if the release
    /// hits it and it is enabled, and whether the focus changed.
    pub fn release(&mut self, tree: &UiTree, x: i64, y: i64) -> (Option<Action>, bool) {
        self.forget_released(tree);
        let pressed = self.pressed.take();
        let target = hit_test(tree, x, y);
        if pressed.is_none() || pressed != target {
            return (None, false);
        }
        match target.unwrap() {
            Target::Element { node, index } => {
                let element = tree.node(node).unwrap().chain[index].element;
                let action = match element {
                    Element::Clickable {
                        handler,
                        enabled: true,
                        ..
                    } => Some(Action::Click(handler)),
                    Element::Toggleable {
                        handler,
                        value,
                        enabled: true,
                        ..
                    } => Some(Action::Toggle(handler, !value)),
                    _ => None,
                };
                (action, false)
            }
            Target::TextField(node) => {
                let enabled = tree.node(node).unwrap().enabled;
                if !enabled || self.focused == Some(node) {
                    return (None, false);
                }
                self.focused = Some(node);
                (None, true)
            }
        }
    }

    /// The focused, enabled text field with a value-change handler, and its value.
    fn editable<'a>(&mut self, tree: &'a UiTree) -> Option<(HandlerId, &'a str)> {
        let node = tree.node(self.focused(tree)?)?;
        if !node.enabled {
            return None;
        }
        Some((node.value_change?, node.text.as_str()))
    }

    /// A typed character.
    pub fn character(&mut self, tree: &UiTree, character: char) -> Option<Action> {
        let (handler, value) = self.editable(tree)?;
        let mut buffer = [0u8; 4];
        let typed = character.encode_utf8(&mut buffer);
        let mut text = String::with_capacity(value.len() + typed.len());
        text.push_str(value);
        text.push_str(typed);
        Some(Action::Text(handler, OwnedText::new(&text)))
    }

    /// A key.
    pub fn key(&mut self, tree: &UiTree, key: Key) -> Option<Action> {
        let (handler, value) = self.editable(tree)?;
        match key {
            Key::Enter => None,
            Key::Backspace => {
                let (last, _) = value.char_indices().next_back()?;
                Some(Action::Text(handler, OwnedText::new(&value[..last])))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::HandlerKind;
    use crate::properties::{Color, Role, Shape};
    use crate::test_support::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn lay_out(builder: &mut TreeBuilder) {
        builder.lay_out();
    }

    fn click(dispatcher: &mut Dispatcher, builder: &TreeBuilder, x: i64, y: i64) -> Option<Action> {
        dispatcher.press(&builder.tree, x, y);
        dispatcher.release(&builder.tree, x, y).0
    }

    fn perform(builder: &TreeBuilder, action: Action) {
        let function = builder.handlers.function(action.handler()).unwrap();
        unsafe { action.perform(&function) }
    }

    /// `Button(onClick)` holding a text, as material3 builds it.
    fn button(
        builder: &mut TreeBuilder,
        parent: NodeId,
        label: &str,
        enabled: bool,
        on_click: FunctionValue,
    ) -> NodeId {
        let node = builder.add(parent, NodeKind::Box);
        let id = builder.register(HandlerKind::Click, on_click);
        builder.chain(
            node,
            &[
                background(0xFF67_50A4, Shape::new(0, 50)),
                clickable(id, enabled, Some(Role::Button)),
            ],
        );
        let row = builder.add(node, NodeKind::Row);
        builder.chain(row, &[default_min_size(58, 40), padding_each(24, 8, 24, 8)]);
        builder.text(row, label);
        node
    }

    #[test]
    fn the_topmost_button_is_hit() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let boxed = b.add(root, NodeKind::Box);
        let (under, over) = (log.clone(), log.clone());
        button(
            &mut b,
            boxed,
            "under",
            true,
            handler("under", move || under.borrow_mut().push("under")),
        );
        let top = button(
            &mut b,
            boxed,
            "over",
            true,
            handler("over", move || over.borrow_mut().push("over")),
        );
        lay_out(&mut b);
        assert_eq!(
            hit_test(&b.tree, 10, 20),
            Some(Target::Element {
                node: top,
                index: 1
            })
        );
        let mut dispatcher = Dispatcher::new();
        let action = click(&mut dispatcher, &b, 10, 20).unwrap();
        perform(&b, action);
        assert_eq!(*log.borrow(), ["over"]);
        assert_eq!(hit_test(&b.tree, 300, 200), None);
    }

    #[test]
    fn an_elements_bounds_follow_the_chain() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let boxed = b.add(root, NodeKind::Box);
        let id = b.register(HandlerKind::Click, inline_handler(0));
        b.chain(
            boxed,
            &[padding(20), clickable(id, true, None), size(10, 10)],
        );
        lay_out(&mut b);
        let mut dispatcher = Dispatcher::new();
        assert!(
            click(&mut dispatcher, &b, 5, 5).is_none(),
            "inside the padding"
        );
        assert!(matches!(
            click(&mut dispatcher, &b, 25, 25),
            Some(Action::Click(_))
        ));
        assert!(
            click(&mut dispatcher, &b, 30, 30).is_none(),
            "the right edge is outside"
        );
    }

    #[test]
    fn a_later_element_of_a_chain_is_hit_first() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let boxed = b.add(root, NodeKind::Box);
        let outer = b.register(HandlerKind::Click, inline_handler(1));
        let inner = b.register(HandlerKind::Click, inline_handler(2));
        b.chain(
            boxed,
            &[
                clickable(outer, true, None),
                padding(4),
                clickable(inner, true, None),
                size(10, 10),
            ],
        );
        lay_out(&mut b);
        assert_eq!(
            hit_test(&b.tree, 5, 5),
            Some(Target::Element {
                node: boxed,
                index: 2
            })
        );
        assert_eq!(
            hit_test(&b.tree, 1, 1),
            Some(Target::Element {
                node: boxed,
                index: 0
            })
        );
    }

    #[test]
    fn releasing_elsewhere_does_not_click() {
        let mut b = TreeBuilder::new(400, 300);
        let screen = counter_screen(&mut b, "Count: 0", inline_handler(1));
        lay_out(&mut b);
        let mut dispatcher = Dispatcher::new();
        dispatcher.press(&b.tree, 45, 64);
        assert_eq!(
            dispatcher.pressed,
            Some(Target::Element {
                node: screen.button,
                index: 1
            })
        );
        assert!(dispatcher.release(&b.tree, 300, 200).0.is_none());
        assert_eq!(dispatcher.pressed, None);
        // A release without a press clicks nothing either.
        assert!(dispatcher.release(&b.tree, 45, 64).0.is_none());
    }

    #[test]
    fn disabled_targets_absorb_input() {
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let under = b.add(root, NodeKind::Box);
        let id = b.register(HandlerKind::Click, inline_handler(0));
        b.chain(under, &[clickable(id, true, None), size(100, 100)]);
        let boxed = b.add(root, NodeKind::Box);
        button(&mut b, boxed, "Go", false, inline_handler(1));
        let field = b.text_field(root, "x");
        b.chain(field, &[padding_each(0, 50, 0, 0)])
            .enabled(field, false);
        lay_out(&mut b);
        let mut dispatcher = Dispatcher::new();
        assert!(
            click(&mut dispatcher, &b, 10, 10).is_none(),
            "the disabled button is hit"
        );
        dispatcher.press(&b.tree, 2, 60);
        let (action, focus) = dispatcher.release(&b.tree, 2, 60);
        assert!(action.is_none() && !focus && dispatcher.focused.is_none());
        assert!(matches!(
            click(&mut dispatcher, &b, 90, 90),
            Some(Action::Click(_))
        ));
    }

    #[test]
    fn a_toggle_calls_with_the_opposite_value() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let checkbox = b.add(root, NodeKind::Box);
        let record = seen.clone();
        let id = b.register(
            HandlerKind::Toggle,
            bool_handler("toggle", move |value| record.borrow_mut().push(value)),
        );
        b.chain(
            checkbox,
            &[
                toggleable(id, false, true, Some(Role::Checkbox)),
                size(20, 20),
            ],
        );
        lay_out(&mut b);
        let mut dispatcher = Dispatcher::new();
        let action = click(&mut dispatcher, &b, 10, 10).unwrap();
        assert!(matches!(action, Action::Toggle(_, true)));
        perform(&b, action);
        let held = b.tree.node(checkbox).unwrap().chain[0]
            .element
            .handler()
            .unwrap();
        b.chain(
            checkbox,
            &[
                toggleable(held, true, true, Some(Role::Checkbox)),
                size(20, 20),
            ],
        );
        let action = click(&mut dispatcher, &b, 10, 10).unwrap();
        perform(&b, action);
        assert_eq!(*seen.borrow(), [true, false]);
    }

    /// The typing app's field: `TextField(value, onValueChange)` in a column.
    fn typing_app(
        builder: &mut TreeBuilder,
        value: &str,
        log: &Rc<RefCell<Vec<String>>>,
    ) -> NodeId {
        let root = builder.root();
        let column = builder.add(root, NodeKind::Column);
        let field = builder.text_field(column, value);
        builder.chain(
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
        let record = log.clone();
        let id = builder.register(
            HandlerKind::Text,
            text_handler("edit", move |text| {
                record.borrow_mut().push(text.to_string())
            }),
        );
        builder.on_value_change(field, id);
        builder.text(column, "Hello");
        field
    }

    #[test]
    fn focus_is_set_on_release_and_cleared_by_a_press_elsewhere() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut b = TreeBuilder::new(400, 300);
        let field = typing_app(&mut b, "", &log);
        lay_out(&mut b);
        let mut dispatcher = Dispatcher::new();
        assert!(!dispatcher.press(&b.tree, 100, 18));
        assert_eq!(dispatcher.focused, None, "focus waits for the release");
        assert!(dispatcher.release(&b.tree, 100, 18).1);
        assert_eq!(dispatcher.focused(&b.tree), Some(field));
        // Clicking the focused field again changes nothing.
        assert!(!dispatcher.press(&b.tree, 10, 10));
        assert!(!dispatcher.release(&b.tree, 10, 10).1);
        // A press on the text below clears it.
        assert!(dispatcher.press(&b.tree, 5, 40));
        assert_eq!(dispatcher.focused, None);
    }

    #[test]
    fn typing_backspace_and_enter() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut b = TreeBuilder::new(400, 300);
        let field = typing_app(&mut b, "", &log);
        lay_out(&mut b);
        let mut dispatcher = Dispatcher::new();
        assert!(
            dispatcher.character(&b.tree, 'a').is_none(),
            "nothing is focused"
        );
        click(&mut dispatcher, &b, 100, 18);
        assert!(
            dispatcher.key(&b.tree, Key::Backspace).is_none(),
            "an empty value"
        );
        assert!(dispatcher.key(&b.tree, Key::Enter).is_none());
        let action = dispatcher.character(&b.tree, 'a').unwrap();
        perform(&b, action);
        // The field shows its value, which the program has not changed.
        let action = dispatcher.character(&b.tree, 'é').unwrap();
        perform(&b, action);
        b.tree.set_text(field, "ab");
        let action = dispatcher.key(&b.tree, Key::Backspace).unwrap();
        perform(&b, action);
        b.tree.set_text(field, "aé");
        let action = dispatcher.key(&b.tree, Key::Backspace).unwrap();
        perform(&b, action);
        assert_eq!(*log.borrow(), ["a", "é", "a", "a"]);
        b.tree.set_enabled(field, false);
        assert!(
            dispatcher.character(&b.tree, 'x').is_none(),
            "a disabled field"
        );
    }

    #[test]
    fn a_released_focused_field_loses_the_focus() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut b = TreeBuilder::new(400, 300);
        let field = typing_app(&mut b, "", &log);
        lay_out(&mut b);
        let mut dispatcher = Dispatcher::new();
        click(&mut dispatcher, &b, 100, 18);
        let parent = b.tree.node(field).unwrap().parent.unwrap();
        let mut drops = Vec::new();
        b.tree.remove(parent, field);
        b.tree.release(field, &mut b.handlers, &mut drops);
        crate::handlers::release_functions(&mut drops);
        assert_eq!(dispatcher.focused(&b.tree), None);
        assert!(dispatcher.character(&b.tree, 'a').is_none());
    }

    /// D7 of `layout-primitive`: a node its parent does not place is not hit, nor is its
    /// subtree, even where it lies under the press.
    #[test]
    fn an_unplaced_button_is_not_hit() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut b = TreeBuilder::new(400, 300);
        let root = b.root();
        let node = b.add(root, NodeKind::Layout);
        b.set_policy(node, first_only_policy());
        b.text(node, "shown");
        let seen = log.clone();
        button(
            &mut b,
            node,
            "hidden",
            true,
            handler("hidden", move || seen.borrow_mut().push("hidden")),
        );
        lay_out(&mut b);
        let mut dispatcher = Dispatcher::new();
        assert!(click(&mut dispatcher, &b, 10, 10).is_none());
        assert!(hit_test(&b.tree, 10, 10).is_none());
        assert!(log.borrow().is_empty());
    }
}
