//! A bit-set dataflow framework over a body's control-flow graph (D4 of `borrowck-moves`).
//!
//! An [`Analysis`] names a domain of bits (move paths, move events, ...), a direction, the
//! state at the boundary of the body, and the effect of each statement and terminator. [`solve`]
//! iterates the blocks (in reverse postorder forward, postorder backward) joining states by union
//! until no block's state changes, and a [`ResultsCursor`] replays a block's effects to give the
//! state at any location. `borrowck-exclusivity` adds its liveness (backward) and borrow
//! analyses on the same framework.

use crate::location::{postorder, predecessors, reverse_postorder, Location};
use prism_mir::{BlockId, Body, Statement, Terminator};
use prism_typed_tree::ArenaId;

/// A fixed-size set of small integers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BitSet {
    words: Vec<u64>,
    size: usize,
}

impl BitSet {
    /// The empty set over `0..size`.
    pub fn new(size: usize) -> Self {
        Self {
            words: vec![0; size.div_ceil(64)],
            size,
        }
    }

    /// The size of the domain.
    pub fn domain_size(&self) -> usize {
        self.size
    }

    /// Adds `bit`, returning whether it was absent.
    pub fn insert(&mut self, bit: usize) -> bool {
        assert!(
            bit < self.size,
            "bit {} out of a domain of {}",
            bit,
            self.size
        );
        let (word, mask) = (bit / 64, 1u64 << (bit % 64));
        let absent = self.words[word] & mask == 0;
        self.words[word] |= mask;
        absent
    }

    /// Removes `bit`, returning whether it was present.
    pub fn remove(&mut self, bit: usize) -> bool {
        assert!(
            bit < self.size,
            "bit {} out of a domain of {}",
            bit,
            self.size
        );
        let (word, mask) = (bit / 64, 1u64 << (bit % 64));
        let present = self.words[word] & mask != 0;
        self.words[word] &= !mask;
        present
    }

    pub fn contains(&self, bit: usize) -> bool {
        bit < self.size && self.words[bit / 64] & (1u64 << (bit % 64)) != 0
    }

    /// Adds every bit of `other`, returning whether this set changed.
    pub fn union(&mut self, other: &BitSet) -> bool {
        assert_eq!(
            self.size, other.size,
            "union of sets over different domains"
        );
        let mut changed = false;
        for (word, other) in self.words.iter_mut().zip(&other.words) {
            let joined = *word | *other;
            changed |= joined != *word;
            *word = joined;
        }
        changed
    }

    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    /// Removes every bit.
    pub fn clear(&mut self) {
        self.words.iter_mut().for_each(|word| *word = 0);
    }

    /// The bits of the set, in increasing order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.size).filter(|bit| self.contains(*bit))
    }
}

/// Whether an analysis follows the edges of the control-flow graph or runs against them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// A dataflow analysis over the bits `0..domain_size` of one body, joined by union.
pub trait Analysis {
    /// The number of bits of the domain.
    fn domain_size(&self) -> usize;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    /// Sets the state at the boundary: the entry of `bb0` forward, the exit of each block without
    /// successors backward.
    fn initialize_boundary(&self, state: &mut BitSet);

    /// Applies the effect of `statement`, at `location`.
    fn apply_statement(&self, state: &mut BitSet, statement: &Statement, location: Location);

    /// Applies the effect of `terminator`, at `location`. Forward, it holds on every outgoing
    /// edge.
    fn apply_terminator(&self, state: &mut BitSet, terminator: &Terminator, location: Location);
}

/// The fixpoint of an analysis: the state where the analysis enters each block (its start
/// forward, its end backward).
pub struct Results<A> {
    pub analysis: A,
    block_states: Vec<BitSet>,
}

/// Solves `analysis` over every edge of `body`.
pub fn solve<A: Analysis>(body: &Body, analysis: A) -> Results<A> {
    solve_with(body, analysis, |_, _| true)
}

