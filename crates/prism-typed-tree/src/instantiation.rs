//! The instantiation graph and its growing cycles (D8 and D9 of `monomorphization`).
//!
//! A generic needs infinitely many instances exactly when it reaches itself with a type argument
//! that holds its own type parameter nested in another type: `fun <T> f(x: T) { f(Box(x)) }`
//! needs `f<Int>`, `f<Box<Int>>`, `f<Box<Box<Int>>>` and so on. The graph has a node per type
//! parameter of a generic (a function, an extension or a struct or enum). A use of generic `g`
//! with type arguments `A1..An` inside generic `f` adds, for each type parameter `i` of `f`
//! occurring in `Aj`, an edge from (`f`, `i`) to (`g`, `j`). The edge grows unless `Aj` is the
//! parameter itself or its nullable, which flattening keeps at one level. A strongly connected
//! component holding a growing edge is a polymorphic recursion. This is Go's "instantiation
//! cycle" check; it is linear in the number of edges, and shared by the type checker (`E4027`,
//! one file) and the backend (`E6004`, a program's import cycles).

use crate::tree::DeclarationPath;
use crate::types::Type;
use std::collections::{BTreeSet, HashMap, VecDeque};

/// A type parameter of a generic, by its position: `generic` is the generic's identity (its
/// symbol, or its type's qualified name) and `parameter` the index of the parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstantiationNode {
    pub generic: String,
    pub parameter: u32,
}

/// One use of a type parameter in a type argument: `from`'s parameter occurs in `argument`, the
/// argument passed for `to`'s parameter, at the use `site`, an index the caller gives meaning to
/// (a call, or a field holding a generic type).
#[derive(Debug, Clone, PartialEq)]
pub struct InstantiationEdge {
    pub from: InstantiationNode,
    pub to: InstantiationNode,
    /// Whether `argument` holds the parameter nested in another type.
    pub grows: bool,
    pub argument: Type,
    pub site: usize,
}

/// Whether passing `argument` for a type parameter grows the type parameter `parameter` of the
/// user: false exactly when `argument` is that parameter or its nullable.
pub fn grows(argument: &Type, parameter: u32) -> bool {
    let bare = match argument {
        Type::Nullable(inner) => inner.as_ref(),
        other => other,
    };
    !matches!(bare, Type::Parameter(p) if p.index == parameter)
}

/// The name of the generic function or extension `path` in the instantiation graph: its module,
/// its receiver's constructor for an extension, and its name, as the backend's symbols write it
/// (`lib/box::lib/box.Box.get`).
pub fn generic_name(path: &DeclarationPath) -> String {
    match &path.receiver {
        Some(receiver) => format!(
            "{}::{}.{}",
            path.module,
            receiver.receiver_constructor(),
            path.name
        ),
        None => format!("{}::{}", path.module, path.name),
    }
}

