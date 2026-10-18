//! Generational handles of nodes and handlers, and the arena they index (D10 and D23 of
//! `ui-toolkit`).
//!
//! A [`NodeId`] and a [`HandlerId`] are one 64-bit word each, the word generated code passes:
//! a 32-bit index in their arena and the 32-bit generation of the entry they name, as
//! `prism-compose`'s handles are. Removing an entry bumps its generation, so every handle to it
//! becomes stale and validating one is an index check and a compare. Generations start at 1, so
//! the word 0 is never a handle. An index whose generation reaches `u32::MAX` is retired, never
//! reused, so a handle can never come to name a newer entry.
//!
//! The arena is this crate's own rather than `prism-compose`'s because the handler table walks
//! its live entries when an app ends.

/// A node of the UI tree: the state-handle invariant's `NodeId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u64);

/// An entry of the handler table, which a `Copy` modifier element names (D23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HandlerId(pub u64);

/// The index and generation packed in a handle word.
fn pack(index: u32, generation: u32) -> u64 {
    ((generation as u64) << 32) | index as u64
}

fn index_of(word: u64) -> u32 {
    word as u32
}

fn generation_of(word: u64) -> u32 {
    (word >> 32) as u32
}

impl NodeId {
    /// The word generated code passes.
    pub fn word(self) -> u64 {
        self.0
    }
}

impl HandlerId {
    /// The word generated code passes.
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

/// An arena of values named by generational handle words, with a free list of indexes to
/// reuse.
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

    /// Inserts `value`, reusing a freed index when there is one, and gives its handle word.
    pub fn insert(&mut self, value: T) -> u64 {
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let entry = &mut self.entries[index as usize];
            entry.value = Some(value);
            return pack(index, entry.generation);
        }
        let index = u32::try_from(self.entries.len()).expect("fewer than 2^32 entries");
        self.entries.push(Entry {
            generation: 1,
            value: Some(value),
        });
        pack(index, 1)
    }

    /// The live value `word` names, or `None` for a stale handle.
    pub fn get(&self, word: u64) -> Option<&T> {
        let entry = self.entries.get(index_of(word) as usize)?;
        if entry.generation != generation_of(word) {
            return None;
        }
        entry.value.as_ref()
    }

    /// The live value `word` names, mutably.
    pub fn get_mut(&mut self, word: u64) -> Option<&mut T> {
        let entry = self.entries.get_mut(index_of(word) as usize)?;
        if entry.generation != generation_of(word) {
            return None;
        }
        entry.value.as_mut()
    }

    /// Whether `word` names a live value.
    pub fn contains(&self, word: u64) -> bool {
        self.get(word).is_some()
    }

    /// Removes the value `word` names and makes every handle to it stale. The index is reused
    /// unless its generation reached the maximum, in which case it is retired.
    pub fn remove(&mut self, word: u64) -> Option<T> {
        let index = index_of(word);
        let entry = self.entries.get_mut(index as usize)?;
        if entry.generation != generation_of(word) {
            return None;
        }
        let value = entry.value.take()?;
        self.live -= 1;
        if entry.generation < u32::MAX - 1 {
            entry.generation += 1;
            self.free.push(index);
        } else {
            entry.generation = u32::MAX;
        }
        Some(value)
    }

    /// How many values are live.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// The handle word of every live value, in index order, into `out`.
    pub fn live_words(&self, out: &mut Vec<u64>) {
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.value.is_some() {
                out.push(pack(index as u32, entry.generation));
            }
        }
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
        let word = pack(7, 3);
        assert_eq!(index_of(word), 7);
        assert_eq!(generation_of(word), 3);
        assert_eq!(word, (3 << 32) | 7);
    }

    #[test]
    fn a_released_handle_is_stale() {
        let mut arena = Arena::new();
        let a = arena.insert("a");
        assert_eq!(arena.get(a), Some(&"a"));
        assert_eq!(arena.remove(a), Some("a"));
        assert_eq!(arena.get(a), None);
        assert_eq!(arena.remove(a), None);
        assert!(arena.is_empty());
        // The word 0 is never a handle.
        assert_eq!(arena.get(0), None);
    }

    #[test]
    fn a_reused_index_has_a_new_generation() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        arena.remove(a);
        let b = arena.insert(2);
        assert_eq!(index_of(b), index_of(a));
        assert_ne!(generation_of(b), generation_of(a));
        assert_eq!(arena.get(a), None);
        assert_eq!(arena.get(b), Some(&2));
        assert_eq!(arena.len(), 1);
        let mut words = Vec::new();
        arena.live_words(&mut words);
        assert_eq!(words, [b]);
    }

    #[test]
    fn a_retired_index_is_never_handed_out_again() {
        let mut arena = Arena::new();
        let a = arena.insert(1);
        arena.preset_generation(index_of(a), u32::MAX - 1);
        let a = pack(index_of(a), u32::MAX - 1);
        assert_eq!(arena.remove(a), Some(1));
        let b = arena.insert(2);
        assert_ne!(index_of(b), index_of(a));
        for _ in 0..10 {
            let c = arena.insert(3);
            assert_ne!(index_of(c), index_of(a));
            arena.remove(c);
        }
        assert_eq!(arena.get(a), None);
        assert_eq!(arena.get(pack(index_of(a), u32::MAX)), None);
    }
}
