//! A dataflow framework over a body's control-flow graph (D4 of `borrowck-moves`).
//!
//! An [`Analysis`] names its states (a [`Domain`], such as bits over move paths or move
//! events), a direction, the state at the boundary of the body, and the effect of each
//! statement and terminator. [`solve`] iterates the blocks (in reverse postorder forward,
//! postorder backward) joining states until no block's state changes, and a [`ResultsCursor`]
//! replays a block's effects to give the state at any location. `borrowck-exclusivity` adds its
//! liveness (backward) and borrow analyses on the same framework.
//!
//! Its cost is linear in the body's locations times the cost of copying, joining and changing
//! a state, per iteration: a forward analysis stores a state only for the blocks that are not
//! chained to their predecessor ([`Results`]), and a backward cursor visited in program order
//! keeps the states of its current block instead of replaying the block for each location.
//! The states keep that cost small in large bodies: a [`BitSet`] holds few bits as a list and
//! many in chunks shared between copies, a [`PairSet`] holds its pairs in shared chunks, and
//! the solver walks the edges from a flat [`Graph`] read once from the body.

use crate::location::{Graph, Location};
use prism_mir::{BlockId, Body, Statement, Terminator};
use prism_typed_tree::ArenaId;
use std::rc::Rc;

/// The words of one chunk of a [`BitSet`].
const CHUNK_WORDS: usize = 32;
/// The bits of one chunk of a [`BitSet`].
const CHUNK_BITS: usize = CHUNK_WORDS * 64;

/// One chunk of a [`BitSet`]: `CHUNK_BITS` bits, fewer for the last one. A chunk with no bit
/// set is always `Zeros` and one with every bit set always `Ones`, so equal sets have equal
/// chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Chunk {
    Zeros,
    Ones,
    /// Some bits, with their number, shared between the copies of a state until one of them
    /// changes it.
    Mixed(u16, Rc<[u64; CHUNK_WORDS]>),
}

/// The words of a chunk of `size` bits with every bit set.
fn full_words(size: usize) -> [u64; CHUNK_WORDS] {
    let mut words = [0; CHUNK_WORDS];
    for (index, word) in words.iter_mut().enumerate() {
        let bits = size.saturating_sub(index * 64).min(64);
        *word = if bits == 64 {
            u64::MAX
        } else {
            (1 << bits) - 1
        };
    }
    words
}

/// The bits a [`BitSet`] holds as a sorted list before it switches to chunks.
const SPARSE_BITS: usize = 32;

/// A fixed-size set of small integers: a sorted list of at most `SPARSE_BITS` bits, or chunks
/// of `CHUNK_BITS` bits, each empty, full or shared copy-on-write (rustc's `HybridBitSet` and
/// `ChunkedBitSet`). Copying a small set copies its few bits, and copying a large one copies one
/// word per chunk while changing it copies only the chunks it changes: so the states that an
/// analysis stores per block and its cursors copy per location cost little even over the many
/// locals or events of a large body, where most states hold few bits or share most chunks.
#[derive(Debug, Clone)]
pub struct BitSet {
    bits: Bits,
    size: usize,
}

#[derive(Debug, Clone)]
enum Bits {
    /// At most `SPARSE_BITS` bits, sorted.
    Sparse(Vec<u32>),
    Chunked(Vec<Chunk>),
}

impl PartialEq for BitSet {
    fn eq(&self, other: &Self) -> bool {
        self.size == other.size && self.iter().eq(other.iter())
    }
}

impl Eq for BitSet {}

impl BitSet {
    /// The empty set over `0..size`.
    pub fn new(size: usize) -> Self {
        Self {
            bits: Bits::Sparse(Vec::new()),
            size,
        }
    }

    /// The size of the domain.
    pub fn domain_size(&self) -> usize {
        self.size
    }

    /// The number of bits of chunk `chunk`.
    fn chunk_size(&self, chunk: usize) -> usize {
        (self.size - chunk * CHUNK_BITS).min(CHUNK_BITS)
    }

    fn check_bit(&self, bit: usize) {
        assert!(
            bit < self.size,
            "bit {} out of a domain of {}",
            bit,
            self.size
        );
    }

    /// Switches a sparse set to chunks.
    fn make_chunked(&mut self) -> &mut Vec<Chunk> {
        if let Bits::Sparse(bits) = &mut self.bits {
            let bits = std::mem::take(bits);
            self.bits = Bits::Chunked(vec![Chunk::Zeros; self.size.div_ceil(CHUNK_BITS)]);
            for bit in bits {
                self.insert_chunked(bit as usize);
            }
        }
        match &mut self.bits {
            Bits::Chunked(chunks) => chunks,
            Bits::Sparse(_) => unreachable!("the set was switched to chunks"),
        }
    }

    /// Adds `bit`, returning whether it was absent.
    pub fn insert(&mut self, bit: usize) -> bool {
        self.check_bit(bit);
        if let Bits::Sparse(bits) = &mut self.bits {
            let Err(position) = bits.binary_search(&(bit as u32)) else {
                return false;
            };
            if bits.len() < SPARSE_BITS {
                bits.insert(position, bit as u32);
                return true;
            }
            self.make_chunked();
        }
        self.insert_chunked(bit)
    }

    /// Adds `bit` to a chunked set, returning whether it was absent.
    fn insert_chunked(&mut self, bit: usize) -> bool {
        let (index, offset) = (bit / CHUNK_BITS, bit % CHUNK_BITS);
        let size = self.chunk_size(index);
        let (word, mask) = (offset / 64, 1u64 << (offset % 64));
        let Bits::Chunked(chunks) = &mut self.bits else {
            unreachable!("a chunked set")
        };
        let chunk = &mut chunks[index];
        match chunk {
            Chunk::Ones => false,
            Chunk::Zeros if size == 1 => {
                *chunk = Chunk::Ones;
                true
            }
            Chunk::Zeros => {
                let mut words = [0; CHUNK_WORDS];
                words[word] = mask;
                *chunk = Chunk::Mixed(1, Rc::new(words));
                true
            }
            Chunk::Mixed(count, words) => {
                if words[word] & mask != 0 {
                    return false;
                }
                if *count as usize + 1 == size {
                    *chunk = Chunk::Ones;
                } else {
                    Rc::make_mut(words)[word] |= mask;
                    *count += 1;
                }
                true
            }
        }
    }