/// Solves `analysis` over the edges `from -> to` of `body` for which `keep_edge` holds, such as
/// every edge but the back edges.
pub fn solve_with<A: Analysis>(
    body: &Body,
    analysis: A,
    keep_edge: impl Fn(BlockId, BlockId) -> bool,
) -> Results<A> {
    let size = analysis.domain_size();
    let count = body.blocks.len();
    let mut block_states = vec![BitSet::new(size); count];
    let direction = analysis.direction();
    let order = match direction {
        Direction::Forward => reverse_postorder(body),
        Direction::Backward => postorder(body),
    };
    match direction {
        Direction::Forward => {
            if count > 0 {
                analysis.initialize_boundary(&mut block_states[0]);
            }
        }
        Direction::Backward => {
            for (block, data) in body.blocks.iter() {
                if data.terminator.kind.successors().is_empty() {
                    analysis.initialize_boundary(&mut block_states[block.index()]);
                }
            }
        }
    }
    let predecessors = predecessors(body);
    let mut changed = true;
    while changed {
        changed = false;
        for &block in &order {
            let mut state = block_states[block.index()].clone();
            apply_block(body, &analysis, block, &mut state);
            match direction {
                Direction::Forward => {
                    for successor in body.blocks[block].terminator.kind.successors() {
                        if keep_edge(block, successor) {
                            changed |= block_states[successor.index()].union(&state);
                        }
                    }
                }
                Direction::Backward => {
                    for &predecessor in &predecessors[block.index()] {
                        if keep_edge(predecessor, block) {
                            changed |= block_states[predecessor.index()].union(&state);
                        }
                    }
                }
            }
        }
    }
    Results {
        analysis,
        block_states,
    }
}

/// Applies the effects of `block` to `state`, in the analysis's direction.
fn apply_block<A: Analysis>(body: &Body, analysis: &A, block: BlockId, state: &mut BitSet) {
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
    /// The state where the analysis enters `block`: its start forward, its end backward.
    pub fn block_state(&self, block: BlockId) -> &BitSet {
        &self.block_states[block.index()]
    }

    /// A cursor over these results in `body`.
    pub fn cursor<'a>(&'a self, body: &'a Body) -> ResultsCursor<'a, A> {
        ResultsCursor {
            results: self,
            body,
            position: None,
            state: BitSet::new(self.analysis.domain_size()),
        }
    }
}

/// Gives the state at any location of a body by replaying its block's effects from the block's
/// state, reusing the replay when locations of one block are visited in the analysis's
/// direction.
pub struct ResultsCursor<'a, A> {
    results: &'a Results<A>,
    body: &'a Body,
    /// The location whose effect `state` was last brought to, exclusive: the next effect to
    /// apply is at `position` forward, before it backward.
    position: Option<Location>,
    state: BitSet,
}

impl<A: Analysis> ResultsCursor<'_, A> {
    /// The state before the effect of `location` in the analysis's direction: before it in
    /// program order forward, after it backward.
    pub fn seek(&mut self, location: Location) -> &BitSet {
        let analysis = &self.results.analysis;
        let data = &self.body.blocks[location.block];
        let forward = analysis.direction() == Direction::Forward;
        let reusable = self.position.is_some_and(|position| {
            position.block == location.block
                && if forward {
                    position.statement <= location.statement
                } else {
                    position.statement > location.statement
                }
        });
        if !reusable {
            self.state = self.results.block_state(location.block).clone();
            let start = if forward {
                0
            } else {
                // Nothing applied yet: the next effect is the terminator's.
                data.statements.len() + 1
            };
            self.position = Some(Location {
                block: location.block,
                statement: start,
            });
        }
        let mut position = self.position.expect("the cursor has a position");
        if forward {
            while position.statement < location.statement {
                self.apply(position);
                position.statement += 1;
            }
        } else {
            while position.statement > location.statement + 1 {
                position.statement -= 1;
                self.apply(position);
            }
        }
        self.position = Some(position);
        &self.state
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

    /// Forward: the blocks that may have run before a point. Backward: the blocks that may run
    /// after it.
    struct BlocksRun {
        size: usize,
        direction: Direction,
    }

    impl Analysis for BlocksRun {
        fn domain_size(&self) -> usize {
            self.size
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
        assert_eq!(blocks(results.block_state(BlockId(0))), Vec::<usize>::new());
        // The loop head is reached from the entry and, around the loop, from the body.
        assert_eq!(blocks(results.block_state(BlockId(1))), vec![0, 1, 2]);
        assert_eq!(blocks(results.block_state(BlockId(3))), vec![0, 1, 2]);
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
        assert_eq!(blocks(acyclic.block_state(BlockId(1))), vec![0]);
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
        assert_eq!(blocks(results.block_state(BlockId(3))), Vec::<usize>::new());
        assert_eq!(blocks(results.block_state(BlockId(2))), vec![1, 2, 3]);
        assert_eq!(blocks(results.block_state(BlockId(0))), vec![1, 2, 3]);
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
