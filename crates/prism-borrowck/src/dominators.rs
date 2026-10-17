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
            return Self { immediate, order };
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
        Self { immediate, order }
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
        if self.immediate[b.index()].is_none() {
            return false;
        }
        let mut current = b;
        loop {
            if current == a {
                return true;
            }
            match self.immediate_dominator(current) {
                Some(next) => current = next,
                None => return false,
            }
        }
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
