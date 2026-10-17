//! Dense arenas addressed by typed `u32` ids.
//!
//! Every node of a body lives in one of the body's arenas, and refers to its children by id, so
//! a node can be named by later stages (borrow-checker diagnostics, MIR source maps, the
//! language server) and wrapped after it is built by swapping an id (D3 of `typed-tree`). Ids
//! are allocated in checking order, so the tree of a file is deterministic.

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

/// An id addressing one element of an [`Arena`].
pub trait ArenaId: Copy + Eq + Ord + fmt::Debug {
    /// The id of the element at `index`.
    fn from_index(index: usize) -> Self;
    /// The position of the element in its arena.
    fn index(self) -> usize;
}

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u32);

        impl ArenaId for $name {
            fn from_index(index: usize) -> Self {
                Self(u32::try_from(index).expect("arena overflow"))
            }

            fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

define_id!(
    /// An expression of a body.
    ExpressionId
);
define_id!(
    /// A statement of a body.
    StatementId
);
define_id!(
    /// A binding of a body: a parameter, local, loop variable, closure parameter or `this`.
    LocalId
);
define_id!(
    /// A body of a file: the value of a function or of a top-level `val`.
    BodyId
);

impl fmt::Display for LocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.0)
    }
}

impl fmt::Display for StatementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A vector of `T` indexed by the id type `I`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arena<I, T> {
    items: Vec<T>,
    marker: PhantomData<I>,
}

impl<I: ArenaId, T> Arena<I, T> {
    /// An empty arena.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            marker: PhantomData,
        }
    }

    /// Adds `item`, returning its id.
    pub fn alloc(&mut self, item: T) -> I {
        let id = I::from_index(self.items.len());
        self.items.push(item);
        id
    }

    /// The element `id`, if it belongs to this arena.
    pub fn get(&self, id: I) -> Option<&T> {
        self.items.get(id.index())
    }

    /// Whether `id` belongs to this arena.
    pub fn contains(&self, id: I) -> bool {
        id.index() < self.items.len()
    }

    /// The number of elements.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the arena has no element.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The elements with their ids, in allocation order.
    pub fn iter(&self) -> impl Iterator<Item = (I, &T)> {
        self.items
            .iter()
            .enumerate()
            .map(|(index, item)| (I::from_index(index), item))
    }

    /// The ids, in allocation order.
    pub fn ids(&self) -> impl Iterator<Item = I> {
        (0..self.items.len()).map(I::from_index)
    }
}

impl<I: ArenaId, T> Default for Arena<I, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: ArenaId, T> Index<I> for Arena<I, T> {
    type Output = T;

    fn index(&self, id: I) -> &T {
        &self.items[id.index()]
    }
}

impl<I: ArenaId, T> IndexMut<I> for Arena<I, T> {
    fn index_mut(&mut self, id: I) -> &mut T {
        &mut self.items[id.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_dense_and_in_allocation_order() {
        let mut arena: Arena<ExpressionId, &str> = Arena::new();
        assert!(arena.is_empty());
        let a = arena.alloc("a");
        let b = arena.alloc("b");
        assert_eq!((a, b), (ExpressionId(0), ExpressionId(1)));
        assert_eq!(arena.len(), 2);
        assert_eq!(arena[b], "b");
        assert!(arena.contains(b));
        assert!(!arena.contains(ExpressionId(2)));
        assert_eq!(arena.get(ExpressionId(2)), None);
        let collected: Vec<_> = arena.iter().collect();
        assert_eq!(collected, vec![(a, &"a"), (b, &"b")]);
    }

    #[test]
    fn elements_can_be_replaced_in_place() {
        let mut arena: Arena<LocalId, i32> = Arena::new();
        let id = arena.alloc(1);
        arena[id] = 2;
        assert_eq!(arena[id], 2);
        assert_eq!(id.to_string(), "%0");
        assert_eq!(StatementId(3).to_string(), "#3");
    }
}
