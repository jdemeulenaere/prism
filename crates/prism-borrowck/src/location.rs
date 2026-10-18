//! Locations in a body's control-flow graph, and the block orders the analyses visit.

use prism_mir::{BlockId, Body, TerminatorKind};
use prism_typed_tree::ArenaId;

/// A point of a body: statement `statement` of block `block`, or its terminator when `statement`
/// is the number of statements of the block. Locations order by block, then statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location {
    pub block: BlockId,
    pub statement: usize,
}

impl Location {
    /// The first location of `block`.
    pub fn start(block: BlockId) -> Self {
        Self {
            block,
            statement: 0,
        }
    }

    /// The terminator of `block` in `body`.
    pub fn terminator(body: &Body, block: BlockId) -> Self {
        Self {
            block,
            statement: body.blocks[block].statements.len(),
        }
    }

    /// Whether the location is its block's terminator in `body`.
    pub fn is_terminator(self, body: &Body) -> bool {
        self.statement == body.blocks[self.block].statements.len()
    }
}

/// Successor `index` of a terminator of kind `kind`, in the order of
/// [`TerminatorKind::successors`], which this reads without building a list: the analyses walk
/// the edges of every block several times.
pub fn successor(kind: &TerminatorKind, index: usize) -> Option<BlockId> {
    match kind {
        TerminatorKind::Goto(target) | TerminatorKind::Drop { target, .. } => {
            (index == 0).then_some(*target)
        }
        TerminatorKind::SwitchInt {
            targets, otherwise, ..
        } => match targets.get(index) {
            Some((_, target)) => Some(*target),
            None => (index == targets.len()).then_some(*otherwise),
        },
        TerminatorKind::Call { target, .. } => target.filter(|_| index == 0),
        TerminatorKind::Return | TerminatorKind::Unreachable => None,
    }
}

/// The successors of a terminator of kind `kind`, in order.
pub fn successors(kind: &TerminatorKind) -> impl Iterator<Item = BlockId> + '_ {
    (0..).map_while(move |index| successor(kind, index))
}

/// The edges of a body's control-flow graph and its postorder, in flat arrays read once from
/// the body: the solver walks the edges of every block on each of its passes, and the blocks of
/// a large body are far apart in memory.
pub struct Graph {
    /// Where the successors of each block start in `successor_targets`, then their end.
    successor_offsets: Vec<u32>,
    successor_targets: Vec<BlockId>,
    /// Where the predecessors of each block start in `predecessor_sources`, then their end.
    predecessor_offsets: Vec<u32>,
    predecessor_sources: Vec<BlockId>,
    /// The blocks reachable from `bb0`, in postorder.
    postorder: Vec<BlockId>,
}

impl Graph {
    pub fn new(body: &Body) -> Self {
        let count = body.blocks.len();
        let mut successor_offsets = Vec::with_capacity(count + 1);
        let mut successor_targets = Vec::with_capacity(count);
        let mut incoming = vec![0u32; count + 1];
        for (_, data) in body.blocks.iter() {
            successor_offsets.push(successor_targets.len() as u32);
            for target in successors(&data.terminator.kind) {
                successor_targets.push(target);
                incoming[target.index() + 1] += 1;
            }
        }
        successor_offsets.push(successor_targets.len() as u32);
        // Predecessors in block order, once per edge.
        for block in 0..count {
            incoming[block + 1] += incoming[block];
        }
        let predecessor_offsets = incoming.clone();
        let mut next = incoming;
        let mut predecessor_sources = vec![BlockId(0); successor_targets.len()];
        for block in 0..count {
            let range = successor_offsets[block] as usize..successor_offsets[block + 1] as usize;
            for target in &successor_targets[range] {
                predecessor_sources[next[target.index()] as usize] = BlockId(block as u32);
                next[target.index()] += 1;
            }
        }
        let mut graph = Self {
            successor_offsets,
            successor_targets,
            predecessor_offsets,
            predecessor_sources,
            postorder: Vec::with_capacity(count),
        };
        if count > 0 {
            let mut visited = vec![false; count];
            // Each frame is a block and the index of its next successor to visit.
            let mut stack = vec![(BlockId(0), 0usize)];
            visited[0] = true;
            while let Some((block, next)) = stack.last_mut() {
                if let Some(&target) = graph.successors(*block).get(*next) {
                    *next += 1;
                    if !visited[target.index()] {
                        visited[target.index()] = true;
                        stack.push((target, 0));
                    }
                } else {
                    graph.postorder.push(*block);
                    stack.pop();
                }
            }
        }
        graph
    }

