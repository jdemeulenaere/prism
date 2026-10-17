//! Drop elaboration (D12 of `borrowck-moves`): after a body passes the use check, each drop of
//! its MIR is removed where its value was moved on every path reaching it, kept where the value
//! is present on every path, and otherwise guarded by a drop flag.
//!
//! A drop flag is a `Bool` local of [`LocalKind::DropFlag`], one per local with a guarded drop,
//! without storage markers. It is set at the start of `bb0` (true for an argument or a capture),
//! set after each initialization of its local (for a call's destination, at the start of the
//! call's target, on an edge of its own when the target has other predecessors), and cleared
//! before each move of its local and after each `StorageLive` and `StorageDead` of it: so it is
//! true exactly when the local holds a value. A guarded drop becomes a switch on the flag to a
//! block holding the drop. Drops of a field of a local (the old value of `u.name = v`) and
//! through a dereference (the old referent of `*r = v`) are kept: the use check proved what
//! holds them initialized.

use crate::analyses::{MaybeInitialized, ReachingUninitializations};
use crate::dataflow::{solve, Analysis, BitSet, Results};
use crate::location::{predecessors, Location};
use crate::move_paths::{Effect, MoveData};
use prism_ast::Span;
use prism_mir::{
    merge_goto_blocks, BasicBlock, BlockId, Body, Constant, LocalDeclaration, LocalId, LocalKind,
    Operand, Place, Rvalue, Statement, StatementKind, Terminator, TerminatorKind,
};
use prism_typed_tree::{ArenaId, Type, TypeTable};
use std::collections::{BTreeMap, HashMap};

/// What elaboration does with one drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropAction {
    /// The value is present on every path: the drop always runs.
    Keep,
    /// The value was moved on every path: the drop becomes a jump to its target.
    Remove,
    /// The value is present on some paths: the drop runs when the local's flag is set.
    Guard(LocalId),
}

/// The elaborated copy of `body`, which passed the use check, with the move data and
/// initialization analyses that checked it.
pub(crate) fn elaborate_body(
    body: &Body,
    data: &MoveData,
    maybe: &Results<MaybeInitialized>,
    reaching: &Results<ReachingUninitializations>,
) -> Body {
    let actions = classify_drops(body, data, maybe, reaching);
    let mut elaborated = body.clone();
    // One flag per local with a guarded drop, in local order.
    let mut flags: BTreeMap<LocalId, LocalId> = BTreeMap::new();
    for action in actions.values() {
        if let DropAction::Guard(local) = action {
            flags.entry(*local).or_insert(LocalId(0));
        }
    }
    for (local, flag) in flags.iter_mut() {
        *flag = elaborated.locals.alloc(LocalDeclaration {
            ty: Type::Bool,
            kind: LocalKind::DropFlag,
            mutable: false,
            name: None,
            binding: None,
            span: body.locals[*local].span,
        });
    }
    let flag_of = |path| {
        flags
            .iter()
            .find(|(local, _)| data.root(**local) == path)
            .map(|(_, flag)| *flag)
    };
    let incoming = predecessors(body);
    // Assignments at the start of each block, and new blocks for split call edges.
    let mut prefixes: Vec<Vec<Statement>> = vec![Vec::new(); body.blocks.len()];
    for (local, flag) in &flags {
        let on_entry = matches!(
            body.locals[*local].kind,
            LocalKind::Argument | LocalKind::Capture
        );
        prefixes[0].push(set_flag(*flag, on_entry, body.span));
    }
    let mut new_blocks: Vec<BasicBlock> = Vec::new();
    let mut retargets: HashMap<BlockId, BlockId> = HashMap::new();
    for (block, block_data) in body.blocks.iter() {
        let mut statements = std::mem::take(&mut prefixes[block.index()]);
        for (index, statement) in block_data.statements.iter().enumerate() {
            let location = Location {
                block,
                statement: index,
            };
            let (before, after) = flag_updates(data, location, statement.span, &flag_of);
            statements.extend(before);
            statements.push(statement.clone());
            statements.extend(after);
        }
        let terminator = &block_data.terminator;
        let location = Location::terminator(body, block);
        let (before, after) = flag_updates(data, location, terminator.span, &flag_of);
        statements.extend(before);
        elaborated.blocks[block].statements = statements;
        if !after.is_empty() {
            // Only a call's destination is initialized by a terminator: its flag is set where
            // the call returns.
            let TerminatorKind::Call {
                target: Some(target),
                ..
            } = terminator.kind
            else {
                panic!("internal error: a terminator without a return initializes a local");
            };
            if incoming[target.index()].len() > 1 {
                let split = BlockId((body.blocks.len() + new_blocks.len()) as u32);
                new_blocks.push(BasicBlock {
                    statements: after,
                    terminator: Terminator {
                        kind: TerminatorKind::Goto(target),
                        span: terminator.span,
                    },
                });
                retargets.insert(block, split);
            } else if target > block {
                // Not rebuilt yet: its prefix is added when it is.
                prefixes[target.index()].extend(after);
            } else {
                elaborated.blocks[target].statements.splice(0..0, after);
            }
        }
    }
    for (block, split) in retargets {
        if let TerminatorKind::Call { target, .. } = &mut elaborated.blocks[block].terminator.kind {
            *target = Some(split);
        }
    }
    for (block, action) in &actions {
        let terminator = elaborated.blocks[*block].terminator.clone();
        let TerminatorKind::Drop { place, target } = &terminator.kind else {
            continue;
        };
        match action {
            DropAction::Keep => {}
            DropAction::Remove => {
                elaborated.blocks[*block].terminator.kind = TerminatorKind::Goto(*target)
            }
            DropAction::Guard(local) => {
                let drop = BlockId((body.blocks.len() + new_blocks.len()) as u32);
                new_blocks.push(BasicBlock {
                    statements: Vec::new(),
                    terminator: Terminator {
                        kind: TerminatorKind::Drop {
                            place: place.clone(),
                            target: *target,
                        },
                        span: terminator.span,
                    },
                });
                elaborated.blocks[*block].terminator.kind = TerminatorKind::SwitchInt {
                    value: Operand::Copy(Place::local(flags[local])),
                    targets: vec![(0, *target)],
                    otherwise: drop,
                };
            }
        }
    }
    for block in new_blocks {
        elaborated.blocks.alloc(block);
    }
    merge_goto_blocks(&mut elaborated);
    elaborated
}

