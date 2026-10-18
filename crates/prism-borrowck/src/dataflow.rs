//! A dataflow framework over a body's control-flow graph (D4 of `borrowck-moves`).
//!
//! An [`Analysis`] names its states (a [`Domain`], such as bits over move paths or move
//! events), a direction, the state at the boundary of the body, and the effect of each
//! statement and terminator. [`solve`] iterates the blocks (in reverse postorder forward,
//! postorder backward) joining states until no block's state changes, and a [`ResultsCursor`]
//! replays a block's effects to give the state at any location. `borrowck-exclusivity` adds its
//! liveness (backward) and borrow analyses on the same framework.
//!
//! Its cost is linear in the body's locations times the size of a state, per iteration: a
//! forward analysis stores a state only for the blocks that are not chained to their
//! predecessor ([`Results`]), and a backward cursor visited in program order keeps the states
//! of its current block instead of replaying the block for each location.

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
}

/// The fixpoint of an analysis: the state where the analysis enters each block (its start
/// forward, its end backward).
///
/// Forward, a block whose only predecessor has it as only successor is *chained* to that
/// predecessor: its state is the predecessor's exit state, which the solver and the cursor
/// carry from one to the other instead of storing it, so that a long straight-line body (whose
/// calls each end a block) stores one state, not one per block.
pub struct Results<A: Analysis> {
    pub analysis: A,
    /// The state where the analysis enters each block that is not chained; `None` for a
    /// chained block.
    block_states: Vec<Option<A::Domain>>,
    /// The predecessor each chained block is chained to.
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
    body: &Body,
    order: &[BlockId],
    predecessors: &[Vec<BlockId>],
    keep_edge: &impl Fn(BlockId, BlockId) -> bool,
) -> Vec<Option<BlockId>> {
    let mut chained_from = vec![None; body.blocks.len()];
    // Each block's number of successors, counted once: a switch has many.
    let exits: Vec<usize> = body
        .blocks
        .iter()
        .map(|(_, data)| data.terminator.kind.successors().len())
        .collect();
    for &block in order {
        if block.index() == 0 {
            continue;
        }
        if let [predecessor] = predecessors[block.index()][..] {
            let single_exit = exits[predecessor.index()] == 1;
            if predecessor != block && single_exit && keep_edge(predecessor, block) {
                chained_from[block.index()] = Some(predecessor);
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
    let order = match direction {
        Direction::Forward => reverse_postorder(body),
        Direction::Backward => postorder(body),
    };
    let predecessors = predecessors(body);
    let chained_from = match direction {
        Direction::Forward => chains(body, &order, &predecessors, &keep_edge),
        Direction::Backward => vec![None; count],
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
            for (block, data) in body.blocks.iter() {
                if data.terminator.kind.successors().is_empty() {
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
                Some(predecessor) => {
                    let mut state = carried
                        .take()
                        .expect("a chained block is visited right after its predecessor");
                    analysis.apply_edge(&mut state, predecessor, block);
                    state
                }
                None => block_states[block.index()]
                    .clone()
                    .expect("the state of a block that is not chained"),
            };
            apply_block(body, &analysis, block, &mut state);
            match direction {
                Direction::Forward => {
                    let successors = body.blocks[block].terminator.kind.successors();
                    if let [successor] = successors[..] {
                        if chained_from[successor.index()] == Some(block) {
                            carried = Some(state);
                            continue;
                        }
                    }
                    for successor in successors {
                        if keep_edge(block, successor) {
                            let mut edge_state = state.clone();
                            analysis.apply_edge(&mut edge_state, block, successor);
                            changed |= stored(&mut block_states, successor).join(&edge_state);
                        }
                    }
                }
                Direction::Backward => {
                    for &predecessor in &predecessors[block.index()] {
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
        // The chain back to the first block whose state is stored: `block`, its predecessor, ...
        let mut chain = vec![block];
        while let Some(predecessor) = self.chained_from[chain[chain.len() - 1].index()] {
            chain.push(predecessor);
        }
        let head = chain[chain.len() - 1];
        let mut state = self.block_states[head.index()]
            .clone()
            .expect("the state of a block that is not chained");
        for pair in chain.windows(2).rev() {
            let (to, from) = (pair[0], pair[1]);
            apply_block(body, &self.analysis, from, &mut state);
            self.analysis.apply_edge(&mut state, from, to);
        }
        state
    }

    /// A cursor over these results in `body`.
    pub fn cursor<'a>(&'a self, body: &'a Body) -> ResultsCursor<'a, A> {
        ResultsCursor {
            results: self,
            body,
            position: None,
            state: self.analysis.bottom(),
            snapshots: None,
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
            let analysis = &self.results.analysis;
            let data = &self.body.blocks[location.block];
            let mut state = self.results.block_state(location.block, self.body);
            // The state after each location, from the terminator's back to the first
            // statement's.
            let mut states = Vec::with_capacity(data.statements.len() + 1);
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
