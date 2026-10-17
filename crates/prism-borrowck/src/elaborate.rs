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
///
/// The check solves, on the elaborated body alone, the states (value of the local's drop flag,
/// what the local holds) that each local may be in at each point. A local holds nothing, a
/// value, or `null`, which owns nothing: a nullable becomes `null` on the edge where a test of
/// its tag finds it so. A drop takes the value, and a switch on a drop flag lets only the
/// states whose flag matches the edge through. Then every drop of a whole local and every use
/// must find something on every path, and no local that needs a drop may lose a value it still
/// holds: by a new assignment, the end of its storage, or, for an argument, the return.
pub(crate) fn verify_body(body: &Body, types: &TypeTable) -> Vec<String> {
    let data = MoveData::gather(body, types);
    // The local each flag guards: a switch on a flag whose `otherwise` target drops the local.
    let mut guards: HashMap<LocalId, LocalId> = HashMap::new();
    let mut guarded_drops: HashMap<BlockId, LocalId> = HashMap::new();
    for (_, block_data) in body.blocks.iter() {
        if let Some((flag, _, otherwise)) = flag_switch(body, &block_data.terminator) {
            if let TerminatorKind::Drop { place, .. } = &body.blocks[otherwise].terminator.kind {
                if let Some(local) = place.as_local() {
                    guards.insert(flag, local);
                    guarded_drops.insert(otherwise, local);
                }
            }
        }
    }
    let analysis = DropStates {
        data: &data,
        body,
        types,
        guards: &guards,
    };
    let results = solve(body, analysis);
    let analysis = &results.analysis;
    let needs_drop = |local: LocalId| prism_mir::needs_drop(&body.locals[local].ty, types);
    let mut cursor = results.cursor(body);
    let mut violations = Vec::new();
    for (block, block_data) in body.blocks.iter() {
        for index in 0..=block_data.statements.len() {
            let location = Location {
                block,
                statement: index,
            };
            let mut state = cursor.seek(location).clone();
            let statement = block_data.statements.get(index);
            let mut check = |state: &BitSet, step: Step| {
                let problem = match step {
                    Step::Use(local)
                        if body.locals[local].kind != LocalKind::DropFlag
                            && may_hold(state, local, Content::Nothing) =>
                    {
                        format!("uses _{}, which may hold no value there", local.0)
                    }
                    Step::Overwrite(local)
                        if needs_drop(local) && may_hold(state, local, Content::Value) =>
                    {
                        format!("assigns _{}, which may still hold a value there", local.0)
                    }
                    Step::StorageDead(local)
                        if needs_drop(local) && may_hold(state, local, Content::Value) =>
                    {
                        format!(
                            "ends the storage of _{}, which may still hold a value there",
                            local.0
                        )
                    }
                    _ => return,
                };
                violations.push(format!("bb{} {}", block.0, problem));
            };
            analysis.apply_location(&mut state, location, statement, &mut check);
            if statement.is_some() {
                continue;
            }
            let state = cursor.seek(location);
            match &block_data.terminator.kind {
                TerminatorKind::Drop { place, .. } => {
                    let Some(local) = place.as_local() else {
                        continue;
                    };
                    // A guarded drop's block is entered only with its flag set.
                    if may_hold(state, local, Content::Nothing) {
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
                    let mismatch = state.contains(bit(*local, true, Content::Nothing))
                        || state.contains(bit(*local, false, Content::Value))
                        || state.contains(bit(*local, false, Content::Null));
                    if mismatch {
                        violations.push(format!(
                            "bb{}: the drop flag {} does not mirror whether _{} holds a value",
                            block.0, place.local.0, local.0
                        ));
                    }
                }
                TerminatorKind::Return => {
                    for (local, declaration) in body.locals.iter() {
                        if declaration.kind == LocalKind::Argument
                            && needs_drop(local)
                            && may_hold(state, local, Content::Value)
                        {
                            violations.push(format!(
                                "bb{} returns while the argument _{} may still hold a value",
                                block.0, local.0
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    violations
}

/// The flag, the target for a cleared flag and the `otherwise` target of `terminator`, when it
/// is a switch on a drop flag.
fn flag_switch(body: &Body, terminator: &Terminator) -> Option<(LocalId, BlockId, BlockId)> {
    let (flag, unset, otherwise) = zero_switch(terminator)?;
    (body.locals[flag].kind == LocalKind::DropFlag).then_some((flag, unset, otherwise))
}

/// The local, the target for `0` and the `otherwise` target of `terminator`, when it is a
/// switch of a local between `0` and anything else.
fn zero_switch(terminator: &Terminator) -> Option<(LocalId, BlockId, BlockId)> {
    let TerminatorKind::SwitchInt {
        value: Operand::Copy(value) | Operand::Move(value),
        targets,
        otherwise,
    } = &terminator.kind
    else {
        return None;
    };
    match targets.as_slice() {
        [(0, zero)] if zero != otherwise => Some((value.as_local()?, *zero, *otherwise)),
        _ => None,
    }
}

/// What a local may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Content {
    Nothing,
    Value,
    /// `null`, which owns nothing to drop.
    Null,
}

const CONTENTS: [Content; 3] = [Content::Nothing, Content::Value, Content::Null];

/// The bit of the state (flag value, content) of `local`.
fn bit(local: LocalId, flag: bool, content: Content) -> usize {
    local.index() * 6 + usize::from(flag) * 3 + content as usize
}

/// Whether `local` may hold `content`, whatever its flag.
fn may_hold(state: &BitSet, local: LocalId, content: Content) -> bool {
    [false, true]
        .iter()
        .any(|flag| state.contains(bit(local, *flag, content)))
}

/// One step of a location that the verification checks, before it applies.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// A use of a path of the local.
    Use(LocalId),
    /// A write of a whole new value to the local.
    Overwrite(LocalId),
    StorageDead(LocalId),
}

/// For each local, the states (value of its drop flag, what it holds) that may hold at a point;
/// a local without a flag keeps its flag component false.
struct DropStates<'a> {
    data: &'a MoveData,
    body: &'a Body,
    types: &'a TypeTable,
    /// The local each flag guards.
    guards: &'a HashMap<LocalId, LocalId>,
}

impl DropStates<'_> {
    /// Maps each possible state of `local` through `update`.
    fn update(
        &self,
        state: &mut BitSet,
        local: LocalId,
        update: impl Fn(bool, Content) -> (bool, Content),
    ) {
        let mut next = Vec::new();
        for flag in [false, true] {
            for content in CONTENTS {
                if state.remove(bit(local, flag, content)) {
                    next.push(update(flag, content));
                }
            }
        }
        for (flag, content) in next {
            state.insert(bit(local, flag, content));
        }
    }

    /// The local whose root path or a path below it is `path`.
    fn local_of(&self, path: crate::move_paths::MovePathId) -> LocalId {
        self.data.paths[path].place.local
    }

    /// Applies the effects of `location` (the statement `statement`, or the block's terminator
    /// when it is `None`) in evaluation order, calling `check` with the state before each step
    /// it verifies.
    fn apply_location(
        &self,
        state: &mut BitSet,
        location: Location,
        statement: Option<&Statement>,
        check: &mut impl FnMut(&BitSet, Step),
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
                    self.update(state, *local, |_, content| (value, content));
                    return;
                }
            }
        }
        for effect in self.data.effects(location) {
            match effect {
                Effect::Use { path, .. } => check(state, Step::Use(self.local_of(*path))),
                Effect::Move(out) => {
                    let local = self.local_of(self.data.moves[*out].path);
                    self.update(state, local, |flag, _| (flag, Content::Nothing));
                }
                Effect::Init(init) => {
                    let path = self.data.inits[*init].path;
                    if self.data.paths[path].parent.is_none() {
                        let local = self.local_of(path);
                        check(state, Step::Overwrite(local));
                        self.update(state, local, |flag, _| (flag, Content::Value));
                    }
                }
                Effect::StorageLive(start) => {
                    let local = self.data.storage_starts[*start].local;
                    self.update(state, local, |flag, _| (flag, Content::Nothing));
                }
                Effect::StorageDead(local) => {
                    check(state, Step::StorageDead(*local));
                    self.update(state, *local, |flag, _| (flag, Content::Nothing));
                }
            }
        }
        if statement.is_none() {
            let terminator = &self.body.blocks[location.block].terminator;
            if let TerminatorKind::Drop { place, .. } = &terminator.kind {
                if let Some(local) = place.as_local() {
                    self.update(state, local, |flag, _| (flag, Content::Nothing));
                }
            }
        }
    }

    /// The nullable local whose tag `block` switches on, as `_t = discriminant _l` followed by a
    /// switch of `_t` between `0` and anything else: the local and the target where it is `null`.
    fn tag_switch(&self, block: BlockId) -> Option<(LocalId, BlockId)> {
        let data = &self.body.blocks[block];
        let (tag, null, _) = zero_switch(&data.terminator)?;
        let StatementKind::Assign(place, rvalue) = &data.statements.last()?.kind else {
            return None;
        };
        let Rvalue::Discriminant(tested) = rvalue.as_ref() else {
            return None;
        };
        let local = tested.as_local()?;
        let nullable = matches!(self.body.locals[local].ty, Type::Nullable(_));
        (place.as_local() == Some(tag) && nullable).then_some((local, null))
    }
}

impl Analysis for DropStates<'_> {
    fn domain_size(&self) -> usize {
        self.body.locals.len() * 6
    }

    fn initialize_boundary(&self, state: &mut BitSet) {
        for (local, declaration) in self.body.locals.iter() {
            let initialized = matches!(declaration.kind, LocalKind::Argument | LocalKind::Capture);
            let content = if initialized {
                Content::Value
            } else {
                Content::Nothing
            };
            state.insert(bit(local, false, content));
        }
    }

    fn apply_statement(&self, state: &mut BitSet, statement: &Statement, location: Location) {
        self.apply_location(state, location, Some(statement), &mut |_, _| {});
    }

    fn apply_terminator(&self, state: &mut BitSet, _: &Terminator, location: Location) {
        self.apply_location(state, location, None, &mut |_, _| {});
    }

    /// A switch on a guarding flag lets only the states whose flag matches the edge through,
    /// and a nullable whose tag is `0` holds `null`.
    fn apply_edge(&self, state: &mut BitSet, from: BlockId, to: BlockId) {
        let terminator = &self.body.blocks[from].terminator;
        if let Some((flag, _, otherwise)) = flag_switch(self.body, terminator) {
            if let Some(local) = self.guards.get(&flag) {
                let set = to == otherwise;
                for content in CONTENTS {
                    state.remove(bit(*local, !set, content));
                }
            }
        }
        if let Some((local, null)) = self.tag_switch(from) {
            if to == null && prism_mir::needs_drop(&self.body.locals[local].ty, self.types) {
                self.update(state, local, |flag, content| match content {
                    Content::Value => (flag, Content::Null),
                    other => (flag, other),
                });
            }
        }
    }
}