    /// Removes `bit`, returning whether it was present.
    pub fn remove(&mut self, bit: usize) -> bool {
        self.check_bit(bit);
        let (index, offset) = (bit / CHUNK_BITS, bit % CHUNK_BITS);
        let size = self.chunk_size(index);
        let (word, mask) = (offset / 64, 1u64 << (offset % 64));
        let chunks = match &mut self.bits {
            Bits::Sparse(bits) => {
                let Ok(position) = bits.binary_search(&(bit as u32)) else {
                    return false;
                };
                bits.remove(position);
                return true;
            }
            Bits::Chunked(chunks) => chunks,
        };
        let chunk = &mut chunks[index];
        match chunk {
            Chunk::Zeros => false,
            Chunk::Ones if size == 1 => {
                *chunk = Chunk::Zeros;
                true
            }
            Chunk::Ones => {
                let mut words = full_words(size);
                words[word] &= !mask;
                *chunk = Chunk::Mixed((size - 1) as u16, Rc::new(words));
                true
            }
            Chunk::Mixed(count, words) => {
                if words[word] & mask == 0 {
                    return false;
                }
                if *count == 1 {
                    *chunk = Chunk::Zeros;
                } else {
                    Rc::make_mut(words)[word] &= !mask;
                    *count -= 1;
                }
                true
            }
        }
    }

    pub fn contains(&self, bit: usize) -> bool {
        if bit >= self.size {
            return false;
        }
        let chunks = match &self.bits {
            Bits::Sparse(bits) => return bits.binary_search(&(bit as u32)).is_ok(),
            Bits::Chunked(chunks) => chunks,
        };
        let offset = bit % CHUNK_BITS;
        match &chunks[bit / CHUNK_BITS] {
            Chunk::Zeros => false,
            Chunk::Ones => true,
            Chunk::Mixed(_, words) => words[offset / 64] & (1u64 << (offset % 64)) != 0,
        }
    }

    /// Adds every bit of `other`, returning whether this set changed.
    pub fn union(&mut self, other: &BitSet) -> bool {
        assert_eq!(
            self.size, other.size,
            "union of sets over different domains"
        );
        let other_chunks = match &other.bits {
            Bits::Sparse(bits) => {
                let mut changed = false;
                for bit in bits {
                    changed |= self.insert(*bit as usize);
                }
                return changed;
            }
            Bits::Chunked(chunks) => chunks,
        };
        if let Bits::Sparse(bits) = &self.bits {
            // The other set's chunks, with these few bits added.
            let mut joined = other.clone();
            for bit in bits {
                joined.insert_chunked(*bit as usize);
            }
            let changed = joined.chunked_len() != bits.len();
            *self = joined;
            return changed;
        }
        let size = self.size;
        let chunks = self.make_chunked();
        let mut changed = false;
        for (index, (chunk, other)) in chunks.iter_mut().zip(other_chunks).enumerate() {
            match (&mut *chunk, other) {
                (Chunk::Ones, _) | (_, Chunk::Zeros) => {}
                (Chunk::Zeros, _) | (_, Chunk::Ones) => {
                    *chunk = other.clone();
                    changed = true;
                }
                (Chunk::Mixed(count, words), Chunk::Mixed(_, others)) => {
                    if Rc::ptr_eq(words, others)
                        || words.iter().zip(others.iter()).all(|(a, b)| b & !a == 0)
                    {
                        continue;
                    }
                    let joined = Rc::make_mut(words);
                    for (word, other) in joined.iter_mut().zip(others.iter()) {
                        *word |= other;
                    }
                    let total: u32 = joined.iter().map(|word| word.count_ones()).sum();
                    if total as usize == (size - index * CHUNK_BITS).min(CHUNK_BITS) {
                        *chunk = Chunk::Ones;
                    } else {
                        *count = total as u16;
                    }
                    changed = true;
                }
            }
        }
        changed
    }

    /// The number of bits in the set.
    pub fn len(&self) -> usize {
        match &self.bits {
            Bits::Sparse(bits) => bits.len(),
            Bits::Chunked(_) => self.chunked_len(),
        }
    }

    /// The number of bits of a chunked set, counted per chunk.
    fn chunked_len(&self) -> usize {
        let Bits::Chunked(chunks) = &self.bits else {
            unreachable!("a chunked set")
        };
        chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| match chunk {
                Chunk::Zeros => 0,
                Chunk::Ones => self.chunk_size(index),
                Chunk::Mixed(count, _) => *count as usize,
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        match &self.bits {
            Bits::Sparse(bits) => bits.is_empty(),
            Bits::Chunked(chunks) => chunks.iter().all(|chunk| *chunk == Chunk::Zeros),
        }
    }

    /// Removes every bit.
    pub fn clear(&mut self) {
        self.bits = Bits::Sparse(Vec::new());
    }

    /// The bits of the set, in increasing order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        let (sparse, chunks): (&[u32], &[Chunk]) = match &self.bits {
            Bits::Sparse(bits) => (bits, &[]),
            Bits::Chunked(chunks) => (&[], chunks),
        };
        sparse
            .iter()
            .map(|bit| *bit as usize)
            .chain(chunks.iter().enumerate().flat_map(move |(index, chunk)| {
                let start = index * CHUNK_BITS;
                let size = self.chunk_size(index);
                let bits: Box<dyn Iterator<Item = usize>> = match chunk {
                    Chunk::Zeros => Box::new(std::iter::empty()),
                    Chunk::Ones => Box::new(start..start + size),
                    Chunk::Mixed(_, words) => Box::new(
                        words
                            .iter()
                            .enumerate()
                            .flat_map(move |(word, bits)| set_bits(*bits, start + word * 64)),
                    ),
                };
                bits
            }))
    }
}

/// The positions of the bits set in `word`, offset by `start`.
fn set_bits(mut word: u64, start: usize) -> impl Iterator<Item = usize> {
    std::iter::from_fn(move || {
        if word == 0 {
            return None;
        }
        let bit = word.trailing_zeros() as usize;
        word &= word - 1;
        Some(start + bit)
    })
}

/// The pairs a chunk of a [`PairSet`] holds before it is split in two.
const PAIR_CHUNK: usize = 128;

