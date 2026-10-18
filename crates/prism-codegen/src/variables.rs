//! Which locals held in Cranelift variables share one (D20 of `collections`, a fix of the
//! lowering's compile time).
//!
//! Cranelift's SSA builder keeps, for every variable, a map indexed by block, which grows to the
//! highest block the variable is defined or looked up in. A body with one variable per loop
//! counter, call result or user local therefore costs the number of variables times the number
//! of blocks: thousands of loops in one body lowered in quadratic time. Locals whose live blocks
//! never meet hold no value at the same time, so they can share a variable: on every path from
//! a definition of one to its uses, no other local of the variable is written, and Cranelift's
//! SSA construction gives each use its own local's definition.
//!
//! A local's blocks are those that mention it and those it is live into, found by walking back
//! from each block where it is read before being written, up to the blocks that write it.
//! Only locals live in a few blocks share variables (loop counters, call results, the locals
//! of one statement or loop); a local live longer keeps its own, as before, and so costs
//! nothing to find. Locals are given variables in order, each one of the few variables of its machine type last
//! given a local, when no local holding it occupies any of its blocks, and a new one otherwise.
//! The walk costs each local's live blocks, and the assignment a constant number of lookups per
//! live block, so the whole is linear in the sum of the locals' live blocks.

use cranelift_codegen::ir::Type as MachineType;
use prism_mir::{
    Body, Callee, Capture, LocalId, Operand, Place, Projection, Rvalue, StatementKind,
    TerminatorKind,
};
use std::collections::{HashMap, HashSet};

/// How many of the variables last given a local of one type a local may share.
const REUSE_CANDIDATES: usize = 16;

/// How many blocks a local may be live in and still share a variable.
const SHARED_LIVE_BLOCKS: usize = 16;

