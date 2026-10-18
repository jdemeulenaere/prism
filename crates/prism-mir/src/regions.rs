//! The open regions of a composable body (D6 of `compose-transform`): one forward pass in block
//! order from `bb0`, keeping the stack of regions open at each block's entry. A body's markers
//! are well nested when every region starts inside its parent, ends where it is the innermost
//! open one, every join is reached with the same stack, and nothing is open at a return. The
//! validator checks that, and the compose transform reads the stacks to know which region each
//! call is in.

use crate::mir::{BlockId, Body, GroupId, StatementKind, TerminatorKind};
use prism_typed_tree::ArenaId;

/// The regions open at the entry of each block of `body`, innermost last, `None` for a block
/// not reached from `bb0`; or the first violation of the markers' nesting, as a message.
pub fn open_regions(body: &Body) -> Result<Vec<Option<Vec<GroupId>>>, String> {
    let mut entry: Vec<Option<Vec<GroupId>>> = vec![None; body.blocks.len()];
    if body.blocks.is_empty() {
        return Ok(entry);
    }
    entry[0] = Some(Vec::new());
    // Blocks are visited in index order, again whenever an earlier block reaches a later one
    // first: lowering creates blocks mostly in order, so this is one pass in practice.
    let mut work = vec![BlockId(0)];
    while let Some(block) = work.pop() {
        let mut stack = entry[block.index()].clone().expect("a reached block");
        let data = &body.blocks[block];
        for (index, statement) in data.statements.iter().enumerate() {
            match statement.kind {
                StatementKind::GroupStart(group) => {
                    let parent = body.groups.get(group).map(|region| region.parent);
                    match parent {
                        None => {
                            return Err(format!(
                                "bb{}[{}] starts g{}, which is not a region",
                                block.0, index, group.0
                            ))
                        }
                        Some(parent) if parent != stack.last().copied() => {
                            return Err(format!(
                                "bb{}[{}] starts g{} outside its parent region",
                                block.0, index, group.0
                            ))
                        }
                        Some(_) => stack.push(group),
                    }
                }
                StatementKind::GroupEnd(group) => {
                    if stack.last() != Some(&group) {
                        return Err(format!(
                            "bb{}[{}] ends g{}, which is not the innermost open region",
                            block.0, index, group.0
                        ));
                    }
                    stack.pop();
                }
                _ => {}
            }
        }
        if matches!(data.terminator.kind, TerminatorKind::Return) && !stack.is_empty() {
            return Err(format!(
                "bb{} returns while g{} is open",
                block.0,
                stack.last().unwrap().0
            ));
        }
        for successor in data.terminator.kind.successors() {
            match &entry[successor.index()] {
                Some(existing) if *existing != stack => {
                    return Err(format!(
                        "bb{} is reached with different open regions",
                        successor.0
                    ));
                }
                Some(_) => {}
                None => {
                    entry[successor.index()] = Some(stack.clone());
                    work.push(successor);
                }
            }
        }
    }
    Ok(entry)
}

/// Whether `body` holds a region marker.
pub fn has_markers(body: &Body) -> bool {
    body.blocks.iter().any(|(_, data)| {
        data.statements.iter().any(|statement| {
            matches!(
                statement.kind,
                StatementKind::GroupStart(_) | StatementKind::GroupEnd(_)
            )
        })
    })
}