/// A set of pairs of small integers, sorted, in chunks of at most `PAIR_CHUNK` pairs shared
/// copy-on-write between the copies of a state, as [`BitSet`]'s chunks are. Copying a set copies
/// one pointer per chunk, adding or removing a pair copies at most one chunk, and joining two
/// states that share most of their chunks, such as the two branches of an `if` over the state
/// before it, skips the shared ones: so a state of `n` pairs costs `n / PAIR_CHUNK` to copy or
/// join, and `PAIR_CHUNK + n / PAIR_CHUNK` to change.
#[derive(Debug, Clone, Default)]
pub struct PairSet {
    /// Non-empty chunks, each sorted, every pair of a chunk before every pair of the next.
    chunks: Vec<Rc<Vec<(u32, u32)>>>,
    /// The last pair of each chunk, side by side, so that finding a pair's chunk searches one
    /// array rather than following a pointer per step.
    lasts: Vec<(u32, u32)>,
    len: usize,
}

impl PartialEq for PairSet {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}

impl Eq for PairSet {}

impl PairSet {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The pairs, in increasing order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.chunks.iter().flat_map(|chunk| chunk.iter().copied())
    }

    /// The chunk where `pair` is or would be inserted: the first whose last pair is not below
    /// it, or the last chunk.
    fn chunk_of(&self, pair: (u32, u32)) -> usize {
        self.lasts
            .partition_point(|last| *last < pair)
            .min(self.chunks.len().saturating_sub(1))
    }

    pub fn contains(&self, pair: (u32, u32)) -> bool {
        self.chunks
            .get(self.chunk_of(pair))
            .is_some_and(|chunk| chunk.binary_search(&pair).is_ok())
    }

    /// The pairs whose first element is `first`, in increasing order.
    pub fn with_first(&self, first: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        // The first pair not below `(first, 0)` is found by binary search, in the chunk and
        // within it, so a lookup costs `log n` whatever the chunk holds.
        let start = self.chunk_of((first, 0)).min(self.chunks.len());
        let offset = self
            .chunks
            .get(start)
            .map_or(0, |chunk| chunk.partition_point(|pair| pair.0 < first));
        self.chunks[start..]
            .iter()
            .flat_map(|chunk| chunk.iter().copied())
            .skip(offset)
            .take_while(move |pair| pair.0 == first)
    }

    /// Adds `pair`, returning whether it was absent.
    pub fn insert(&mut self, pair: (u32, u32)) -> bool {
        if self.chunks.is_empty() {
            self.chunks.push(Rc::new(vec![pair]));
            self.lasts.push(pair);
            self.len = 1;
            return true;
        }
        let index = self.chunk_of(pair);
        let Err(position) = self.chunks[index].binary_search(&pair) else {
            return false;
        };
        let chunk = Rc::make_mut(&mut self.chunks[index]);
        chunk.insert(position, pair);
        if position + 1 == chunk.len() {
            self.lasts[index] = pair;
        }
        if chunk.len() > PAIR_CHUNK {
            let upper = chunk.split_off(chunk.len() / 2);
            let (lower_last, upper_last) = (
                *chunk.last().expect("a non-empty chunk"),
                *upper.last().expect("a non-empty chunk"),
            );
            self.chunks.insert(index + 1, Rc::new(upper));
            self.lasts[index] = lower_last;
            self.lasts.insert(index + 1, upper_last);
        }
        self.len += 1;
        true
    }

    /// Removes `pair`, returning whether it was present.
    pub fn remove(&mut self, pair: (u32, u32)) -> bool {
        if self.chunks.is_empty() {
            return false;
        }
        let index = self.chunk_of(pair);
        let Ok(position) = self.chunks[index].binary_search(&pair) else {
            return false;
        };
        let chunk = Rc::make_mut(&mut self.chunks[index]);
        chunk.remove(position);
        let small = chunk.len();
        if let Some(last) = chunk.last() {
            self.lasts[index] = *last;
        }
        self.len -= 1;
        if small == 0 {
            self.chunks.remove(index);
            self.lasts.remove(index);
        } else if small < PAIR_CHUNK / 4 && index + 1 < self.chunks.len() {
            // A small chunk joins the next one when both fit in one, so that a set that shrank
            // keeps about `len / PAIR_CHUNK` chunks.
            if small + self.chunks[index + 1].len() <= PAIR_CHUNK {
                let next = self.chunks.remove(index + 1);
                Rc::make_mut(&mut self.chunks[index]).extend(next.iter().copied());
                self.lasts[index] = self.lasts.remove(index + 1);
            }
        }
        true
    }

    /// Keeps only the pairs for which `keep` holds.
    pub fn retain(&mut self, keep: impl Fn(&(u32, u32)) -> bool) {
        if self.iter().all(|pair| keep(&pair)) {
            return;
        }
        let kept: Vec<(u32, u32)> = self.iter().filter(|pair| keep(pair)).collect();
        *self = Self::from_sorted(kept);
    }

    /// The set of `pairs`, sorted and without duplicates.
    fn from_sorted(pairs: Vec<(u32, u32)>) -> Self {
        let len = pairs.len();
        let chunks: Vec<Rc<Vec<(u32, u32)>>> = pairs
            .chunks(PAIR_CHUNK / 2)
            .map(|chunk| Rc::new(chunk.to_vec()))
            .collect();
        let lasts = chunks
            .iter()
            .map(|chunk| *chunk.last().expect("a non-empty chunk"))
            .collect();
        Self { chunks, lasts, len }
    }

    /// Adds every pair of `other`, returning whether this set changed. The chunks of `other`
    /// that this set shares are skipped; when it shares few, the two are merged in one pass.
    pub fn union(&mut self, other: &PairSet) -> bool {
        if other.is_empty() {
            return false;
        }
        if self.is_empty() {
            *self = other.clone();
            return true;
        }
        let shared: std::collections::HashSet<*const Vec<(u32, u32)>> =
            self.chunks.iter().map(Rc::as_ptr).collect();
        let unshared: Vec<&Rc<Vec<(u32, u32)>>> = other
            .chunks
            .iter()
            .filter(|chunk| !shared.contains(&Rc::as_ptr(chunk)))
            .collect();
        let missing: Vec<(u32, u32)> = unshared
            .iter()
            .flat_map(|chunk| chunk.iter().copied())
            .filter(|pair| !self.contains(*pair))
            .collect();
        if missing.is_empty() {
            return false;
        }
        // A few new pairs go in one by one, which keeps the other chunks shared.
        if missing.len() * 8 <= self.len {
            for pair in missing {
                self.insert(pair);
            }
            return true;
        }
        let mut merged = Vec::with_capacity(self.len + other.len);
        {
            let (mut left, mut right) = (self.iter().peekable(), other.iter().peekable());
            loop {
                let next = match (left.peek(), right.peek()) {
                    (Some(a), Some(b)) if a < b => left.next(),
                    (Some(a), Some(b)) if b < a => right.next(),
                    (Some(_), Some(_)) => {
                        right.next();
                        left.next()
                    }
                    (Some(_), None) => left.next(),
                    (None, Some(_)) => right.next(),
                    (None, None) => break,
                };
                merged.extend(next);
            }
        }
        *self = Self::from_sorted(merged);
        true
    }
}

