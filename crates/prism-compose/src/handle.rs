//! Generational handles and the arena they index (D3 of `compose-runtime`).
//!
//! A [`Handle`] is one 64-bit word: a 32-bit index in its arena and the 32-bit generation of
//! the entry it names, the representation the state-handle invariant gives `State<T>`.
//! Removing an entry bumps its generation, so every handle to it becomes stale and validating
//! one is an index check and a compare. Generations start at 1, so the word 0 is never a
//! handle. An index whose generation reaches `u32::MAX` is retired, never reused, so a handle
//! can never come to name a newer entry.

/// A generational handle: a 32-bit index and a 32-bit generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(pub u64);

impl Handle {
    /// The handle of the entry at `index` of `generation`.
    pub fn new(index: u32, generation: u32) -> Handle {
        Handle(((generation as u64) << 32) | index as u64)
    }

    /// Its index in the arena.
    pub fn index(self) -> u32 {
        self.0 as u32
    }

    /// The generation of the entry it names.
    pub fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// The handle as the word generated code passes.
    pub fn word(self) -> u64 {
        self.0
    }
}

/// One entry of an arena: its current generation and its value while it is live.
#[derive(Debug)]
struct Entry<T> {
    generation: u32,
    value: Option<T>,
}

/// An arena of values named by generational handles, with a free list of indexes to reuse.
#[derive(Debug)]
pub struct Arena<T> {
    entries: Vec<Entry<T>>,
    free: Vec<u32>,
    live: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Arena<T> {
    pub const fn new() -> Self {
        Arena {
            entries: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }

    /// Inserts `value`, reusing a freed index when there is one.
    pub fn insert(&mut self, value: T) -> Handle {
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let entry = &mut self.entries[index as usize];
            entry.value = Some(value);
            return Handle::new(index, entry.generation);
        }
        let index = u32::try_from(self.entries.len()).expect("fewer than 2^32 entries");
        self.entries.push(Entry {
            generation: 1,
            value: Some(value),
        });
        Handle::new(index, 1)
    }

    /// The live value `handle` names, or `None` for a stale handle.
    pub fn get(&self, handle: Handle) -> Option<&T> {
        let entry = self.entries.get(handle.index() as usize)?;
        if entry.generation != handle.generation() {
            return None;
        }
        entry.value.as_ref()
    }

    /// The live value `handle` names, mutably.
    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        let entry = self.entries.get_mut(handle.index() as usize)?;
        if entry.generation != handle.generation() {
            return None;
        }
        entry.value.as_mut()
    }

    /// Whether `handle` names a live value.
    pub fn contains(&self, handle: Handle) -> bool {
        self.get(handle).is_some()
    }

    /// Removes the value `handle` names and makes every handle to it stale. The index is
    /// reused unless its generation reached the maximum, in which case it is retired.
    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        let entry = self.entries.get_mut(handle.index() as usize)?;
        if entry.generation != handle.generation() {
            return None;
        }
        let value = entry.value.take()?;
        self.live -= 1;
        if entry.generation < u32::MAX - 1 {
            entry.generation += 1;
            self.free.push(handle.index());
        } else {
            entry.generation = u32::MAX;
        }
        Some(value)
    }

    /// The live values with their handles, in index order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle, &T)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let value = entry.value.as_ref()?;
                Some((Handle::new(index as u32, entry.generation), value))
            })
    }

    /// The live values, mutably, in index order.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries
            .iter_mut()
            .filter_map(|entry| entry.value.as_mut())
    }

    /// How many values are live.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// Gives the next insertion at a fresh index the generation `generation`, so tests can
    /// reach the retirement of an index without 2^32 removals.
    #[cfg(test)]
    fn preset_generation(&mut self, index: u32, generation: u32) {
        self.entries[index as usize].generation = generation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_packs_index_and_generation() {
        let handle = Handle::new(7, 3);
        assert_eq!(handle.index(), 7);
        assert_eq!(handle.generation(), 3);
        assert_eq!(handle.word(), (3 << 32) | 7);
    }

    #[test]
    fn a_removed_handle_is_stale() {
        let mut arena = Arena::new();
        let a = arena.insert("a");
        assert_eq!(arena.get(a), Some(&"a"));
        assert_eq!(arena.remove(a), Some("a"));
        assert_eq!(arena.get(a), None);
        assert_eq!(arena.remove(a), None);
        assert!(arena.is_empty());
        // The word 0 is never a handle.
        assert_eq!(arena.get(Handle(0)), None);
    }

    #[test]
    fn a_reused_index_has_a_new_generation() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        arena.remove(a);
        let b = arena.insert(2);
        assert_eq!(b.index(), a.index());
        assert_ne!(b.generation(), a.generation());
        assert_eq!(arena.get(a), None);
        assert_eq!(arena.get(b), Some(&2));
        assert_eq!(arena.len(), 1);
    }

    #[test]
    fn a_retired_index_is_never_handed_out_again() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        arena.preset_generation(a.index(), u32::MAX - 1);
        let a = Handle::new(a.index(), u32::MAX - 1);
        assert_eq!(arena.remove(a), Some(1));
        // The index is retired: the next insertion takes a fresh one.
        let b = arena.insert(2);
        assert_ne!(b.index(), a.index());
        for _ in 0..10 {
            let c = arena.insert(3);
            assert_ne!(c.index(), a.index());
            arena.remove(c);
        }
        assert_eq!(arena.get(a), None);
        assert_eq!(arena.get(Handle::new(a.index(), u32::MAX)), None);
    }
}