/// The variable each candidate local shares, by index into the returned machine types: one
/// entry per local, `None` for a local that is not a candidate. A candidate is a local held in
/// a variable of the machine type `candidates` gives it.
pub(crate) fn shared_variables(
    body: &Body,
    candidates: &[Option<MachineType>],
) -> (Vec<Option<usize>>, Vec<MachineType>) {
    let count = body.blocks.len();
    // For each block, the candidates it mentions, each with whether it is read before it is
    // wholly written there (live into the block).
    let mut mentions: Vec<Vec<(LocalId, bool)>> = vec![Vec::new(); count];
    let mut predecessors: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (block, data) in body.blocks.iter() {
        let index = block.0 as usize;
        let mut seen: HashMap<LocalId, bool> = HashMap::new();
        let mut order = Vec::new();
        let mut note = |local: LocalId, written: bool| {
            if candidates[local.0 as usize].is_none() {
                return;
            }
            seen.entry(local).or_insert_with(|| {
                order.push(local);
                !written
            });
        };
        for statement in &data.statements {
            match &statement.kind {
                StatementKind::Assign(place, rvalue) => {
                    rvalue_reads(rvalue, &mut |local| note(local, false));
                    write(place, &mut note);
                }
                StatementKind::Read(place) => place_reads(place, &mut |local| note(local, false)),
                StatementKind::Destructure { source, parts, .. } => {
                    note(*source, false);
                    for (_, target) in parts {
                        note(*target, true);
                    }
                }
                StatementKind::StorageLive(_)
                | StatementKind::StorageDead(_)
                | StatementKind::GroupStart(_)
                | StatementKind::GroupEnd(_) => {}
            }
        }
        match &data.terminator.kind {
            TerminatorKind::SwitchInt { value, .. } => {
                operand_reads(value, &mut |local| note(local, false))
            }
            TerminatorKind::Call {
                callee,
                arguments,
                destination,
                ..
            } => {
                if let Callee::Value(place, _) = callee {
                    place_reads(place, &mut |local| note(local, false));
                }
                for argument in arguments {
                    operand_reads(argument, &mut |local| note(local, false));
                }
                write(destination, &mut note);
            }
            TerminatorKind::Drop { place, .. } => {
                place_reads(place, &mut |local| note(local, false))
            }
            TerminatorKind::Remembered {
                composer,
                destination,
                ..
            } => {
                operand_reads(composer, &mut |local| note(local, false));
                write(destination, &mut note);
            }
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
        }
        mentions[index] = order
            .into_iter()
            .map(|local| (local, seen[&local]))
            .collect();
        for successor in data.terminator.kind.successors() {
            predecessors[successor.0 as usize].push(index);
        }
    }
    // Each candidate's blocks: its mentions, then the blocks it is live into or through.
    let mut blocks: Vec<HashSet<usize>> = vec![HashSet::new(); candidates.len()];
    let mut writes: Vec<HashSet<usize>> = vec![HashSet::new(); candidates.len()];
    let mut exposed: Vec<Vec<usize>> = vec![Vec::new(); candidates.len()];
    for (block, mentioned) in mentions.iter().enumerate() {
        for (local, read_first) in mentioned {
            let local = local.0 as usize;
            blocks[local].insert(block);
            if *read_first {
                exposed[local].push(block);
            } else {
                writes[local].insert(block);
            }
        }
    }
    // A local live over more blocks than [`SHARED_LIVE_BLOCKS`] keeps a variable of its own:
    // finding the blocks of every long-lived local would cost their total, quadratic in a body
    // of many locals live to its end, which its own variable does not.
    let mut long_lived = vec![false; candidates.len()];
    for local in 0..candidates.len() {
        let mut pending = std::mem::take(&mut exposed[local]);
        let mut visited: HashSet<usize> = pending.iter().copied().collect();
        'walk: while let Some(block) = pending.pop() {
            for &predecessor in &predecessors[block] {
                blocks[local].insert(predecessor);
                if blocks[local].len() > SHARED_LIVE_BLOCKS {
                    long_lived[local] = true;
                    break 'walk;
                }
                // A block that writes the local first ends its live range going back.
                if !writes[local].contains(&predecessor) && visited.insert(predecessor) {
                    pending.push(predecessor);
                }
            }
        }
        if blocks[local].len() > SHARED_LIVE_BLOCKS {
            long_lived[local] = true;
        }
    }
    // A variable of the local's type that no local given one occupies in its blocks, among the
    // few last given one; otherwise a new variable. Looking at only those keeps the assignment
    // linear when many locals live at once, and finds the variables of the loop or statement
    // just before, whose blocks are behind.
    let mut occupied: Vec<HashSet<usize>> = vec![HashSet::new(); count];
    let mut types: Vec<MachineType> = Vec::new();
    let mut recent: HashMap<MachineType, Vec<usize>> = HashMap::new();
    let mut classes = vec![None; candidates.len()];
    for (local, machine) in candidates.iter().enumerate() {
        let Some(machine) = machine else {
            continue;
        };
        if long_lived[local] {
            types.push(*machine);
            classes[local] = Some(types.len() - 1);
            continue;
        }
        let recent = recent.entry(*machine).or_default();
        let free = recent
            .iter()
            .rev()
            .take(REUSE_CANDIDATES)
            .position(|class| {
                blocks[local]
                    .iter()
                    .all(|block| !occupied[*block].contains(class))
            });
        let class = match free {
            Some(position) => recent.remove(recent.len() - 1 - position),
            None => {
                types.push(*machine);
                types.len() - 1
            }
        };
        recent.push(class);
        for block in &blocks[local] {
            occupied[*block].insert(class);
        }
        classes[local] = Some(class);
    }
    (classes, types)
}

/// Notes a write of `place`: a definition of a whole local, or a read of the local whose part
/// it writes, and of the locals its indices read.
fn write(place: &Place, note: &mut impl FnMut(LocalId, bool)) {
    if place.projections.is_empty() {
        note(place.local, true);
    } else {
        place_reads(place, &mut |local| note(local, false));
    }
}

/// Calls `read` with the locals `place` reads: its own and those its indices read.
fn place_reads(place: &Place, read: &mut impl FnMut(LocalId)) {
    for projection in &place.projections {
        if let Projection::Index(index) = projection {
            read(*index);
        }
    }
    read(place.local);
}

fn operand_reads(operand: &Operand, read: &mut impl FnMut(LocalId)) {
    if let Some(place) = operand.place() {
        place_reads(place, read);
    }
}

fn rvalue_reads(rvalue: &Rvalue, read: &mut impl FnMut(LocalId)) {
    match rvalue {
        Rvalue::Use(operand)
        | Rvalue::Unary(_, operand)
        | Rvalue::WrapNullable(operand)
        | Rvalue::Subtype(operand) => operand_reads(operand, read),
        Rvalue::Binary(_, left, right) | Rvalue::Bits(_, left, right) => {
            operand_reads(left, read);
            operand_reads(right, read);
        }
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                operand_reads(operand, read);
            }
        }
        Rvalue::Borrow { place, .. } | Rvalue::Discriminant(place) => place_reads(place, read),
        Rvalue::Closure { captures, .. } => {
            for place in captures.iter().filter_map(Capture::place) {
                place_reads(place, read);
            }
        }
        Rvalue::Template(_) => {
            for operand in rvalue.template_operands() {
                operand_reads(operand, read);
            }
        }
    }
}
