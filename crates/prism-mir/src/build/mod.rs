//! Lowering a typed tree to MIR (`mir-lowering`).
//!
//! [`lower_file`] lowers each body of a file without type errors with one [`Builder`], which
//! walks the typed tree in evaluation order, destination-passing (D8): a value-producing node is
//! lowered into a destination place, and a place node to a MIR place. The builder keeps the
//! stack of scopes whose locals it drops and marks dead where they end (D6, D7), the open loops
//! and `?.` chains that jumps leave, and the current block, which is `None` after a diverging
//! expression so that nothing after it is lowered (D9).
//!
//! The typed tree's error-free invariants are the lowering's preconditions: a node that cannot
//! occur in a tree without type errors is an internal compiler error (a panic naming the node).

mod call;
mod closure;
mod comparison;
mod control_flow;
mod place;
mod scope;

use crate::mir::{
    BasicBlock, BlockId, Body, BodyId, BodyOwner, LocalDeclaration, LocalId, LocalKind, MirFile,
    Place, Projection, Rvalue, Statement, StatementKind, Terminator, TerminatorKind,
};
use crate::types::mir_type_table;
use prism_ast::Span;
use prism_typed_tree::{
    Arena, ExpressionId, FunctionItem, Item, StatementId, Type, TypeTable, TypedFile, ValueItem,
};
use scope::Scope;
use std::collections::HashMap;

/// Lowers the typed tree of a file without type errors to MIR: one body per function with a
/// body, top-level `val`, parameter default and closure, in item order, each function followed
/// by its default generators in parameter order and each body by the closures lowered inside it
/// (D3).
pub fn lower_file(file: &TypedFile) -> MirFile {
    let types = mir_type_table(&file.types);
    let mut bodies = BodySlots::default();
    for item in &file.items {
        match item {
            Item::Function(function) => lower_function(file, &types, &mut bodies, function),
            Item::Value(value) => lower_value(file, &types, &mut bodies, value),
            Item::Struct(_) | Item::Enum(_) => {}
        }
    }
    MirFile {
        module: file.module.clone(),
        types,
        bodies: bodies.finish(),
    }
}

/// The bodies of the file, reserved in order and filled when lowered, so that a closure's body
/// follows its parent's.
#[derive(Default)]
pub(crate) struct BodySlots {
    slots: Vec<Option<Body>>,
}

impl BodySlots {
    fn reserve(&mut self) -> BodyId {
        self.slots.push(None);
        BodyId((self.slots.len() - 1) as u32)
    }

    fn fill(&mut self, id: BodyId, body: Body) {
        self.slots[id.0 as usize] = Some(body);
    }

    fn finish(self) -> Arena<BodyId, Body> {
        let mut arena = Arena::new();
        for slot in self.slots {
            arena.alloc(slot.expect("every reserved body is lowered"));
        }
        arena
    }
}

fn lower_function(
    file: &TypedFile,
    types: &TypeTable,
    bodies: &mut BodySlots,
    function: &FunctionItem,
) {
    let tree = &file.bodies[function.body];
    if let Some(value) = tree.value {
        let id = bodies.reserve();
        let owner = BodyOwner::Function(function.path.clone());
        let mut builder = Builder::new(file, types, bodies, tree, id, owner, function.span);
        builder.is_compose = function.is_compose;
        builder.set_return_type(function.return_type.clone(), function.span);
        for &parameter in &tree.parameters {
            builder.bind_argument(parameter);
        }
        builder.lower_value(value);
        let body = builder.finish();
        bodies.fill(id, body);
    }
    let receiver_count = usize::from(function.receiver.is_some());
    for (index, default) in tree.defaults.iter().enumerate() {
        if let Some(default) = default {
            let id = bodies.reserve();
            let body = Builder::lower_default(
                file,
                types,
                bodies,
                function,
                id,
                index,
                receiver_count,
                *default,
            );
            bodies.fill(id, body);
        }
    }
}