/// Whether an analysis follows the edges of the control-flow graph or runs against them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// The states of an analysis, joined where control flow merges.
pub trait Domain: Clone + PartialEq {
    /// Adds everything `other` holds, returning whether this state changed.
    fn join(&mut self, other: &Self) -> bool;
}

impl Domain for BitSet {
    fn join(&mut self, other: &Self) -> bool {
        self.union(other)
    }
}

/// A dataflow analysis over the states [`Analysis::Domain`] of one body, such as the bits
/// `0..n` of a [`BitSet`], joined by union.
pub trait Analysis {
    /// The analysis's states.
    type Domain: Domain;

    /// The state that holds nothing, where the analysis starts at every block but the boundary.
    fn bottom(&self) -> Self::Domain;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    /// Sets the state at the boundary: the entry of `bb0` forward, the exit of each block without
    /// successors backward.
    fn initialize_boundary(&self, state: &mut Self::Domain);

    /// Applies the effect of `statement`, at `location`.
    fn apply_statement(&self, state: &mut Self::Domain, statement: &Statement, location: Location);

    /// Applies the effect of `terminator`, at `location`. Forward, it holds on every outgoing
    /// edge.
    fn apply_terminator(
        &self,
        state: &mut Self::Domain,
        terminator: &Terminator,
        location: Location,
    );

    /// Applies the effect of the edge `from -> to` to the state leaving `from`, forward only,
    /// for an analysis that tells the successors of a switch apart. None by default.
    fn apply_edge(&self, _state: &mut Self::Domain, _from: BlockId, _to: BlockId) {}

    /// Whether [`Analysis::apply_edge`] changes states, so that the solver copies the state
    /// leaving a block for each of its edges. False by default.
    fn has_edge_effects(&self) -> bool {
        false
    }
}

/// The fixpoint of an analysis: the state where the analysis enters each block (its start
/// forward, its end backward).
///
/// A block whose state is another block's exit state in the analysis's direction is *chained*
/// to that block, its *source*: forward, a block whose only predecessor has it as only
/// successor is chained to that predecessor; backward, a block whose only successor has it as
/// only predecessor is chained to that successor. The solver and the cursors carry the state
/// from a source to the block chained to it instead of storing it, so that a long
/// straight-line body (whose calls each end a block) stores one state, not one per block, and
/// a state of many bits is not copied and joined at every block.
pub struct Results<A: Analysis> {
    pub analysis: A,
    /// The state where the analysis enters each block that is not chained; `None` for a
    /// chained block.
    block_states: Vec<Option<A::Domain>>,
    /// The source each chained block is chained to.
    chained_from: Vec<Option<BlockId>>,
}

/// Solves `analysis` over every edge of `body`.
pub fn solve<A: Analysis>(body: &Body, analysis: A) -> Results<A> {
    solve_with(body, analysis, |_, _| true)
}

/// The predecessor each reachable block of `body` other than `bb0` is chained to, when the
/// edge between them is its only way in, its predecessor's only way out, and kept: the order
/// visits a chained block right after its predecessor.
fn chains(
    graph: &Graph,
    order: &[BlockId],
    keep_edge: &impl Fn(BlockId, BlockId) -> bool,
) -> Vec<Option<BlockId>> {
    let mut chained_from = vec![None; graph.block_count()];
    for &block in order {
        if block.index() == 0 {
            continue;
        }
        if let [predecessor] = *graph.predecessors(block) {
            let single_exit = graph.successors(predecessor).len() == 1;
            if predecessor != block && single_exit && keep_edge(predecessor, block) {
                chained_from[block.index()] = Some(predecessor);
            }
        }
    }
    chained_from
}

/// Backward: the successor each reachable block of `body` is chained to, when the edge between
/// them is its only way out, its successor's only way in, and kept, and the successor is not
/// `bb0`. A depth-first search reaches such a successor only through the block, and finishes
/// the block right after it, so the postorder visits a chained block right after its source.
fn backward_chains(
    graph: &Graph,
    order: &[BlockId],
    keep_edge: &impl Fn(BlockId, BlockId) -> bool,
) -> Vec<Option<BlockId>> {
    let mut chained_from = vec![None; graph.block_count()];
    for &block in order {
        if let [successor] = *graph.successors(block) {
            let single_entry = graph.predecessors(successor).len() == 1;
            if successor != block
                && successor.index() != 0
                && single_entry
                && keep_edge(block, successor)
            {
                chained_from[block.index()] = Some(successor);
            }
        }
    }
    chained_from
}

/// The stored state of `block`, which is not chained.
fn stored<D>(states: &mut [Option<D>], block: BlockId) -> &mut D {
    states[block.index()]
        .as_mut()
        .expect("the state of a block that is not chained")
}

