//! The handler table (D23 of `ui-toolkit`): the handlers of `clickable`, `toggleable` and text
//! fields, and the measure policies of layout nodes (D8 of `layout-primitive`), held by `prism-ui` so that a `Copy` modifier element can name one by a
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
//!   handlers have the same code and equal captures. The environment equality entry is program
//!   code, so callers holding the thread's state copy the two function values out
//!   ([`HandlerTable::comparable`]) and compare them after letting the state go (D2): an
//!   equality entry comparing a captured `Modifier` calls `prism_ui_handlers_equal` again.
//! - **Interning.** A handler whose environment is a single word that owns nothing (a keepable
//!   closure capturing nothing or one `Copy` word, with no drop and no duplicate entry) is the
//!   same value as any other handler with its code and word. Registering one while an equal
//!   entry is live gives that entry's id instead of a new one, so equal handlers built on two
//!   passes are one id, and every slot and node naming them names a live entry.
//!
//! Every handler has one owner at a time and is dropped exactly once, without counting. The
//! table and its pending list are reused vectors, so a steady-state pass registering and
//! releasing handlers that capture one `Copy` word allocates nothing.

use crate::error::{UiResult, HANDLER_OF_ANOTHER_TYPE, SHARED_HANDLER, STALE_HANDLER};
use crate::handle::{Arena, HandlerId, NodeId};
use crate::runtime;
use prism_runtime::FunctionValue;
use std::collections::HashMap;

/// The type of a handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandlerKind {
    /// `() -> Unit`, of `clickable`.
    Click,
    /// `(Bool) -> Unit`, of `toggleable`.
    Toggle,
    /// `(String) -> Unit`, of a text field.
    Text,
    /// `MeasureScope.(&List<Measurable>, Constraints) -> MeasureResult`, a node's measure
    /// policy (D8 of `layout-primitive`).
    Measure,
    /// `MeasureScope.(Measurable, Constraints) -> MeasureResult`, the step of a
    /// `Modifier.layout` (D3 of `layout-in-prism`).
    LayoutModifier,
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

/// A bitwise copy of a live entry's kind and function value, to compare outside the thread's
/// state; the table keeps owning the function value.
#[derive(Debug)]
pub struct Comparable {
    kind: HandlerKind,
    function: FunctionValue,
}

impl Comparable {
    /// Whether two entries hold equal handlers: the same kind, code and captures. Runs the
    /// environment equality entry, program code, so no thread state may be borrowed.
    pub fn equal(a: Option<&Comparable>, b: Option<&Comparable>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => {
                a.kind == b.kind && runtime::environments_equal(&a.function, &b.function)
            }
            _ => false,
        }
    }

    /// Whether this entry's handler equals `function`, a handler of `kind`.
    pub fn equals_function(&self, kind: HandlerKind, function: &FunctionValue) -> bool {
        self.kind == kind && runtime::environments_equal(&self.function, function)
    }
}

/// What an interned handler is known by: its kind, descriptor and environment word.
type InternKey = (HandlerKind, usize, usize);

/// The handlers of one thread's app.
#[derive(Debug, Default)]
pub struct HandlerTable {
    entries: Arena<Entry>,
    /// The entries registered since the last end of a pass, some of them claimed since.
    pending: Vec<HandlerId>,
    /// The live entries of handlers whose environment is a word owning nothing, by kind, code
    /// and word: the entry a registration of an equal handler gives instead of a new one.
    interned: HashMap<InternKey, HandlerId>,
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

    /// Moves `function` into a new pending entry, interned when its environment is a word that
    /// owns nothing. Callers that may hold an equal interned entry ask
    /// [`HandlerTable::interned`] first.
    pub fn register(&mut self, kind: HandlerKind, function: FunctionValue) -> HandlerId {
        let key = runtime::intern_key(&function).map(|(code, word)| (kind, code, word));
        let id = HandlerId(self.entries.insert(Entry {
            function,
            kind,
            owner: Owner::Pending,
        }));
        self.pending.push(id);
        if let Some(key) = key {
            self.interned.entry(key).or_insert(id);
        }
        id
    }

    /// The live interned entry with `function`'s kind, code and word, if any, with a copy of
    /// its handler to compare with `function` outside the state: when they are equal, a
    /// registration of `function` gives that entry instead (and drops `function`, which owns
    /// nothing).
    pub fn interned(
        &self,
        kind: HandlerKind,
        function: &FunctionValue,
    ) -> Option<(HandlerId, Comparable)> {
        let (code, word) = runtime::intern_key(function)?;
        let id = *self.interned.get(&(kind, code, word))?;
        Some((id, self.comparable(id)?))
    }

    /// A copy of `id`'s kind and handler, when live, to compare outside the state.
    pub fn comparable(&self, id: HandlerId) -> Option<Comparable> {
        self.entries.get(id.0).map(|entry| Comparable {
            kind: entry.kind,
            function: runtime::borrowed(&entry.function),
        })
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
            if let Some((code, word)) = runtime::intern_key(&entry.function) {
                let key = (entry.kind, code, word);
                if self.interned.get(&key) == Some(&id) {
                    self.interned.remove(&key);
                }
            }
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
        self.interned.clear();
    }

    /// Whether `a` and `b` hold equal handlers. It runs environment equality entries with the
    /// table borrowed, so only callers that hold no thread state use it (Rust tests and their
    /// hand-built handlers); `prism_ui_handlers_equal` compares [`Comparable`]s instead.
    pub fn equal(&self, a: HandlerId, b: HandlerId) -> bool {
        a == b || Comparable::equal(self.comparable(a).as_ref(), self.comparable(b).as_ref())
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
