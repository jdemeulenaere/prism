//! The handler table (D23 of `ui-toolkit`): the handlers of `clickable`, `toggleable` and text
//! fields, held by `prism-ui` so that a `Copy` modifier element can name one by a
//! [`HandlerId`].
//!
//! - **Registering** moves a function value into a free entry, which is *pending*.
//! - **Claiming** gives a pending entry to the node whose chain names it. A node naming an
//!   entry another node holds gets a duplicate when the handler is keepable, and the misuse
//!   `a handler that owns captures that are not Copy was given to two nodes` otherwise; naming
//!   a released entry is `stale modifier handler`.
//! - **Releasing** an entry takes its function value out of the table; the caller drops it
//!   once no state is borrowed (D2), since a drop entry is program code. Pending entries are
//!   released at the end of each pass, and every entry when the app ends.
//! - **Equality** of two ids: an id equals itself, and two live entries are equal when their
//!   handlers have the same code and equal captures.
//!
//! Every handler has one owner at a time and is dropped exactly once, without counting. The
//! table and its pending list are reused vectors, so a steady-state pass registering and
//! releasing handlers that capture one `Copy` word allocates nothing.

use crate::error::{UiResult, HANDLER_OF_ANOTHER_TYPE, SHARED_HANDLER, STALE_HANDLER};
use crate::handle::{Arena, HandlerId, NodeId};
use crate::runtime;
use prism_runtime::FunctionValue;

/// The type of a handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerKind {
    /// `() -> Unit`, of `clickable`.
    Click,
    /// `(Bool) -> Unit`, of `toggleable`.
    Toggle,
    /// `(String) -> Unit`, of a text field.
    Text,
}

/// Who holds an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Pending,
    Node(NodeId),
}

#[derive(Debug)]
struct Entry {
    function: FunctionValue,
    kind: HandlerKind,
    owner: Owner,
}

/// The handlers of one thread's app.
#[derive(Debug, Default)]
pub struct HandlerTable {
    entries: Arena<Entry>,
    /// The entries registered since the last end of a pass, some of them claimed since.
    pending: Vec<HandlerId>,
    /// Scratch for [`HandlerTable::release_all`].
    words: Vec<u64>,
}

impl HandlerTable {
    pub fn new() -> HandlerTable {
        HandlerTable::default()
    }

    /// How many entries are live.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Moves `function` into a new pending entry.
    pub fn register(&mut self, kind: HandlerKind, function: FunctionValue) -> HandlerId {
        let id = HandlerId(self.entries.insert(Entry {
            function,
            kind,
            owner: Owner::Pending,
        }));
        self.pending.push(id);
        id
    }

    /// Who holds `id`, or `None` when it was released.
    pub fn owner(&self, id: HandlerId) -> Option<Owner> {
        self.entries.get(id.0).map(|entry| entry.owner)
    }

    /// The kind of `id`, when live.
    pub fn kind(&self, id: HandlerId) -> Option<HandlerKind> {
        self.entries.get(id.0).map(|entry| entry.kind)
    }

    /// A bitwise copy of `id`'s function value, to call outside the table, which keeps owning
    /// it.
    pub fn function(&self, id: HandlerId) -> Option<FunctionValue> {
        self.entries
            .get(id.0)
            .map(|entry| runtime::borrowed(&entry.function))
    }

    /// Gives `id`, of `kind`, to `node`, and gives the id of the entry `node` holds from then
    /// on: `id` itself when it was pending, or a duplicate when another node holds it. The
    /// caller has checked that `node` does not already hold `id` at another position.
    pub fn claim(&mut self, id: HandlerId, kind: HandlerKind, node: NodeId) -> UiResult<HandlerId> {
        let Some(entry) = self.entries.get_mut(id.0) else {
            return Err(STALE_HANDLER);
        };
        if entry.kind != kind {
            return Err(HANDLER_OF_ANOTHER_TYPE);
        }
        match entry.owner {
            Owner::Pending => {
                entry.owner = Owner::Node(node);
                Ok(id)
            }
            Owner::Node(_) => {
                let Some(copy) = runtime::duplicate(&entry.function) else {
                    return Err(SHARED_HANDLER);
                };
                Ok(HandlerId(self.entries.insert(Entry {
                    function: copy,
                    kind,
                    owner: Owner::Node(node),
                })))
            }
        }
    }

    /// Releases `id`, if live, moving its function value to `drops`.
    pub fn release(&mut self, id: HandlerId, drops: &mut Vec<FunctionValue>) {
        if let Some(entry) = self.entries.remove(id.0) {
            drops.push(entry.function);
        }
    }

    /// Releases the entries still pending: what the end of a pass does.
    pub fn release_unclaimed(&mut self, drops: &mut Vec<FunctionValue>) {
        for index in 0..self.pending.len() {
            let id = self.pending[index];
            if self.owner(id) == Some(Owner::Pending) {
                self.release(id, drops);
            }
        }
        self.pending.clear();
    }

    /// Releases every entry: what the end of an app does.
    pub fn release_all(&mut self, drops: &mut Vec<FunctionValue>) {
        let mut words = std::mem::take(&mut self.words);
        self.entries.live_words(&mut words);
        for &word in &words {
            self.release(HandlerId(word), drops);
        }
        words.clear();
        self.words = words;
        self.pending.clear();
    }