fn lower_value(file: &TypedFile, types: &TypeTable, bodies: &mut BodySlots, value: &ValueItem) {
    let tree = &file.bodies[value.body];
    let Some(initializer) = tree.value else {
        panic!(
            "internal error: top-level value {} has no initializer",
            value.path
        );
    };
    let id = bodies.reserve();
    let owner = BodyOwner::Value(value.path.clone());
    let mut builder = Builder::new(file, types, bodies, tree, id, owner, value.span);
    builder.set_return_type(value.ty.clone(), value.span);
    builder.lower_value(initializer);
    let body = builder.finish();
    bodies.fill(id, body);
}

/// Merges each block that a `goto` is the only way into with the block jumping to it, then
/// renumbers the remaining blocks in order. The lowering creates such blocks at the joins and
/// returns that only one path reaches.
fn merge_blocks(mut blocks: Vec<BasicBlock>) -> Arena<BlockId, BasicBlock> {
    let mut predecessors = vec![0usize; blocks.len()];
    for block in &blocks {
        for target in block.terminator.kind.successors() {
            predecessors[target.0 as usize] += 1;
        }
    }
    let mut removed = vec![false; blocks.len()];
    for index in 0..blocks.len() {
        if removed[index] {
            continue;
        }
        while let TerminatorKind::Goto(target) = blocks[index].terminator.kind {
            let target = target.0 as usize;
            if target == index || target == 0 || predecessors[target] != 1 || removed[target] {
                break;
            }
            let merged = std::mem::replace(
                &mut blocks[target],
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Terminator {
                        kind: TerminatorKind::Unreachable,
                        span: Span::dummy(),
                    },
                },
            );
            removed[target] = true;
            blocks[index].statements.extend(merged.statements);
            blocks[index].terminator = merged.terminator;
        }
    }
    let mut numbers = vec![None; blocks.len()];
    let mut next = 0u32;
    for (index, number) in numbers.iter_mut().enumerate() {
        if !removed[index] {
            *number = Some(BlockId(next));
            next += 1;
        }
    }
    let renumber = |target: &mut BlockId| {
        *target = numbers[target.0 as usize].expect("a jump targets a kept block");
    };
    let mut arena = Arena::new();
    for (index, mut block) in blocks.into_iter().enumerate() {
        if removed[index] {
            continue;
        }
        match &mut block.terminator.kind {
            TerminatorKind::Goto(target) => renumber(target),
            TerminatorKind::SwitchInt {
                targets, otherwise, ..
            } => {
                for (_, target) in targets.iter_mut() {
                    renumber(target);
                }
                renumber(otherwise);
            }
            TerminatorKind::Call { target, .. } => {
                if let Some(target) = target {
                    renumber(target);
                }
            }
            TerminatorKind::Drop { target, .. } => renumber(target),
            TerminatorKind::Return | TerminatorKind::Unreachable => {}
        }
        arena.alloc(block);
    }
    arena
}

/// A block being built: its statements, and its terminator once it ends.
struct BlockData {
    statements: Vec<Statement>,
    terminator: Option<Terminator>,
}

/// A temporary created before the node that evaluates it, which lowering a `Temporary` node
/// uses instead of a new temporary: the temporary of a `val r = &make()`, which lives as long as
/// the binding (D7), or a default generator's slot (D11).
#[derive(Debug, Clone)]
enum Preallocated {
    /// A temporary of the binding's block, evaluated in place.
    Local(LocalId),
    /// The caller-owned slot `*slot` of type `R?`, into which a value of type `R` is wrapped,
    /// or a value of type `R?` is assigned.
    Slot(Place),
}

/// An open loop, which `break` and `continue` statements name by its statement.
struct LoopContext {
    statement: StatementId,
    /// The number of scopes that stay open at the loop's exit and at its next iteration.
    break_depth: usize,
    continue_depth: usize,
    /// The block after the loop, created by the first jump to it.
    exit: Option<BlockId>,
    /// The block that starts the next iteration: the condition of a `while` (always present),
    /// the latch of a `for` or the condition of a `do`/`while` (created by the first jump).
    next: Option<BlockId>,
    /// For a `do`/`while`, the index of its body's scope, which stays open in the condition.
    body_scope: Option<usize>,
    /// For a `do`/`while`, each `continue` lowered so far: the block it jumped to, how many
    /// locals the body's scope had then, and its span. The body's locals declared after it are marked live
    /// there, once the body is lowered, before the jump to the condition.
    continues: Vec<(BlockId, usize, Span)>,
}

