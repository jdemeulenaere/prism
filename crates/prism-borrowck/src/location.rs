//! Locations in a body's control-flow graph, and the block orders the analyses visit.

use prism_mir::{BlockId, Body};
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
        let successors = body.blocks[*block].terminator.kind.successors();
        if let Some(&successor) = successors.get(*next) {
            *next += 1;
            if !visited[successor.index()] {
                visited[successor.index()] = true;
                stack.push((successor, 0));
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
        for successor in data.terminator.kind.successors() {
            predecessors[successor.index()].push(block);
        }
    }
    predecessors
}

/// Every location of `body`, block by block in reverse postorder, each block's statements then
/// its terminator.
pub fn locations(body: &Body) -> Vec<Location> {
    let mut locations = Vec::new();
    for block in reverse_postorder(body) {
        for statement in 0..=body.blocks[block].statements.len() {
            locations.push(Location { block, statement });
        }
    }
    locations
}
