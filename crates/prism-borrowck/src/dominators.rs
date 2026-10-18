//! Dominators of a body's blocks and the back edges they define (D7 of `borrowck-moves`).
//!
//! Computed by the iterative algorithm of Cooper, Harvey and Kennedy over reverse postorder. MIR
//! lowered from structured source is reducible, so every cycle enters through a block that
//! dominates it, and the back edges are exactly the edges to a dominator of their source.
//! `borrowck-exclusivity` reuses them.

use crate::location::{predecessors, reverse_postorder};
use prism_mir::{BlockId, Body};
use prism_typed_tree::ArenaId;

/// The immediate dominator of each block reachable from `bb0`.
pub struct Dominators {
    /// The immediate dominator of each block; `bb0` is its own, an unreachable block has none.
    immediate: Vec<Option<BlockId>>,
    /// The position of each block in reverse postorder.
    order: Vec<usize>,
    /// The interval of each reachable block in a depth-first walk of the dominator tree: when
    /// it is entered and left. A block dominates another exactly when its interval contains the
    /// other's, which answers [`Self::dominates`] in constant time however deep the tree is.
    intervals: Vec<(usize, usize)>,
}

impl Dominators {
    pub fn compute(body: &Body) -> Self {
        let count = body.blocks.len();
        let rpo = reverse_postorder(body);
        let mut order = vec![usize::MAX; count];
        for (position, block) in rpo.iter().enumerate() {
            order[block.index()] = position;
        }
        let predecessors = predecessors(body);
        let mut immediate: Vec<Option<BlockId>> = vec![None; count];
        if count == 0 {
            return Self {
                immediate,
                order,
                intervals: Vec::new(),
            };
        }
        immediate[0] = Some(BlockId(0));
        let mut changed = true;
        while changed {
            changed = false;
            for &block in rpo.iter().skip(1) {
                let mut new = None;
                for &predecessor in &predecessors[block.index()] {
                    if immediate[predecessor.index()].is_none() {
                        continue;
                    }
                    new = Some(match new {
                        None => predecessor,
                        Some(current) => intersect(&immediate, &order, predecessor, current),
                    });
                }
                if new.is_some() && immediate[block.index()] != new {
                    immediate[block.index()] = new;
                    changed = true;
                }
            }
        }
        let intervals = tree_intervals(&immediate);
        Self {
            immediate,
            order,
            intervals,
        }
    }

    /// The immediate dominator of `block`, `None` for `bb0` and unreachable blocks.
    pub fn immediate_dominator(&self, block: BlockId) -> Option<BlockId> {
        match self.immediate[block.index()] {
            Some(dominator) if dominator != block => Some(dominator),
            _ => None,
        }
    }

    /// Whether `a` dominates `b`: every path from `bb0` to `b` passes through `a`.
    pub fn dominates(&self, a: BlockId, b: BlockId) -> bool {
        if self.immediate[a.index()].is_none() || self.immediate[b.index()].is_none() {
            return false;
        }
        let (a, b) = (self.intervals[a.index()], self.intervals[b.index()]);
        a.0 <= b.0 && b.1 <= a.1
    }

    /// Whether the edge `from -> to` is a back edge: one to a block that dominates its source.
    pub fn is_back_edge(&self, from: BlockId, to: BlockId) -> bool {
        self.dominates(to, from)
    }

    /// The position of `block` in reverse postorder.
    pub fn reverse_postorder_index(&self, block: BlockId) -> usize {
        self.order[block.index()]
    }
}

/// The interval of each block reachable from `bb0` in an iterative depth-first walk of the
/// dominator tree given by `immediate`: the times the walk enters and leaves it.
fn tree_intervals(immediate: &[Option<BlockId>]) -> Vec<(usize, usize)> {
    let count = immediate.len();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (block, dominator) in immediate.iter().enumerate() {
        if let Some(dominator) = dominator {
            if dominator.index() != block {
                children[dominator.index()].push(block);
            }
        }
    }
    let mut intervals = vec![(0, 0); count];
    let mut time = 0;
    // Each frame is a block and the index of its next child to visit.
    let mut stack = vec![(0usize, 0usize)];
    intervals[0].0 = time;
    while let Some((block, next)) = stack.last_mut() {
        if let Some(&child) = children[*block].get(*next) {
            *next += 1;
            time += 1;
            intervals[child].0 = time;
            stack.push((child, 0));
        } else {
            time += 1;
            intervals[*block].1 = time;
            stack.pop();
        }
    }
    intervals
}

/// The nearest common dominator of `a` and `b`, walking up from the later of the two in reverse
/// postorder.
fn intersect(immediate: &[Option<BlockId>], order: &[usize], a: BlockId, b: BlockId) -> BlockId {
    let (mut a, mut b) = (a, b);
    while a != b {
        while order[a.index()] > order[b.index()] {
            a = immediate[a.index()].expect("a processed block has a dominator");
        }
        while order[b.index()] > order[a.index()] {
            b = immediate[b.index()].expect("a processed block has a dominator");
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_bodies::body_with_edges;

    fn back_edges(body: &Body) -> Vec<(u32, u32)> {
        let dominators = Dominators::compute(body);
        let mut edges = Vec::new();
        for (block, data) in body.blocks.iter() {
            for target in data.terminator.kind.successors() {
                if dominators.is_back_edge(block, target) {
                    edges.push((block.0, target.0));
                }
            }
        }
        edges
    }

    #[test]
    fn a_diamond_has_no_back_edge() {
        // bb0 -> {bb1, bb2} -> bb3
        let body = body_with_edges(&[&[1, 2], &[3], &[3], &[]]);
        let dominators = Dominators::compute(&body);
        assert_eq!(dominators.immediate_dominator(BlockId(3)), Some(BlockId(0)));
        assert_eq!(dominators.immediate_dominator(BlockId(1)), Some(BlockId(0)));
        assert_eq!(dominators.immediate_dominator(BlockId(0)), None);
        assert!(!dominators.dominates(BlockId(1), BlockId(3)));
        assert!(dominators.dominates(BlockId(0), BlockId(3)));
        assert_eq!(back_edges(&body), vec![]);
    }

    #[test]
    fn a_while_loop_has_one_back_edge() {
        // bb0 -> bb1 (head) -> {bb3 (exit), bb2 (body) -> bb1}
        let body = body_with_edges(&[&[1], &[3, 2], &[1], &[]]);
        assert_eq!(back_edges(&body), vec![(2, 1)]);
        let dominators = Dominators::compute(&body);
        assert_eq!(dominators.immediate_dominator(BlockId(3)), Some(BlockId(1)));
    }

    #[test]
    fn nested_loops_have_a_back_edge_each() {
        // bb0 -> bb1 (outer head) -> {bb5 (exit), bb2 (inner head)}
        // bb2 -> {bb4 (outer latch) -> bb1, bb3 (inner body) -> bb2}
        let body = body_with_edges(&[&[1], &[5, 2], &[4, 3], &[2], &[1], &[]]);
        assert_eq!(back_edges(&body), vec![(3, 2), (4, 1)]);
        let dominators = Dominators::compute(&body);
        assert!(dominators.dominates(BlockId(1), BlockId(3)));
        assert!(!dominators.dominates(BlockId(3), BlockId(4)));
    }
}