/// The action for each drop of `body`, by block.
fn classify_drops(
    body: &Body,
    data: &MoveData,
    maybe: &Results<MaybeInitialized>,
    reaching: &Results<ReachingUninitializations>,
) -> BTreeMap<BlockId, DropAction> {
    let mut maybe_cursor = maybe.cursor(body);
    let mut reaching_cursor = reaching.cursor(body);
    let mut actions = BTreeMap::new();
    for (block, block_data) in body.blocks.iter() {
        let TerminatorKind::Drop { place, .. } = &block_data.terminator.kind else {
            continue;
        };
        let Some(local) = place.as_local() else {
            actions.insert(block, DropAction::Keep);
            continue;
        };
        let location = Location::terminator(body, block);
        let path = data.root(local);
        let initialized = maybe_cursor.seek(location).contains(path.index());
        let events = reaching
            .analysis
            .reaching(reaching_cursor.seek(location), path);
        if events.iter().any(|event| data.event_path(*event) != path) {
            panic!(
                "internal error: _{} is partly moved where it is dropped in {:?}",
                local.0, body.owner
            );
        }
        let action = if !initialized {
            DropAction::Remove
        } else if events.is_empty() {
            DropAction::Keep
        } else {
            DropAction::Guard(local)
        };
        actions.insert(block, action);
    }
    actions
}

/// `flag = const value`.
fn set_flag(flag: LocalId, value: bool, span: Span) -> Statement {
    Statement {
        kind: StatementKind::assign(
            Place::local(flag),
            Rvalue::Use(Operand::Constant(Constant::Bool(value))),
        ),
        span,
    }
}

/// The flag assignments before and after `location`: a move clears the moved local's flag
/// before it, an initialization sets it after, and a storage marker clears it after.
fn flag_updates(
    data: &MoveData,
    location: Location,
    span: Span,
    flag_of: &impl Fn(crate::move_paths::MovePathId) -> Option<LocalId>,
) -> (Vec<Statement>, Vec<Statement>) {
    let (mut before, mut after) = (Vec::new(), Vec::new());
    for effect in data.effects(location) {
        match effect {
            Effect::Use { .. } => {}
            Effect::Move(out) => {
                if let Some(flag) = flag_of(data.moves[*out].path) {
                    before.push(set_flag(flag, false, span));
                }
            }
            Effect::Init(init) => {
                if let Some(flag) = flag_of(data.inits[*init].path) {
                    after.push(set_flag(flag, true, span));
                }
            }
            Effect::StorageLive(start) => {
                let local = data.storage_starts[*start].local;
                if let Some(flag) = flag_of(data.root(local)) {
                    after.push(set_flag(flag, false, span));
                }
            }
            Effect::StorageDead(local) => {
                if let Some(flag) = flag_of(data.root(*local)) {
                    after.push(set_flag(flag, false, span));
                }
            }
        }
    }
    (before, after)
}