/// Solves `analysis` over the edges `from -> to` of `body` for which `keep_edge` holds, such as
/// every edge but the back edges.
pub fn solve_with<A: Analysis>(
    body: &Body,
    analysis: A,
    keep_edge: impl Fn(BlockId, BlockId) -> bool,
) -> Results<A> {
    let count = body.blocks.len();
    let direction = analysis.direction();
    let graph = Graph::new(body);
    let order = match direction {
        Direction::Forward => graph.reverse_postorder(),
        Direction::Backward => graph.postorder().to_vec(),
    };
    let chained_from = match direction {
        Direction::Forward => chains(&graph, &order, &keep_edge),
        Direction::Backward => backward_chains(&graph, &order, &keep_edge),
    };
    let mut block_states: Vec<Option<A::Domain>> = chained_from
        .iter()
        .map(|chained| chained.is_none().then(|| analysis.bottom()))
        .collect();
    match direction {
        Direction::Forward => {
            if count > 0 {
                analysis.initialize_boundary(stored(&mut block_states, BlockId(0)));
            }
        }
        Direction::Backward => {
            for (block, _) in body.blocks.iter() {
                if graph.successors(block).is_empty() {
                    analysis.initialize_boundary(stored(&mut block_states, block));
                }
            }
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        // The exit state of the block just visited, when the next block is chained to it.
        let mut carried: Option<A::Domain> = None;
        for &block in &order {
            let mut state = match chained_from[block.index()] {
                Some(source) => {
                    let mut state = carried
                        .take()
                        .expect("a chained block is visited right after its source");
                    if direction == Direction::Forward {
                        analysis.apply_edge(&mut state, source, block);
                    }
                    state
                }
                None => block_states[block.index()]
                    .clone()
                    .expect("the state of a block that is not chained"),
            };
            apply_block(body, &analysis, block, &mut state);
            match direction {
                Direction::Forward => {
                    let successors = graph.successors(block);
                    if let [next] = *successors {
                        if chained_from[next.index()] == Some(block) {
                            carried = Some(state);
                            continue;
                        }
                    }
                    for &next in successors {
                        if !keep_edge(block, next) {
                            continue;
                        }
                        if analysis.has_edge_effects() {
                            let mut edge_state = state.clone();
                            analysis.apply_edge(&mut edge_state, block, next);
                            changed |= stored(&mut block_states, next).join(&edge_state);
                        } else {
                            changed |= stored(&mut block_states, next).join(&state);
                        }
                    }
                }
                Direction::Backward => {
                    let predecessors = graph.predecessors(block);
                    if let [predecessor] = *predecessors {
                        if chained_from[predecessor.index()] == Some(block) {
                            carried = Some(state);
                            continue;
                        }
                    }
                    for &predecessor in predecessors {
                        if keep_edge(predecessor, block) {
                            changed |= stored(&mut block_states, predecessor).join(&state);
                        }
                    }
                }
            }
        }
    }
    Results {
        analysis,
        block_states,
        chained_from,
    }
}

/// Applies the effects of `block` to `state`, in the analysis's direction.
fn apply_block<A: Analysis>(body: &Body, analysis: &A, block: BlockId, state: &mut A::Domain) {
    let data = &body.blocks[block];
    let terminator = Location::terminator(body, block);
    match analysis.direction() {
        Direction::Forward => {
            for (index, statement) in data.statements.iter().enumerate() {
                analysis.apply_statement(
                    state,
                    statement,
                    Location {
                        block,
                        statement: index,
                    },
                );
            }
            analysis.apply_terminator(state, &data.terminator, terminator);
        }
        Direction::Backward => {
            analysis.apply_terminator(state, &data.terminator, terminator);
            for (index, statement) in data.statements.iter().enumerate().rev() {
                analysis.apply_statement(
                    state,
                    statement,
                    Location {
                        block,
                        statement: index,
                    },
                );
            }
        }
    }
}

impl<A: Analysis> Results<A> {
    /// The state where the analysis enters `block`: its start forward, its end backward. The
    /// state of a chained block is replayed from the block its chain starts at.
    pub fn block_state(&self, block: BlockId, body: &Body) -> A::Domain {
        // The chain back to the first block whose state is stored: `block`, its source, ...
        let mut chain = vec![block];
        while let Some(source) = self.chained_from[chain[chain.len() - 1].index()] {
            chain.push(source);
        }
        let head = chain[chain.len() - 1];
        let mut state = self.block_states[head.index()]
            .clone()
            .expect("the state of a block that is not chained");
        for pair in chain.windows(2).rev() {
            let (to, from) = (pair[0], pair[1]);
            apply_block(body, &self.analysis, from, &mut state);
            if self.analysis.direction() == Direction::Forward {
                self.analysis.apply_edge(&mut state, from, to);
            }
        }
        state
    }

    /// Calls `visit` on every block of `body` with the state where the analysis enters it,
    /// which `visit` must bring through the whole block, in the analysis's direction: the
    /// state it leaves is carried to the block chained to it, so that each state is copied
    /// once per stored block rather than once per block.
    pub fn visit_blocks(&self, mut visit: impl FnMut(BlockId, &mut A::Domain)) {
        let mut chained_to = vec![None; self.chained_from.len()];
        for (block, source) in self.chained_from.iter().enumerate() {
            if let Some(source) = source {
                chained_to[source.index()] = Some(BlockId(block as u32));
            }
        }
        for (head, stored) in self.block_states.iter().enumerate() {
            let Some(stored) = stored else {
                continue;
            };
            let mut state = stored.clone();
            let mut block = BlockId(head as u32);
            loop {
                visit(block, &mut state);
                let Some(next) = chained_to[block.index()] else {
                    break;
                };
                if self.analysis.direction() == Direction::Forward {
                    self.analysis.apply_edge(&mut state, block, next);
                }
                block = next;
            }
        }
    }

    /// A cursor over these results in `body`.
    pub fn cursor<'a>(&'a self, body: &'a Body) -> ResultsCursor<'a, A> {
        ResultsCursor {
            results: self,
            body,
            position: None,
            state: self.analysis.bottom(),
            snapshots: None,
            chain_exits: std::collections::HashMap::new(),
        }
    }
}

/// Gives the state at any location of a body by replaying its block's effects from the block's
/// state. Forward, it reuses the replay when locations of one block are visited in order, and
/// carries it into the next block when that block is chained to the current one. Backward, it
/// keeps the state at every location of the block it last entered, so that a block's locations
/// can be visited in any order, such as in program order with a forward cursor, each in
/// constant time.
pub struct ResultsCursor<'a, A: Analysis> {
    results: &'a Results<A>,
    body: &'a Body,
    /// Forward: the location whose effect `state` was last brought to, exclusive: the next
    /// effect to apply is at `position`.
    position: Option<Location>,
    state: A::Domain,
    /// Backward: the block last entered, and the state after each of its locations.
    snapshots: Option<(BlockId, Vec<A::Domain>)>,
    /// Backward: the exit states of chained blocks replayed when a block of their chain was
    /// entered, each taken when its block is entered, so that entering every block of a long
    /// chain replays it once.
    chain_exits: std::collections::HashMap<BlockId, A::Domain>,
}