/// An open `?.` chain, which its payload tests leave with `null` (D8).
struct ChainContext {
    /// The index of the chain's scope in the scope stack.
    scope: usize,
    /// Whether the first payload test was made, after which the chain's scope owns the
    /// temporaries the chain creates.
    tested: bool,
    /// The block that gives `null`, created by the first test.
    null_block: Option<BlockId>,
}

/// The builder of one body.
pub(crate) struct Builder<'a> {
    file: &'a TypedFile,
    types: &'a TypeTable,
    bodies: &'a mut BodySlots,
    /// The typed-tree body the MIR body is lowered from (a closure's is its parent's).
    tree: &'a prism_typed_tree::Body,
    id: BodyId,
    owner: BodyOwner,
    is_compose: bool,
    span: Span,
    locals: Arena<LocalId, LocalDeclaration>,
    argument_count: usize,
    capture_count: usize,
    blocks: Vec<BlockData>,
    /// The block being filled, `None` after a diverging expression (D9).
    current: Option<BlockId>,
    /// The MIR place of each typed-tree binding lowered so far (D5).
    bindings: HashMap<prism_typed_tree::LocalId, Place>,
    scopes: Vec<Scope>,
    loops: Vec<LoopContext>,
    chains: Vec<ChainContext>,
    preallocated: HashMap<ExpressionId, Preallocated>,
    /// The shared return block, created by the first `return`.
    return_block: Option<BlockId>,
    /// The number of closures lowered in this body, which names the next one (D3).
    closure_count: usize,
}