    /// Whether `a` and `b` hold equal handlers (`prism_ui_handlers_equal`).
    pub fn equal(&self, a: HandlerId, b: HandlerId) -> bool {
        if a == b {
            return true;
        }
        match (self.entries.get(a.0), self.entries.get(b.0)) {
            (Some(a), Some(b)) => {
                a.kind == b.kind && runtime::environments_equal(&a.function, &b.function)
            }
            _ => false,
        }
    }
}

/// Drops the function values in `drops`, emptying it and keeping its capacity: what every
/// caller does once it no longer borrows any state.
pub fn release_functions(drops: &mut Vec<FunctionValue>) {
    for function in drops.drain(..) {
        // SAFETY: every value in `drops` was taken out of the table, which owned it, and is
        // dropped once.
        unsafe { function.release() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{drops, handler, inline_handler, reset_drops};

    fn node(n: u64) -> NodeId {
        NodeId(n | (1 << 32))
    }

    fn released(
        table: &mut HandlerTable,
        f: impl FnOnce(&mut HandlerTable, &mut Vec<FunctionValue>),
    ) {
        let mut out = Vec::new();
        f(table, &mut out);
        release_functions(&mut out);
    }

    #[test]
    fn a_pending_entry_is_claimed_and_kept() {
        reset_drops();
        let mut table = HandlerTable::new();
        let id = table.register(HandlerKind::Click, handler("a", || {}));
        assert_eq!(table.owner(id), Some(Owner::Pending));
        assert_eq!(table.claim(id, HandlerKind::Click, node(1)), Ok(id));
        assert_eq!(table.owner(id), Some(Owner::Node(node(1))));
        released(&mut table, |table, out| table.release_unclaimed(out));
        assert_eq!(table.len(), 1, "a claimed entry outlives its pass");
        assert!(drops().is_empty());
        released(&mut table, |table, out| table.release(id, out));
        assert_eq!(drops(), ["drop a"]);
        assert_eq!(
            table.claim(id, HandlerKind::Click, node(1)),
            Err(STALE_HANDLER)
        );
    }

    #[test]
    fn unclaimed_entries_are_released_at_the_end_of_the_pass() {
        reset_drops();
        let mut table = HandlerTable::new();
        let a = table.register(HandlerKind::Click, handler("a", || {}));
        let b = table.register(HandlerKind::Click, handler("b", || {}));
        table.claim(b, HandlerKind::Click, node(1)).unwrap();
        released(&mut table, |table, out| table.release_unclaimed(out));
        assert_eq!(drops(), ["drop a"]);
        assert_eq!(table.owner(a), None);
        // One registered between passes is released at the end of the next.
        let c = table.register(HandlerKind::Click, handler("c", || {}));
        assert_eq!(table.owner(c), Some(Owner::Pending));
        released(&mut table, |table, out| table.release_unclaimed(out));
        assert_eq!(drops(), ["drop a", "drop c"]);
        released(&mut table, |table, out| table.release_all(out));
        assert_eq!(drops(), ["drop a", "drop c", "drop b"]);
        assert!(table.is_empty());
    }

    #[test]
    fn a_second_node_gets_a_duplicate_of_a_keepable_handler() {
        let mut table = HandlerTable::new();
        let id = table.register(HandlerKind::Click, inline_handler(5));
        table.claim(id, HandlerKind::Click, node(1)).unwrap();
        let copy = table.claim(id, HandlerKind::Click, node(2)).unwrap();
        assert_ne!(copy, id);
        assert_eq!(table.owner(copy), Some(Owner::Node(node(2))));
        assert!(table.equal(id, copy));
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn a_handler_owning_captures_cannot_be_shared() {
        reset_drops();
        let mut table = HandlerTable::new();
        let id = table.register(HandlerKind::Click, handler("a", || {}));
        table.claim(id, HandlerKind::Click, node(1)).unwrap();
        assert_eq!(
            table.claim(id, HandlerKind::Click, node(2)),
            Err(SHARED_HANDLER)
        );
        assert_eq!(
            table.claim(id, HandlerKind::Toggle, node(1)),
            Err(HANDLER_OF_ANOTHER_TYPE)
        );
        released(&mut table, |table, out| table.release_all(out));
    }

    #[test]
    fn equality_compares_code_and_captures() {
        reset_drops();
        let mut table = HandlerTable::new();
        let a = table.register(HandlerKind::Click, inline_handler(1));
        let b = table.register(HandlerKind::Click, inline_handler(1));
        let c = table.register(HandlerKind::Click, inline_handler(2));
        let owning = table.register(HandlerKind::Click, handler("x", || {}));
        assert!(table.equal(a, a));
        assert!(table.equal(a, b));
        assert!(!table.equal(a, c));
        assert!(!table.equal(owning, a));
        assert!(table.equal(owning, owning), "an id equals itself");
        released(&mut table, |table, out| table.release(b, out));
        assert!(!table.equal(a, b), "a released entry is unequal");
        released(&mut table, |table, out| table.release_all(out));
        assert_eq!(drops(), ["drop x"]);
    }
}