impl<A: Analysis> ResultsCursor<'_, A> {
    /// The state before the effect of `location` in the analysis's direction: before it in
    /// program order forward, after it backward.
    pub fn seek(&mut self, location: Location) -> &A::Domain {
        match self.results.analysis.direction() {
            Direction::Forward => self.seek_forward(location),
            Direction::Backward => self.seek_backward(location),
        }
    }

    /// Forward only: the state before `location`, for the caller to bring past the location
    /// by applying its effect itself, in place: the cursor continues from after it. A caller
    /// that replays a location's effects one by one, such as the use check, needs no copy.
    pub fn seek_to_apply(&mut self, location: Location) -> &mut A::Domain {
        assert_eq!(
            self.results.analysis.direction(),
            Direction::Forward,
            "a backward cursor cannot be stepped in place"
        );
        self.seek_forward(location);
        self.position = Some(Location {
            block: location.block,
            statement: location.statement + 1,
        });
        &mut self.state
    }

    fn seek_forward(&mut self, location: Location) -> &A::Domain {
        let results = self.results;
        let reusable = self.position.is_some_and(|position| {
            position.block == location.block && position.statement <= location.statement
        });
        if !reusable {
            // The chain back from the block to the block the cursor is in, when the cursor is
            // earlier in the same chain, or else to the first block whose state is stored:
            // `location.block`, its predecessor, ...
            let mut chain = vec![location.block];
            let mut resumed = None;
            while let Some(predecessor) = results.chained_from[chain[chain.len() - 1].index()] {
                chain.push(predecessor);
                if let Some(position) = self.position.filter(|p| p.block == predecessor) {
                    resumed = Some(position);
                    break;
                }
            }
            let head = chain[chain.len() - 1];
            let mut position = match resumed {
                Some(position) => position,
                None => {
                    self.state = results.block_state(head, self.body);
                    Location::start(head)
                }
            };
            // Each block of the chain but the last, from where the cursor is to its end.
            for pair in chain.windows(2).rev() {
                let (to, from) = (pair[0], pair[1]);
                let terminator = Location::terminator(self.body, from);
                while position.statement <= terminator.statement {
                    self.apply(position);
                    position.statement += 1;
                }
                results.analysis.apply_edge(&mut self.state, from, to);
                position = Location::start(to);
            }
            self.position = Some(position);
        }
        let mut position = self.position.expect("the cursor has a position");
        while position.statement < location.statement {
            self.apply(position);
            position.statement += 1;
        }
        self.position = Some(position);
        &self.state
    }

    fn seek_backward(&mut self, location: Location) -> &A::Domain {
        let entered = matches!(&self.snapshots, Some((block, _)) if *block == location.block);
        if !entered {
            let mut state = self.backward_block_state(location.block);
            let analysis = &self.results.analysis;
            let data = &self.body.blocks[location.block];
            // The state after each location, from the terminator's back to the first
            // statement's, in the buffer of the block entered before.
            let mut states = match self.snapshots.take() {
                Some((_, mut states)) => {
                    states.clear();
                    states
                }
                None => Vec::with_capacity(data.statements.len() + 1),
            };
            for statement in (0..=data.statements.len()).rev() {
                states.push(state.clone());
                if statement == 0 {
                    break;
                }
                let after = Location {
                    block: location.block,
                    statement,
                };
                match data.statements.get(statement) {
                    Some(next) => analysis.apply_statement(&mut state, next, after),
                    None => analysis.apply_terminator(&mut state, &data.terminator, after),
                }
            }
            states.reverse();
            self.snapshots = Some((location.block, states));
        }
        let (_, states) = self.snapshots.as_ref().expect("the block was entered");
        &states[location.statement]
    }

    /// Backward: the state at the exit of `block`. A chained block's is taken from the states
    /// replayed for its chain, replaying the chain from its stored block when it was not.
    fn backward_block_state(&mut self, block: BlockId) -> A::Domain {
        let results = self.results;
        if results.chained_from[block.index()].is_none() {
            return results.block_state(block, self.body);
        }
        if let Some(state) = self.chain_exits.remove(&block) {
            return state;
        }
        // The chain to the stored block: `block`, its source, ... Each source's entry state is
        // the exit state of the block chained to it.
        let mut chain = vec![block];
        while let Some(source) = results.chained_from[chain[chain.len() - 1].index()] {
            chain.push(source);
        }
        let tail = chain[chain.len() - 1];
        let mut state = results.block_states[tail.index()]
            .clone()
            .expect("the state of a block that is not chained");
        for pair in chain.windows(2).rev() {
            let (chained, source) = (pair[0], pair[1]);
            apply_block(self.body, &results.analysis, source, &mut state);
            if chained != block {
                self.chain_exits.insert(chained, state.clone());
            }
        }
        state
    }

    fn apply(&mut self, location: Location) {
        let analysis = &self.results.analysis;
        let data = &self.body.blocks[location.block];
        match data.statements.get(location.statement) {
            Some(statement) => analysis.apply_statement(&mut self.state, statement, location),
            None => analysis.apply_terminator(&mut self.state, &data.terminator, location),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_bodies::body_with_edges;

    #[test]
    fn bit_sets() {
        let mut set = BitSet::new(130);
        assert!(set.is_empty());
        assert!(set.insert(3));
        assert!(!set.insert(3));
        assert!(set.insert(129));
        assert!(set.contains(129) && !set.contains(128));
        let mut other = BitSet::new(130);
        other.insert(64);
        assert!(set.union(&other));
        assert!(!set.union(&other));
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![3, 64, 129]);
        assert!(set.remove(3) && !set.remove(3));
        set.clear();
        assert!(set.is_empty());
    }

    impl BitSet {
        fn chunks(&self) -> &[Chunk] {
            match &self.bits {
                Bits::Chunked(chunks) => chunks,
                Bits::Sparse(_) => &[],
            }
        }
    }

    #[test]
    fn chunked_bit_sets() {
        // Three chunks, the last of 5 bits.
        let size = 2 * CHUNK_BITS + 5;
        let mut set = BitSet::new(size);
        for bit in 0..CHUNK_BITS {
            assert!(set.insert(bit));
        }
        assert_eq!(set.chunks()[0], Chunk::Ones);
        for bit in 2 * CHUNK_BITS..size {
            set.insert(bit);
        }
        assert_eq!(set.chunks()[2], Chunk::Ones);
        assert_eq!(set.iter().count(), CHUNK_BITS + 5);
        // A copy shares its chunks until one of them changes.
        let mut copy = set.clone();
        assert!(copy.remove(7) && !copy.contains(7) && set.contains(7));
        assert!(copy.insert(CHUNK_BITS + 1));
        assert!(!set.contains(CHUNK_BITS + 1));
        // Removing a full chunk's bits, then joining them back, normalizes the chunks.
        assert!(set.union(&copy));
        assert!(!set.union(&copy));
        assert!(copy.insert(7));
        assert_eq!(copy, set);
        for bit in 0..CHUNK_BITS {
            copy.remove(bit);
        }
        assert_eq!(copy.chunks()[0], Chunk::Zeros);
        assert_eq!(
            copy.iter().collect::<Vec<_>>(),
            [CHUNK_BITS + 1]
                .into_iter()
                .chain(2 * CHUNK_BITS..size)
                .collect::<Vec<_>>()
        );
        let mut other = BitSet::new(size);
        other.insert(CHUNK_BITS + 2);
        assert!(copy.union(&other) && copy.contains(CHUNK_BITS + 2));
        copy.clear();
        assert!(copy.is_empty() && !copy.contains(size));
    }

    #[test]
    fn sparse_bit_sets_switch_to_chunks() {
        let size = 3 * CHUNK_BITS;
        let mut sparse = BitSet::new(size);
        for bit in [5, 2 * CHUNK_BITS, 70] {
            sparse.insert(bit);
        }
        assert!(matches!(sparse.bits, Bits::Sparse(_)));
        assert_eq!(
            sparse.iter().collect::<Vec<_>>(),
            vec![5, 70, 2 * CHUNK_BITS]
        );
        let mut chunked = BitSet::new(size);
        for bit in 0..=SPARSE_BITS * 2 {
            chunked.insert(bit * 3);
        }
        assert!(matches!(chunked.bits, Bits::Chunked(_)));
        assert_eq!(chunked.iter().count(), SPARSE_BITS * 2 + 1);
        // Equal sets are equal in either form.
        let mut shrunk = chunked.clone();
        for bit in 0..=SPARSE_BITS * 2 {
            shrunk.remove(bit * 3);
        }
        for bit in [5, 2 * CHUNK_BITS, 70] {
            shrunk.insert(bit);
        }
        assert_eq!(shrunk, sparse);
        // Joining a sparse set into a chunked one and the other way round.
        let mut joined = sparse.clone();
        assert!(joined.union(&chunked));
        assert!(!joined.union(&chunked) && !joined.union(&sparse));
        let mut other = chunked.clone();
        assert!(other.union(&sparse));
        assert_eq!(joined, other);
        assert_eq!(joined.iter().count(), SPARSE_BITS * 2 + 1 + 3);
        let mut empty = BitSet::new(size);
        assert!(!empty.union(&BitSet::new(size)) && empty.is_empty());
    }

    #[test]
    fn pair_sets() {
        let mut set = PairSet::default();
        assert!(set.is_empty() && !set.remove((0, 0)));
        // Enough pairs, inserted out of order, for several chunks.
        let pairs: Vec<(u32, u32)> = (0..1000u32).map(|i| ((i * 7) % 100, i)).collect();
        for pair in &pairs {
            assert!(set.insert(*pair));
        }
        assert!(!set.insert(pairs[3]));
        assert!(set.chunks.len() > 1);
        assert!(set.chunks.iter().all(|chunk| chunk.len() <= PAIR_CHUNK));
        let mut sorted = pairs.clone();
        sorted.sort_unstable();
        assert_eq!(set.iter().collect::<Vec<_>>(), sorted);
        assert_eq!(set.len(), 1000);
        assert_eq!(
            set.with_first(7).collect::<Vec<_>>(),
            sorted
                .iter()
                .copied()
                .filter(|(first, _)| *first == 7)
                .collect::<Vec<_>>()
        );
        assert_eq!(set.with_first(100).count(), 0);
        // A copy shares the chunks; a change to it leaves the original as it was.
        let mut copy = set.clone();
        assert!(copy.remove((7, 1)) && !copy.contains((7, 1)) && set.contains((7, 1)));
        assert!(!set.union(&copy));
        assert!(copy.union(&set) && copy == set);
        // Removing most pairs merges the chunks that became small.
        for pair in &pairs[..990] {
            assert!(set.remove(*pair));
        }
        assert_eq!(set.len(), 10);
        assert!(set.chunks.len() <= 2, "{} chunks", set.chunks.len());
        set.retain(|(first, _)| first % 2 == 0);
        assert!(set.iter().all(|(first, _)| first % 2 == 0));
        // A union with few shared chunks merges the sets.
        let mut other = PairSet::default();
        for i in 0..300 {
            other.insert((i, i));
        }
        let expected: Vec<(u32, u32)> = {
            let mut all: Vec<(u32, u32)> = set.iter().chain(other.iter()).collect();
            all.sort_unstable();
            all.dedup();
            all
        };
        assert!(set.union(&other));
        assert_eq!(set.iter().collect::<Vec<_>>(), expected);
        assert_eq!(set.len(), expected.len());
    }

    /// A pair set and a sorted set kept in step through pseudo-random inserts and removes,
    /// including in copies sharing chunks: each step checks the chunks' last pairs, which the
    /// lookups search.
    #[test]
    fn pair_sets_follow_a_model() {
        let mut set = PairSet::default();
        let mut model = std::collections::BTreeSet::new();
        let mut seed: u64 = 7;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        let mut copies = Vec::new();
        for step in 0..6_000 {
            let pair = (next() % 40, next() % 200);
            if next() % 3 == 0 {
                assert_eq!(set.remove(pair), model.remove(&pair));
            } else {
                assert_eq!(set.insert(pair), model.insert(pair));
            }
            if step % 997 == 0 {
                copies.push((set.clone(), model.clone()));
            }
            assert_eq!(set.len(), model.len());
            assert_eq!(set.lasts.len(), set.chunks.len());
            for (chunk, last) in set.chunks.iter().zip(&set.lasts) {
                assert_eq!(chunk.last(), Some(last));
            }
            let first = next() % 40;
            assert!(set
                .with_first(first)
                .eq(model.iter().copied().filter(|(f, _)| *f == first)));
            assert_eq!(set.contains(pair), model.contains(&pair));
        }
        for (copy, model) in copies {
            assert!(copy.iter().eq(model.iter().copied()));
        }
    }

    /// Forward: the blocks that may have run before a point. Backward: the blocks that may run
    /// after it.
    struct BlocksRun {
        size: usize,
        direction: Direction,
    }

    impl Analysis for BlocksRun {
        type Domain = BitSet;

        fn bottom(&self) -> BitSet {
            BitSet::new(self.size)
        }

        fn direction(&self) -> Direction {
            self.direction
        }

        fn initialize_boundary(&self, _: &mut BitSet) {}

        fn apply_statement(&self, _: &mut BitSet, _: &Statement, _: Location) {}

        fn apply_terminator(&self, state: &mut BitSet, _: &Terminator, location: Location) {
            state.insert(location.block.index());
        }
    }

    fn blocks(set: &BitSet) -> Vec<usize> {
        set.iter().collect()
    }

    /// `bb0 -> bb1 (loop head) -> {bb2 (body) -> bb1, bb3 (exit)}`.
    fn while_loop() -> Body {
        body_with_edges(&[&[1], &[3, 2], &[1], &[]])
    }

    #[test]
    fn a_forward_analysis_reaches_a_fixpoint_through_a_loop() {
        let body = while_loop();
        let results = solve(
            &body,
            BlocksRun {
                size: 4,
                direction: Direction::Forward,
            },
        );
        assert_eq!(
            blocks(&results.block_state(BlockId(0), &body)),
            Vec::<usize>::new()
        );
        // The loop head is reached from the entry and, around the loop, from the body.
        assert_eq!(
            blocks(&results.block_state(BlockId(1), &body)),
            vec![0, 1, 2]
        );
        assert_eq!(
            blocks(&results.block_state(BlockId(3), &body)),
            vec![0, 1, 2]
        );
        let mut cursor = results.cursor(&body);
        let after_head = Location::terminator(&body, BlockId(1));
        assert_eq!(blocks(cursor.seek(after_head)), vec![0, 1, 2]);
        // Ignoring the back edge, the head is reached from the entry only.
        let dominators = crate::dominators::Dominators::compute(&body);
        let acyclic = solve_with(
            &body,
            BlocksRun {
                size: 4,
                direction: Direction::Forward,
            },
            |from, to| !dominators.is_back_edge(from, to),
        );
        assert_eq!(blocks(&acyclic.block_state(BlockId(1), &body)), vec![0]);
    }

    /// `bb0 -> bb1 -> bb2 -> {bb3, bb4} -> bb5`: `bb1` and `bb2` are chained.
    fn chain_then_branch() -> Body {
        body_with_edges(&[&[1], &[2], &[3, 4], &[5], &[5], &[]])
    }

    #[test]
    fn a_chained_block_s_state_is_carried_from_its_predecessor() {
        let body = chain_then_branch();
        let results = solve(
            &body,
            BlocksRun {
                size: 6,
                direction: Direction::Forward,
            },
        );
        assert_eq!(results.chained_from[1], Some(BlockId(0)));
        assert_eq!(results.chained_from[2], Some(BlockId(1)));
        // A block after a branch, and a merge, store their states.
        assert_eq!(results.chained_from[3], None);
        assert_eq!(results.chained_from[5], None);
        assert_eq!(blocks(&results.block_state(BlockId(2), &body)), vec![0, 1]);
        assert_eq!(
            blocks(&results.block_state(BlockId(5), &body)),
            vec![0, 1, 2, 3, 4]
        );
        // In order, the cursor carries the state along the chain, past blocks it skips; out of
        // order, it replays the chain from its start.
        let mut cursor = results.cursor(&body);
        for (block, expected) in [
            (0, vec![]),
            (2, vec![0, 1]),
            (1, vec![0]),
            (2, vec![0, 1]),
            (5, vec![0, 1, 2, 3, 4]),
            (2, vec![0, 1]),
        ] {
            assert_eq!(
                blocks(cursor.seek(Location::start(BlockId(block)))),
                expected
            );
        }
    }

    #[test]
    fn a_cursor_stepped_in_place_continues_from_after_the_location() {
        let body = chain_then_branch();
        let results = solve(
            &body,
            BlocksRun {
                size: 6,
                direction: Direction::Forward,
            },
        );
        let mut cursor = results.cursor(&body);
        // The caller applies each terminator's effect: the next block, chained or not, starts
        // from the state it left.
        for block in [0, 1, 2] {
            let terminator = Location::terminator(&body, BlockId(block));
            let state = cursor.seek_to_apply(terminator);
            assert_eq!(blocks(state), (0..block as usize).collect::<Vec<_>>());
            state.insert(block as usize);
        }
        assert_eq!(
            blocks(cursor.seek(Location::start(BlockId(3)))),
            vec![0, 1, 2]
        );
        assert_eq!(blocks(cursor.seek(Location::start(BlockId(1)))), vec![0]);
    }

    #[test]
    fn a_backward_cursor_is_visited_in_program_order() {
        let body = chain_then_branch();
        let results = solve(
            &body,
            BlocksRun {
                size: 6,
                direction: Direction::Backward,
            },
        );
        let mut cursor = results.cursor(&body);
        // After each block's terminator in program order, then back to an earlier block.
        for (block, expected) in [
            (0, vec![1, 2, 3, 4, 5]),
            (1, vec![2, 3, 4, 5]),
            (2, vec![3, 4, 5]),
            (1, vec![2, 3, 4, 5]),
        ] {
            let terminator = Location::terminator(&body, BlockId(block));
            assert_eq!(blocks(cursor.seek(terminator)), expected);
        }
    }

    #[test]
    fn a_backward_analysis_runs_against_the_edges() {
        let body = while_loop();
        let results = solve(
            &body,
            BlocksRun {
                size: 4,
                direction: Direction::Backward,
            },
        );
        // At the end of each block: the blocks that may run after it.
        assert_eq!(
            blocks(&results.block_state(BlockId(3), &body)),
            Vec::<usize>::new()
        );
        assert_eq!(
            blocks(&results.block_state(BlockId(2), &body)),
            vec![1, 2, 3]
        );
        assert_eq!(
            blocks(&results.block_state(BlockId(0), &body)),
            vec![1, 2, 3]
        );
        let mut cursor = results.cursor(&body);
        // After the terminator of bb3 nothing runs; after the loop head's, the loop may run again.
        assert_eq!(
            blocks(cursor.seek(Location::terminator(&body, BlockId(3)))),
            Vec::<usize>::new()
        );
        assert_eq!(
            blocks(cursor.seek(Location::terminator(&body, BlockId(1)))),
            vec![1, 2, 3]
        );
    }
}