/// The type parameters `ty` mentions, function types included, in index order.
pub fn mentioned_parameters(ty: &Type) -> BTreeSet<u32> {
    fn walk(ty: &Type, out: &mut BTreeSet<u32>) {
        match ty {
            Type::Parameter(parameter) => {
                out.insert(parameter.index);
            }
            Type::Reference { inner, .. } | Type::Nullable(inner) | Type::Heap(inner) => {
                walk(inner, out)
            }
            Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
                arguments.iter().for_each(|argument| walk(argument, out))
            }
            Type::Fn { params, ret, .. } => {
                params.iter().for_each(|param| walk(param, out));
                walk(ret, out);
            }
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(ty, &mut out);
    out
}

/// The edges of one use of `callee` with `arguments` inside `generic`, at `site`.
pub fn edges_of(
    generic: &str,
    callee: &str,
    arguments: &[Type],
    site: usize,
) -> Vec<InstantiationEdge> {
    let mut edges = Vec::new();
    for (j, argument) in arguments.iter().enumerate() {
        for i in mentioned_parameters(argument) {
            edges.push(InstantiationEdge {
                from: InstantiationNode {
                    generic: generic.to_string(),
                    parameter: i,
                },
                to: InstantiationNode {
                    generic: callee.to_string(),
                    parameter: j as u32,
                },
                grows: grows(argument, i),
                argument: argument.clone(),
                site,
            });
        }
    }
    edges
}

/// The first growing cycle of `edges`, if any: see [`growing_cycles`].
pub fn growing_cycle(edges: &[InstantiationEdge]) -> Option<Vec<InstantiationEdge>> {
    growing_cycles(edges).into_iter().next()
}

/// One growing cycle per strongly connected component of `edges` that holds a growing edge, in
/// order of that edge. A cycle starts with the component's first growing edge and follows a
/// shortest path from its target back to its source. Linear in the number of edges.
pub fn growing_cycles(edges: &[InstantiationEdge]) -> Vec<Vec<InstantiationEdge>> {
    if !edges.iter().any(|edge| edge.grows) {
        return Vec::new();
    }
    // Number the nodes in order of first appearance, so the result is deterministic.
    let mut index: HashMap<&InstantiationNode, usize> = HashMap::new();
    for edge in edges {
        for node in [&edge.from, &edge.to] {
            let next = index.len();
            index.entry(node).or_insert(next);
        }
    }
    let count = index.len();
    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (position, edge) in edges.iter().enumerate() {
        successors[index[&edge.from]].push(position);
    }
    let component = strongly_connected_components(count, &successors, edges, &index);
    let mut reported = vec![false; count];
    let mut cycles = Vec::new();
    for edge in edges {
        let (from, to) = (index[&edge.from], index[&edge.to]);
        if !edge.grows || component[from] != component[to] || reported[component[from]] {
            continue;
        }
        reported[component[from]] = true;
        // A shortest path from `to` back to `from` inside the component.
        let mut previous: Vec<Option<usize>> = vec![None; count];
        let mut seen = vec![false; count];
        let mut queue = VecDeque::from([to]);
        seen[to] = true;
        while let Some(node) = queue.pop_front() {
            if node == from {
                break;
            }
            for &position in &successors[node] {
                let next = index[&edges[position].to];
                if component[next] == component[from] && !seen[next] {
                    seen[next] = true;
                    previous[next] = Some(position);
                    queue.push_back(next);
                }
            }
        }
        let mut path = Vec::new();
        let mut node = from;
        while node != to {
            let position = previous[node].expect("a component's nodes reach each other");
            path.push(edges[position].clone());
            node = index[&edges[position].from];
        }
        path.reverse();
        let mut cycle = vec![edge.clone()];
        cycle.extend(path);
        cycles.push(cycle);
    }
    cycles
}

/// The component of each node, by Tarjan's algorithm without recursion.
fn strongly_connected_components(
    count: usize,
    successors: &[Vec<usize>],
    edges: &[InstantiationEdge],
    index: &HashMap<&InstantiationNode, usize>,
) -> Vec<usize> {
    const UNVISITED: usize = usize::MAX;
    let mut order = vec![UNVISITED; count];
    let mut low = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut component = vec![UNVISITED; count];
    let mut stack = Vec::new();
    let mut next_order = 0;
    let mut next_component = 0;
    for root in 0..count {
        if order[root] != UNVISITED {
            continue;
        }
        // Each frame is a node and the position of its next successor edge to look at.
        let mut frames = vec![(root, 0usize)];
        order[root] = next_order;
        low[root] = next_order;
        next_order += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (node, ref mut cursor)) = frames.last_mut() {
            if let Some(&position) = successors[node].get(*cursor) {
                *cursor += 1;
                let next = index[&edges[position].to];
                if order[next] == UNVISITED {
                    order[next] = next_order;
                    low[next] = next_order;
                    next_order += 1;
                    stack.push(next);
                    on_stack[next] = true;
                    frames.push((next, 0));
                } else if on_stack[next] {
                    low[node] = low[node].min(order[next]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent] = low[parent].min(low[node]);
            }
            if low[node] == order[node] {
                loop {
                    let member = stack.pop().expect("the node is on the stack");
                    on_stack[member] = false;
                    component[member] = next_component;
                    if member == node {
                        break;
                    }
                }
                next_component += 1;
            }
        }
    }
    component
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{NominalId, TypeParameter};

    fn parameter(index: u32) -> Type {
        Type::Parameter(TypeParameter {
            index,
            name: format!("T{}", index),
            bound: None,
        })
    }

    fn boxed(inner: Type) -> Type {
        Type::Struct {
            id: NominalId::new("main", "Box"),
            arguments: vec![inner],
        }
    }

    fn generics(cycle: &[InstantiationEdge]) -> Vec<&str> {
        cycle
            .iter()
            .map(|edge| edge.from.generic.as_str())
            .collect()
    }

    #[test]
    fn a_self_edge_that_grows() {
        let edges = edges_of("f", "f", &[boxed(parameter(0))], 0);
        let cycle = growing_cycle(&edges).expect("a cycle");
        assert_eq!(generics(&cycle), ["f"]);
        assert!(cycle[0].grows);
    }

    #[test]
    fn a_two_node_cycle_with_one_growing_edge() {
        let mut edges = edges_of("ping", "pong", &[boxed(parameter(0))], 0);
        edges.extend(edges_of("pong", "ping", &[parameter(0)], 0));
        let cycle = growing_cycle(&edges).expect("a cycle");
        assert_eq!(generics(&cycle), ["ping", "pong"]);
        assert!(cycle[0].grows && !cycle[1].grows);
    }

    #[test]
    fn a_cycle_without_growth() {
        let mut edges = edges_of("count", "count", &[parameter(0)], 0);
        edges.extend(edges_of("a", "b", &[parameter(1), parameter(0)], 0));
        edges.extend(edges_of("b", "a", &[parameter(1), parameter(0)], 0));
        assert!(growing_cycle(&edges).is_none());
    }

    #[test]
    fn a_nullable_argument_does_not_grow() {
        assert!(!grows(&parameter(0).nullable(), 0));
        assert!(grows(&parameter(1).nullable(), 0));
        let edges = edges_of("deep", "deep", &[parameter(0).nullable()], 0);
        assert!(growing_cycle(&edges).is_none());
    }

    #[test]
    fn a_growing_edge_outside_any_cycle() {
        let mut edges = edges_of("f", "g", &[boxed(parameter(0))], 0);
        edges.extend(edges_of("g", "g", &[parameter(0)], 0));
        assert!(growing_cycle(&edges).is_none());
    }

    #[test]
    fn swapped_parameters_reach_each_other() {
        // `f<A, B>` calls `f<B, Box<A>>`: (f,0) -> (f,1) grows, (f,1) -> (f,0) does not.
        let edges = edges_of("f", "f", &[parameter(1), boxed(parameter(0))], 0);
        let cycle = growing_cycle(&edges).expect("a cycle");
        assert_eq!(cycle.len(), 2);
        assert!(cycle[0].grows);
    }

    #[test]
    fn a_long_chain_is_linear() {
        // A chain of 100 000 generics closed by one growing edge: one component, no stack
        // overflow.
        let count = 100_000;
        let mut edges = Vec::new();
        for i in 0..count {
            let next = (i + 1) % count;
            let argument = if next == 0 {
                boxed(parameter(0))
            } else {
                parameter(0)
            };
            edges.extend(edges_of(
                &format!("g{}", i),
                &format!("g{}", next),
                &[argument],
                0,
            ));
        }
        let cycle = growing_cycle(&edges).expect("a cycle");
        assert_eq!(cycle.len(), count);
    }
}