    /// The number of blocks of the body.
    pub fn block_count(&self) -> usize {
        self.successor_offsets.len() - 1
    }

    /// The successors of `block`, in the order of [`TerminatorKind::successors`].
    pub fn successors(&self, block: BlockId) -> &[BlockId] {
        let index = block.index();
        &self.successor_targets
            [self.successor_offsets[index] as usize..self.successor_offsets[index + 1] as usize]
    }

    /// The predecessors of `block`, in block order, once per edge.
    pub fn predecessors(&self, block: BlockId) -> &[BlockId] {
        let index = block.index();
        &self.predecessor_sources
            [self.predecessor_offsets[index] as usize..self.predecessor_offsets[index + 1] as usize]
    }

    /// The blocks reachable from `bb0`, in postorder of a depth-first search that visits
    /// successors in order.
    pub fn postorder(&self) -> &[BlockId] {
        &self.postorder
    }

    /// The blocks reachable from `bb0` in reverse postorder.
    pub fn reverse_postorder(&self) -> Vec<BlockId> {
        self.postorder.iter().rev().copied().collect()
    }
}

/// The blocks of `body` reachable from `bb0`, in postorder of a depth-first search that visits
/// successors in order.
pub fn postorder(body: &Body) -> Vec<BlockId> {
    let count = body.blocks.len();
    let mut visited = vec![false; count];
    let mut order = Vec::with_capacity(count);
    if count == 0 {
        return order;
    }
    // Each frame is a block and the index of its next successor to visit.
    let mut stack = vec![(BlockId(0), 0usize)];
    visited[0] = true;
    while let Some((block, next)) = stack.last_mut() {
        if let Some(target) = successor(&body.blocks[*block].terminator.kind, *next) {
            *next += 1;
            if !visited[target.index()] {
                visited[target.index()] = true;
                stack.push((target, 0));
            }
        } else {
            order.push(*block);
            stack.pop();
        }
    }
    order
}

/// The blocks of `body` reachable from `bb0` in reverse postorder: every block comes after its
/// predecessors, except along back edges.
pub fn reverse_postorder(body: &Body) -> Vec<BlockId> {
    let mut order = postorder(body);
    order.reverse();
    order
}

/// The predecessors of each block of `body`, in block order, once per edge.
pub fn predecessors(body: &Body) -> Vec<Vec<BlockId>> {
    let mut predecessors = vec![Vec::new(); body.blocks.len()];
    for (block, data) in body.blocks.iter() {
        for successor in successors(&data.terminator.kind) {
            predecessors[successor.index()].push(block);
        }
    }
    predecessors
}

/// Every location of `body`, block by block in reverse postorder, each block's statements then
/// its terminator.
pub fn locations(body: &Body) -> Vec<Location> {
    let order = reverse_postorder(body);
    let count = order
        .iter()
        .map(|block| body.blocks[*block].statements.len() + 1)
        .sum();
    let mut locations = Vec::with_capacity(count);
    for block in order {
        for statement in 0..=body.blocks[block].statements.len() {
            locations.push(Location { block, statement });
        }
    }
    locations
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_bodies::body_with_edges;

    #[test]
    fn a_graph_has_the_body_s_edges_and_postorder() {
        // A switch back to the head, an unreachable block and a block reached twice.
        let body = body_with_edges(&[&[1], &[3, 2, 1], &[1], &[], &[3], &[2, 2]]);
        let graph = Graph::new(&body);
        assert_eq!(graph.block_count(), 6);
        for (block, data) in body.blocks.iter() {
            assert_eq!(
                graph.successors(block),
                data.terminator.kind.successors().as_slice()
            );
            assert_eq!(
                graph.predecessors(block),
                predecessors(&body)[block.index()].as_slice()
            );
            assert_eq!(
                successors(&data.terminator.kind).collect::<Vec<_>>(),
                data.terminator.kind.successors()
            );
        }
        assert_eq!(
            graph.predecessors(BlockId(2)),
            &[BlockId(1), BlockId(5), BlockId(5)]
        );
        assert_eq!(graph.postorder(), postorder(&body).as_slice());
        assert_eq!(graph.reverse_postorder(), reverse_postorder(&body));
        assert!(!graph.postorder().contains(&BlockId(4)));
    }
}
