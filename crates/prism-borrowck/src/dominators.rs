//! Dominators of a body's blocks and the back edges they define (D7 of `borrowck-moves`).
//!
//! Computed by the algorithm of Lengauer and Tarjan, with path compression, in time almost
//! linear in the body's edges however deep the dominator tree is (the iterative algorithm of
//! Cooper, Harvey and Kennedy walks the tree once per predecessor of a merge, quadratic in a
//! body of many sequential branches). MIR lowered from structured source is reducible, so every
//! cycle enters through a block that dominates it, and the back edges are exactly the edges to a
//! dominator of their source. `borrowck-exclusivity` reuses them.

use crate::location::Graph;
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
        let graph = Graph::new(body);
        let mut order = vec![usize::MAX; count];
        for (position, block) in graph.postorder().iter().rev().enumerate() {
            order[block.index()] = position;
        }
        let immediate = if count == 0 {
            Vec::new()
        } else {
            lengauer_tarjan(&graph)
        };
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

/// The immediate dominator of each block of `body` reachable from `bb0` (`bb0` its own), by
/// Lengauer and Tarjan's algorithm: semidominators computed in reverse depth-first order over a
/// forest linked as it goes, evaluated with path compression, then corrected in depth-first
/// order. Every step is iterative, so a deep tree cannot overflow the stack.
fn lengauer_tarjan(graph: &Graph) -> Vec<Option<BlockId>> {
    const NONE: usize = usize::MAX;
    let count = graph.block_count();
    // The depth-first preorder: the number of each block, the block of each number and the
    // number of each block's parent in the depth-first tree.
    let mut number = vec![NONE; count];
    let mut vertex: Vec<usize> = Vec::with_capacity(count);
    let mut parent: Vec<usize> = Vec::with_capacity(count);
    let mut stack: Vec<(usize, usize)> = vec![(0, NONE)];
    while let Some((block, from)) = stack.pop() {
        if number[block] != NONE {
            continue;
        }
        number[block] = vertex.len();
        vertex.push(block);
        parent.push(from);
        for successor in graph.successors(BlockId(block as u32)).iter().rev() {
            if number[successor.index()] == NONE {
                stack.push((successor.index(), number[block]));
            }
        }
    }
    let reached = vertex.len();
    // By number: the semidominator, the forest's ancestor and the label of the smallest
    // semidominator on the compressed path, and the numbers whose semidominator each one is.
    let mut semi: Vec<usize> = (0..reached).collect();
    let mut ancestor = vec![NONE; reached];
    let mut label: Vec<usize> = (0..reached).collect();
    let mut bucket: Vec<Vec<usize>> = vec![Vec::new(); reached];
    let mut idom = vec![NONE; reached];
    let mut path = Vec::new();
    // The number with the smallest semidominator on the forest path from `v` to its root.
    let mut eval = |v: usize, ancestor: &mut Vec<usize>, label: &mut Vec<usize>, semi: &[usize]| {
        if ancestor[v] == NONE {
            return v;
        }
        path.clear();
        let mut x = v;
        while ancestor[ancestor[x]] != NONE {
            path.push(x);
            x = ancestor[x];
        }
        for &x in path.iter().rev() {
            let up = ancestor[x];
            if semi[label[up]] < semi[label[x]] {
                label[x] = label[up];
            }
            ancestor[x] = ancestor[up];
        }
        label[v]
    };
    for w in (1..reached).rev() {
        for predecessor in graph.predecessors(BlockId(vertex[w] as u32)) {
            let v = number[predecessor.index()];
            if v == NONE {
                continue;
            }
            let u = eval(v, &mut ancestor, &mut label, &semi);
            if semi[u] < semi[w] {
                semi[w] = semi[u];
            }
        }
        bucket[semi[w]].push(w);
        let p = parent[w];
        ancestor[w] = p;
        for v in std::mem::take(&mut bucket[p]) {
            let u = eval(v, &mut ancestor, &mut label, &semi);
            idom[v] = if semi[u] < semi[v] { u } else { p };
        }
    }
    for w in 1..reached {
        if idom[w] != semi[w] {
            idom[w] = idom[idom[w]];
        }
    }
    let mut immediate = vec![None; count];
    immediate[0] = Some(BlockId(0));
    for w in 1..reached {
        immediate[vertex[w]] = Some(BlockId(vertex[idom[w]] as u32));
    }
    immediate
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

    /// Whether `a` dominates `b` by definition: `b` is unreachable from `bb0` without `a`.
    fn dominates_by_definition(body: &Body, a: usize, b: usize) -> bool {
        if a == b || a == 0 {
            return true;
        }
        let mut seen = vec![false; body.blocks.len()];
        let mut stack = vec![0usize];
        while let Some(block) = stack.pop() {
            if block == a || seen[block] {
                continue;
            }
            seen[block] = true;
            for successor in body.blocks[BlockId(block as u32)]
                .terminator
                .kind
                .successors()
            {
                stack.push(successor.index());
            }
        }
        !seen[b]
    }

    #[test]
    fn dominance_matches_its_definition() {
        let bodies = [
            // A chain of diamonds, a loop with two exits, and an unreachable block.
            body_with_edges(&[&[1, 2], &[3], &[3], &[4, 5], &[6], &[6], &[]]),
            body_with_edges(&[&[1], &[2, 4], &[3, 1], &[1, 4], &[5], &[]]),
            body_with_edges(&[&[1, 2], &[3], &[1, 3], &[], &[3]]),
            // Edges into the middle of a cycle from both sides.
            body_with_edges(&[&[1, 2], &[2, 3], &[1, 3], &[0]]),
        ];
        for body in &bodies {
            let dominators = Dominators::compute(body);
            let reachable: Vec<usize> = crate::location::reverse_postorder(body)
                .iter()
                .map(|b| b.index())
                .collect();
            for &a in &reachable {
                for &b in &reachable {
                    assert_eq!(
                        dominators.dominates(BlockId(a as u32), BlockId(b as u32)),
                        dominates_by_definition(body, a, b),
                        "bb{} dominates bb{}",
                        a,
                        b
                    );
                }
            }
        }
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