impl<'a> Builder<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        file: &'a TypedFile,
        types: &'a TypeTable,
        bodies: &'a mut BodySlots,
        tree: &'a prism_typed_tree::Body,
        id: BodyId,
        owner: BodyOwner,
        span: Span,
    ) -> Self {
        let mut builder = Self {
            file,
            types,
            bodies,
            tree,
            id,
            owner,
            is_compose: false,
            span,
            locals: Arena::new(),
            argument_count: 0,
            capture_count: 0,
            blocks: Vec::new(),
            current: None,
            bindings: HashMap::new(),
            scopes: Vec::new(),
            loops: Vec::new(),
            chains: Vec::new(),
            preallocated: HashMap::new(),
            return_block: None,
            closure_count: 0,
        };
        builder.current = Some(builder.new_block());
        // The arguments' scope: arguments are dropped when the body returns, after its locals,
        // and are never marked.
        builder.scopes.push(Scope::new(span));
        builder
    }

    /// Declares the return place `_0`, which must be the first local.
    fn set_return_type(&mut self, ty: Type, span: Span) {
        assert!(
            self.locals.is_empty(),
            "the return place is the first local"
        );
        self.locals.alloc(LocalDeclaration {
            ty,
            kind: LocalKind::Return,
            mutable: false,
            name: None,
            binding: None,
            span,
        });
    }

    /// Declares the next argument, for the typed-tree binding `binding`.
    fn bind_argument(&mut self, binding: prism_typed_tree::LocalId) {
        let local = &self.tree.locals[binding];
        let id = self.add_argument(
            local.ty.clone(),
            Some(local.name.clone()),
            Some(binding),
            local.span,
        );
        self.locals[id].mutable = local.mutable;
        self.bindings.insert(binding, Place::local(id));
    }

    /// Declares the next argument, owned by the arguments' scope.
    fn add_argument(
        &mut self,
        ty: Type,
        name: Option<String>,
        binding: Option<prism_typed_tree::LocalId>,
        span: Span,
    ) -> LocalId {
        assert_eq!(
            self.locals.len(),
            self.argument_count + 1,
            "arguments follow the return place"
        );
        let id = self.locals.alloc(LocalDeclaration {
            ty,
            kind: LocalKind::Argument,
            mutable: false,
            name,
            binding,
            span,
        });
        self.argument_count += 1;
        self.scopes[0].add_unmarked(id);
        id
    }

    /// Lowers the body's value into the return place, then returns.
    fn lower_value(&mut self, value: prism_typed_tree::ExpressionId) {
        let span = self.tree.expressions[value].span;
        self.push_scope(span);
        self.expression_into(Some(&Place::return_place()), value);
        self.pop_scope();
        if self.current.is_some() {
            match self.return_block {
                Some(block) => self.goto(block, self.span),
                None => self.emit_return(),
            }
        }
        if let Some(block) = self.return_block {
            self.current = Some(block);
            self.emit_return();
        }
    }

    /// Drops the arguments and returns.
    fn emit_return(&mut self) {
        self.exit_scope(0);
        self.terminate(TerminatorKind::Return, self.span);
    }

    fn finish(self) -> Body {
        assert!(
            self.current.is_none(),
            "every block of {:?} ends",
            self.owner
        );
        let mut built = Vec::new();
        for (index, block) in self.blocks.into_iter().enumerate() {
            built.push(BasicBlock {
                statements: block.statements,
                terminator: block
                    .terminator
                    .unwrap_or_else(|| panic!("internal error: bb{} has no terminator", index)),
            });
        }
        let blocks = merge_blocks(built);
        Body {
            owner: self.owner,
            is_compose: self.is_compose,
            locals: self.locals,
            argument_count: self.argument_count,
            capture_count: self.capture_count,
            blocks,
            span: self.span,
        }
    }

    // Blocks.

    fn new_block(&mut self) -> BlockId {
        self.blocks.push(BlockData {
            statements: Vec::new(),
            terminator: None,
        });
        BlockId((self.blocks.len() - 1) as u32)
    }

    /// Appends a statement to the current block, if there is one.
    fn push_statement(&mut self, kind: StatementKind, span: Span) {
        if let Some(block) = self.current {
            self.blocks[block.0 as usize]
                .statements
                .push(Statement { kind, span });
        }
    }

    fn assign(&mut self, place: Place, rvalue: Rvalue, span: Span) {
        if self.current.is_none() {
            return;
        }
        self.record_operands_moved(&rvalue);
        self.push_statement(StatementKind::assign(place, rvalue), span);
    }

    /// Ends the current block with `kind`; there is no current block afterwards.
    fn terminate(&mut self, kind: TerminatorKind, span: Span) {
        if let Some(block) = self.current.take() {
            let data = &mut self.blocks[block.0 as usize];
            assert!(data.terminator.is_none(), "bb{} ends twice", block.0);
            data.terminator = Some(Terminator { kind, span });
        }
    }

    fn goto(&mut self, target: BlockId, span: Span) {
        self.terminate(TerminatorKind::Goto(target), span);
    }

    /// Jumps from the current block, if any, to `target`, created if needed.
    fn goto_lazy(&mut self, target: &mut Option<BlockId>, span: Span) {
        if self.current.is_some() {
            let block = match target {
                Some(block) => *block,
                None => {
                    let block = self.new_block();
                    *target = Some(block);
                    block
                }
            };
            self.goto(block, span);
        }
    }

    /// Joins the blocks `ends` (those of them that fell through) into a new current block;
    /// there is no current block when none fell through.
    fn join(&mut self, ends: &[Option<BlockId>], span: Span) {
        let ends: Vec<BlockId> = ends.iter().flatten().copied().collect();
        if ends.is_empty() {
            self.current = None;
            return;
        }
        let join = self.new_block();
        for end in ends {
            self.current = Some(end);
            self.goto(join, span);
        }
        self.current = Some(join);
    }

    // Locals.

    fn add_local(&mut self, declaration: LocalDeclaration) -> LocalId {
        self.locals.alloc(declaration)
    }

    fn local_type(&self, local: LocalId) -> &Type {
        &self.locals[local].ty
    }

    fn place_type(&self, place: &Place) -> Type {
        let mut ty = self.local_type(place.local).clone();
        for projection in &place.projections {
            ty = crate::types::projection_type(&ty, *projection, self.types).unwrap_or_else(|| {
                panic!(
                    "internal error: projection {:?} out of a {}",
                    projection, ty
                )
            });
        }
        ty
    }

    fn needs_drop(&self, ty: &Type) -> bool {
        crate::types::needs_drop(ty, self.types)
    }

    fn is_copy(&self, ty: &Type) -> bool {
        self.types.is_copy(ty)
    }

    fn expression(&self, id: ExpressionId) -> &'a prism_typed_tree::Expression {
        &self.tree.expressions[id]
    }

    fn deref(place: Place) -> Place {
        place.project(Projection::Deref)
    }
}