/// The violations of the drops of an elaborated `body` (D15): see
/// [`crate::verify_elaborated`].
pub(crate) fn verify_body(body: &Body, types: &TypeTable) -> Vec<String> {
    let data = MoveData::gather(body, types);
    // The flag of each local with a guarded drop: a switch on a flag whose `otherwise` target
    // drops the local.
    let mut guards: HashMap<LocalId, LocalId> = HashMap::new();
    let mut guarded_drops: HashMap<BlockId, LocalId> = HashMap::new();
    for (_, block_data) in body.blocks.iter() {
        if let TerminatorKind::SwitchInt {
            value: Operand::Copy(flag),
            otherwise,
            ..
        } = &block_data.terminator.kind
        {
            let Some(flag) = flag.as_local() else {
                continue;
            };
            if body.locals[flag].kind != LocalKind::DropFlag {
                continue;
            }
            if let TerminatorKind::Drop { place, .. } = &body.blocks[*otherwise].terminator.kind {
                if let Some(local) = place.as_local() {
                    guards.insert(flag, local);
                    guarded_drops.insert(*otherwise, local);
                }
            }
        }
    }
    let analysis = FlagStates {
        data: &data,
        body,
        guards: &guards,
    };
    let results = solve(body, analysis);
    let mut cursor = results.cursor(body);
    let mut violations = Vec::new();
    for (block, block_data) in body.blocks.iter() {
        let location = Location::terminator(body, block);
        match &block_data.terminator.kind {
            TerminatorKind::Drop { place, .. } => {
                let Some(local) = place.as_local() else {
                    continue;
                };
                let state = cursor.seek(location);
                let guarded = guarded_drops.get(&block) == Some(&local);
                let uninitialized = (0..2).any(|flag| {
                    // A guarded drop runs only where the flag is set.
                    (!guarded || flag == 1) && state.contains(pair(local, flag == 1, false))
                });
                if uninitialized {
                    violations.push(format!(
                        "bb{} drops _{}, which may hold no value there",
                        block.0, local.0
                    ));
                }
            }
            TerminatorKind::SwitchInt {
                value: Operand::Copy(place),
                ..
            } => {
                let Some(local) = place.as_local().and_then(|flag| guards.get(&flag)) else {
                    continue;
                };
                let state = cursor.seek(location);
                if state.contains(pair(*local, true, false))
                    || state.contains(pair(*local, false, true))
                {
                    violations.push(format!(
                        "bb{}: the drop flag {} does not mirror whether _{} holds a value",
                        block.0, place.local.0, local.0
                    ));
                }
            }
            _ => {}
        }
    }
    violations
}

/// The bit of the pair (flag value, holds a value) of `local`.
fn pair(local: LocalId, flag: bool, initialized: bool) -> usize {
    local.index() * 4 + usize::from(flag) * 2 + usize::from(initialized)
}

/// For each local, the pairs (value of its drop flag, whether it holds a value) that may hold at
/// a point; a local without a flag keeps its flag component false.
struct FlagStates<'a> {
    data: &'a MoveData,
    body: &'a Body,
    /// The local each flag guards.
    guards: &'a HashMap<LocalId, LocalId>,
}

impl FlagStates<'_> {
    /// Maps each possible pair of `local` through `update`.
    fn update(
        &self,
        state: &mut BitSet,
        local: LocalId,
        update: impl Fn(bool, bool) -> (bool, bool),
    ) {
        let mut next = Vec::new();
        for flag in [false, true] {
            for initialized in [false, true] {
                if state.remove(pair(local, flag, initialized)) {
                    next.push(update(flag, initialized));
                }
            }
        }
        for (flag, initialized) in next {
            state.insert(pair(local, flag, initialized));
        }
    }

    /// The local whose root path or a path below it is `path`.
    fn local_of(&self, path: crate::move_paths::MovePathId) -> LocalId {
        self.data.paths[path].place.local
    }

    fn apply_location(
        &self,
        state: &mut BitSet,
        location: Location,
        statement: Option<&Statement>,
    ) {
        if let Some(Statement {
            kind: StatementKind::Assign(place, rvalue),
            ..
        }) = statement
        {
            if let (Some(flag), Rvalue::Use(Operand::Constant(Constant::Bool(value)))) =
                (place.as_local(), rvalue.as_ref())
            {
                if let Some(local) = self.guards.get(&flag) {
                    let value = *value;
                    self.update(state, *local, |_, initialized| (value, initialized));
                    return;
                }
            }
        }
        for effect in self.data.effects(location) {
            match effect {
                Effect::Use { .. } => {}
                Effect::Move(out) => {
                    let local = self.local_of(self.data.moves[*out].path);
                    self.update(state, local, |flag, _| (flag, false));
                }
                Effect::Init(init) => {
                    let path = self.data.inits[*init].path;
                    if self.data.paths[path].parent.is_none() {
                        let local = self.local_of(path);
                        self.update(state, local, |flag, _| (flag, true));
                    }
                }
                Effect::StorageLive(start) => {
                    let local = self.data.storage_starts[*start].local;
                    self.update(state, local, |flag, _| (flag, false));
                }
                Effect::StorageDead(local) => {
                    self.update(state, *local, |flag, _| (flag, false));
                }
            }
        }
    }
}

impl Analysis for FlagStates<'_> {
    fn domain_size(&self) -> usize {
        self.body.locals.len() * 4
    }

    fn initialize_boundary(&self, state: &mut BitSet) {
        for (local, declaration) in self.body.locals.iter() {
            let initialized = matches!(declaration.kind, LocalKind::Argument | LocalKind::Capture);
            state.insert(pair(local, false, initialized));
        }
    }

    fn apply_statement(&self, state: &mut BitSet, statement: &Statement, location: Location) {
        self.apply_location(state, location, Some(statement));
    }

    fn apply_terminator(&self, state: &mut BitSet, _: &Terminator, location: Location) {
        self.apply_location(state, location, None);
    }
}
