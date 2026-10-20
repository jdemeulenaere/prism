//! Composing one body (D3 to D5, D8, D11 to D15 of `compose-transform`).
//!
//! The body's blocks are replayed in order into new blocks. A composable body first gets the
//! composer and its change words as arguments after its declared ones, every later local
//! renumbered. Then:
//!
//! - region markers become group starts and ends with their keys;
//! - composable calls get the composer and change words, each argument's two bits computed from
//!   a block-local map of what each local copies: a constant, or a parameter of the body, whose
//!   own bits are forwarded (D4);
//! - `remember`, `key` and `currentRecomposeScope` become slot reads and writes, movable groups
//!   and the current scope; in any body, the composition operations become composer
//!   operations, the content wrapped in a root content adapter (D12, D13, D15);
//! - every return goes to one end block, which ends the body group.
//!
//! The prologue that starts the body group, and for a skippable body keeps and compares the
//! arguments and decides whether to skip (D8, D11), is built last and becomes `bb0`. The new
//! temporaries are live from `bb0` to the return. One walk of the body, with constant work per
//! statement, argument and group.

use crate::identity::{operand_source, rvalue_source, HiddenCaptures, HiddenKind, VersionSource};
use crate::stability::{has_version_word, is_function_parameter, is_skippable};
use crate::{Appended, BodyKeys};
use prism_ast::Span;
use prism_mir::{
    BasicBlock, BitOperator, BlockId, Body, BodyId, BorrowOrigin, Callee, Capture,
    ComposerOperation, Constant, GroupId, GroupKey, LocalDeclaration, LocalId, LocalKind, MirFile,
    Operand, Place, Projection, Rvalue, Statement, StatementKind, Terminator, TerminatorKind,
};
use prism_typed_tree::ParameterKind;
use prism_typed_tree::{Arena, ArenaId, BuiltinFunction, Type, TypeTable};
use std::collections::HashMap;

/// How many parameters one change word describes (D4).
pub(crate) const PARAMETERS_PER_WORD: usize = 31;

/// The change bits of a parameter.
pub(crate) const UNCERTAIN: i64 = 0;
pub(crate) const SAME: i64 = 1;
pub(crate) const DIFFERENT: i64 = 2;
pub(crate) const STATIC: i64 = 3;
/// The force bit, bit 0 of the first change word.
pub(crate) const FORCE: i64 = 1;
/// Added to the result of `keep` when the kept value changed (D8).
pub(crate) const KEEP_CHANGED: i64 = 4;

/// The number of change words of a composable with `parameters` parameters.
pub(crate) fn word_count(parameters: usize) -> usize {
    parameters.div_ceil(PARAMETERS_PER_WORD).max(1)
}

/// The word and the shift of parameter `index`'s two bits.
pub(crate) fn bit_position(index: usize) -> (usize, i64) {
    (
        index / PARAMETERS_PER_WORD,
        1 + 2 * (index % PARAMETERS_PER_WORD) as i64,
    )
}

/// Whether a call of `builtin` is a composition operation, which any body may call.
pub(crate) fn is_composition_operation(builtin: BuiltinFunction) -> bool {
    matches!(
        builtin,
        BuiltinFunction::Composition
            | BuiltinFunction::SetContent
            | BuiltinFunction::Recompose
            | BuiltinFunction::DumpComposition
            | BuiltinFunction::InvalidateScope
            | BuiltinFunction::RunApp
    )
}

/// The kind and value type of the state object a `&State<T>` or `&MutableState<T>` borrows.
pub(crate) fn state_object(ty: &Type) -> (prism_typed_tree::IntrinsicStruct, Type) {
    let object = match ty {
        Type::Reference { inner, .. } => inner.as_ref(),
        other => other,
    };
    match object {
        Type::Struct { id, arguments } => (
            id.intrinsic()
                .unwrap_or(prism_typed_tree::IntrinsicStruct::State),
            arguments.first().cloned().unwrap_or(Type::Error),
        ),
        _ => (prism_typed_tree::IntrinsicStruct::State, Type::Error),
    }
}

/// The `(Int) -> Unit` type of restart functions and root content adapters.
pub(crate) fn word_function() -> Type {
    Type::Fn {
        is_compose: false,
        receiver: false,
        params: vec![Type::Int],
        kinds: vec![ParameterKind::Plain],
        ret: Box::new(Type::Unit),
    }
}

/// What a local holds a copy of, in the block being replayed (D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// A constant or a value of a top-level `val`.
    Static,
    /// A shared borrow of a top-level `val`, whose referent copied out is static.
    TopLevelReference,
    /// Declared parameter `index` of the body (0-based), the receiver and context included.
    Parameter(usize),
}

/// The composed form of body `id` of `file`.
pub(crate) fn compose_body(
    file: &MirFile,
    id: BodyId,
    keys: &BodyKeys,
    appended: &mut Appended,
    hidden: &HiddenCaptures,
) -> Body {
    let original = &file.bodies[id];
    let mut rewriter = Rewriter::new(file, id, keys, appended, hidden);
    if original.is_compose {
        rewriter.add_composer_arguments();
        rewriter.add_hidden_captures();
        rewriter.plan_identities();
    }
    rewriter.replay();
    rewriter.finish()
}

pub(crate) struct Rewriter<'a> {
    file: &'a MirFile,
    types: &'a TypeTable,
    id: BodyId,
    keys: &'a BodyKeys,
    appended: &'a mut Appended,
    body: Body,
    /// The original blocks, as the borrow checker left them, their locals renumbered.
    original: Vec<BasicBlock>,
    blocks: Vec<BasicBlock>,
    current: BlockId,
    pending: Vec<Statement>,
    /// The locals this rewrite adds, live from the entry to the return.
    temporaries: Vec<LocalId>,
    /// The number of declared parameters, the context and the receiver included.
    declared: usize,
    composer: Option<LocalId>,
    changed: Vec<LocalId>,
    /// Per declared parameter, the local holding what the body knows of it, once needed.
    bits: Vec<Option<LocalId>>,
    skippable: bool,
    /// The block every return of a composable body goes to.
    end: Option<BlockId>,
    /// The block that starts the replayed entry block.
    entry: BlockId,
    span: Span,
    /// What each local of the block being replayed copies.
    sources: HashMap<LocalId, Source>,
    /// The regions open in the block being replayed, innermost last.
    regions: Vec<GroupId>,
    /// The next ordinal of `key` calls, per region.
    key_ordinals: HashMap<Option<GroupId>, u32>,
    /// The next ordinal of `Node` and `LeafNode` calls, per region (D3 of `ui-toolkit`).
    node_ordinals: HashMap<Option<GroupId>, u32>,
    /// The next ordinal of call-site groups, per loop that a `continue` targets (D26).
    call_site_ordinals: HashMap<GroupId, u32>,
    /// The root content adapter of the body, once one is needed.
    adapter: Option<(BodyId, Type)>,
    /// A `Unit` temporary that operations without a result write.
    unit: Option<LocalId>,
    /// The hidden version captures of the file's composable closures (D6 of `strong-skipping`).
    hidden: &'a HiddenCaptures,
    /// Per declared parameter, its version argument, for any borrow (D4 and D21 of
    /// `strong-skipping`). A borrowed state object's is not its identity: see
    /// [`Rewriter::identity_version`].
    version_arguments: Vec<Option<LocalId>>,
    /// For a composable closure, per hidden capture: the capture local it versions, its own
    /// local and its kind (D6 of `strong-skipping`).
    hidden_locals: Vec<(LocalId, LocalId, HiddenKind)>,
    /// The version local of each reference local whose value reaches a borrowed argument or a
    /// hidden capture (D5 of `strong-skipping`).
    version_locals: HashMap<LocalId, LocalId>,
    /// The reference locals whose version is fixed for the whole body (D5 of
    /// `strong-skipping`).
    fixed_versions: HashMap<LocalId, crate::identity::Fixed>,
}

impl<'a> Rewriter<'a> {
    fn new(
        file: &'a MirFile,
        id: BodyId,
        keys: &'a BodyKeys,
        appended: &'a mut Appended,
        hidden: &'a HiddenCaptures,
    ) -> Self {
        let body = file.bodies[id].clone();
        let declared = body.argument_count;
        let skippable = is_skippable(&body, &file.types);
        Rewriter {
            file,
            types: &file.types,
            id,
            keys,
            appended,
            span: body.span,
            original: Vec::new(),
            blocks: Vec::new(),
            current: BlockId(0),
            pending: Vec::new(),
            temporaries: Vec::new(),
            declared,
            composer: None,
            changed: Vec::new(),
            bits: vec![None; declared],
            skippable,
            end: None,
            entry: BlockId(0),
            body,
            sources: HashMap::new(),
            regions: Vec::new(),
            key_ordinals: HashMap::new(),
            node_ordinals: HashMap::new(),
            call_site_ordinals: HashMap::new(),
            adapter: None,
            unit: None,
            hidden,
            version_arguments: vec![None; declared],
            hidden_locals: Vec::new(),
            version_locals: HashMap::new(),
            fixed_versions: HashMap::new(),
        }
    }

    /// Inserts `$composer`, the change words and the version words after the declared
    /// arguments, renumbering every later local (D19; D4 of `strong-skipping`).
    fn add_composer_arguments(&mut self) {
        let words = word_count(self.declared);
        let versioned: Vec<usize> = (0..self.declared)
            .filter(|index| has_version_word(&self.declared_type(*index)))
            .collect();
        let added = 1 + words + versioned.len();
        let first = self.declared + 1;
        renumber_locals(&mut self.body, first, added);
        let old = std::mem::take(&mut self.body.locals).into_vec();
        let mut locals = Arena::new();
        let mut old = old.into_iter();
        for _ in 0..first {
            locals.alloc(old.next().expect("the return place and the arguments"));
        }
        let argument = |name: String| LocalDeclaration {
            ty: Type::Int,
            kind: LocalKind::Argument,
            mutable: false,
            name: Some(name),
            binding: None,
            parameter_kind: ParameterKind::Plain,
            span: self.span,
        };
        self.composer = Some(locals.alloc(argument("$composer".to_string())));
        for word in 0..words {
            let name = if word == 0 {
                "$changed".to_string()
            } else {
                format!("$changed{}", word)
            };
            self.changed.push(locals.alloc(argument(name)));
        }
        for (ordinal, index) in versioned.into_iter().enumerate() {
            self.version_arguments[index] =
                Some(locals.alloc(argument(format!("$version{}", ordinal))));
        }
        for local in old {
            locals.alloc(local);
        }
        self.body.locals = locals;
        self.body.argument_count += added;
    }

    /// For a composable closure, appends one `Int` capture local per hidden version capture
    /// after its captures, renumbering every later local (D6 of `strong-skipping`).
    fn add_hidden_captures(&mut self) {
        let hidden = self.hidden.of_body(self.id);
        if hidden.is_empty() {
            return;
        }
        let first_capture = self.body.argument_count + 1;
        let first = first_capture + self.body.capture_count;
        renumber_locals(&mut self.body, first, hidden.len());
        let old = std::mem::take(&mut self.body.locals).into_vec();
        let mut locals = Arena::new();
        let mut old = old.into_iter();
        for _ in 0..first {
            locals.alloc(old.next().expect("the arguments and the captures"));
        }
        for capture in hidden {
            let captured = LocalId((first_capture + capture.capture) as u32);
            let name = locals[captured].name.clone().unwrap_or_default();
            let local = locals.alloc(LocalDeclaration {
                ty: Type::Int,
                kind: LocalKind::Capture,
                mutable: false,
                name: Some(format!("$version({})", name)),
                binding: None,
                parameter_kind: ParameterKind::Plain,
                span: self.span,
            });
            self.hidden_locals.push((captured, local, capture.kind));
        }
        for local in old {
            locals.alloc(local);
        }
        self.body.locals = locals;
        let widened = self.body.capture_widenings.len() == self.body.capture_count;
        self.body.capture_count += hidden.len();
        let mode = if crate::identity::HiddenCaptures::owns_captures(self.file, self.id) {
            prism_mir::CaptureMode::Owned
        } else {
            prism_mir::CaptureMode::Shared
        };
        for _ in hidden {
            self.body.capture_modes.push(mode);
            if widened {
                self.body.capture_widenings.push(None);
            }
        }
    }

    /// Finds the versions of the reference locals that reach borrowed arguments and hidden
    /// captures, and gives a version local to each one whose version is not fixed (D5 of
    /// `strong-skipping`).
    fn plan_identities(&mut self) {
        let own: Vec<crate::identity::HiddenCapture> = self.hidden.of_body(self.id).to_vec();
        let analysis =
            crate::identity::analyze(self.file, &self.body, self.hidden, &own, |local| {
                self.given_version(local)
            });
        self.fixed_versions = analysis.fixed;
        let mut needed: Vec<LocalId> = analysis.needed.into_iter().collect();
        needed.sort();
        for local in needed {
            let version = self.temporary(Type::Int);
            self.version_locals.insert(local, version);
        }
    }

    /// The version argument of declared parameter `index` when it is the parameter's identity:
    /// a shared borrow other than of a state object. A borrowed state object is kept by its
    /// cell (D13 of `owned-state`), and a mutable borrow has no identity, so the version word
    /// either is passed is not used, as when it took none (D21 of `strong-skipping`): the body
    /// never takes the replaceable path for it, and forwards it as no identity.
    fn identity_version(&self, index: usize) -> Option<LocalId> {
        let version = self.version_arguments[index]?;
        crate::stability::has_identity(&self.declared_type(index), self.types).then_some(version)
    }

    /// The version the body was given for the argument or capture local `local`: its version
    /// argument or its hidden capture.
    fn given_version(&self, local: LocalId) -> Option<crate::identity::Fixed> {
        let index = local.index();
        if index >= 1 && index <= self.declared {
            return self
                .identity_version(index - 1)
                .map(crate::identity::Fixed::Given);
        }
        self.hidden_locals
            .iter()
            .find(|(captured, _, _)| *captured == local)
            .map(|(_, hidden, _)| crate::identity::Fixed::Given(*hidden))
    }

    /// Whether `local` is a capture local with a hidden version of the place it stands for.
    fn is_hidden_place(&self, local: LocalId) -> bool {
        self.hidden_locals
            .iter()
            .any(|(captured, _, kind)| *captured == local && *kind == HiddenKind::Place)
    }

    /// The version a borrowed argument or hidden capture derived from `source` has (D2 of
    /// `strong-skipping`): a constant, a version argument, a hidden capture or a version local.
    fn version_operand(&self, source: VersionSource) -> Operand {
        let fixed = |fixed: crate::identity::Fixed| match fixed {
            crate::identity::Fixed::Constant(value) => Self::int(value),
            crate::identity::Fixed::Given(local) => self.copy(local),
        };
        match source {
            VersionSource::None => Self::int(0),
            VersionSource::Permanent => Self::int(1),
            VersionSource::CapturedPlace(captured) => {
                if !self.is_hidden_place(captured) {
                    return Self::int(0);
                }
                self.given_version(captured).map_or(Self::int(0), fixed)
            }
            VersionSource::Reference(local) => {
                if let Some(version) = self.fixed_versions.get(&local) {
                    return fixed(*version);
                }
                if let Some(version) = self.version_locals.get(&local) {
                    return self.copy(*version);
                }
                self.given_version(local).map_or(Self::int(0), fixed)
            }
        }
    }

    /// The source of the version of a reference assigned `rvalue` in this body.
    fn assigned_source(&self, rvalue: &Rvalue) -> VersionSource {
        rvalue_source(&self.body, self.types, rvalue, |local| {
            self.is_hidden_place(local)
        })
    }

    /// The hidden version captures to append to a composable closure value built from
    /// `captures`, in the order of its hidden captures (D6 of `strong-skipping`).
    ///
    /// A closure that owns its captures gets copies of the versions. A borrowing closure, whose
    /// environment lives in the creating frame, borrows the locals holding them (a constant is
    /// put in a temporary first), as it borrows the references they describe, so a value built
    /// in an earlier iteration of a loop reads the reference and its version as they are now.
    fn hidden_capture_values(
        &mut self,
        closure: BodyId,
        captures: &[Capture],
        span: Span,
    ) -> Vec<Capture> {
        let owning = crate::identity::HiddenCaptures::owns_captures(self.file, closure);
        let hidden: Vec<_> = self.hidden.of_body(closure).to_vec();
        hidden
            .into_iter()
            .map(|hidden| {
                let source = crate::identity::hidden_source(
                    &self.body,
                    self.types,
                    &captures[hidden.capture],
                    hidden.kind,
                    |local| self.is_hidden_place(local),
                );
                let version = self.version_operand(source);
                if owning {
                    return Capture::Owned(version);
                }
                match version {
                    Operand::Copy(place, _) | Operand::Move(place, _) => Capture::Borrowed(place),
                    constant => {
                        let local = self.temporary(Type::Int);
                        self.assign(local, Rvalue::Use(constant), span);
                        Capture::Borrowed(Place::local(local))
                    }
                }
            })
            .collect()
    }

    /// When `destination` is a reference local with a version local, a block assigning it
    /// `value` before going to `target`, for a call whose result has that version; else
    /// `target`.
    fn version_after_call(&mut self, destination: &Place, target: BlockId, value: i64) -> BlockId {
        let Some(version) = destination
            .as_local()
            .and_then(|local| self.version_locals.get(&local).copied())
        else {
            return target;
        };
        let block = self.new_block();
        let span = self.span;
        self.blocks[block.index()].statements.push(Statement {
            kind: StatementKind::assign(Place::local(version), Rvalue::Use(Self::int(value))),
            span,
        });
        self.blocks[block.index()].terminator = Terminator {
            kind: TerminatorKind::Goto(target),
            span,
        };
        block
    }

    // Blocks.

    fn new_block(&mut self) -> BlockId {
        self.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::Unreachable,
                span: self.span,
            },
        });
        BlockId((self.blocks.len() - 1) as u32)
    }

    /// Ends the current block with `kind`; the caller sets the next current block.
    fn terminate(&mut self, kind: TerminatorKind, span: Span) {
        let block = &mut self.blocks[self.current.index()];
        block.statements = std::mem::take(&mut self.pending);
        block.terminator = Terminator { kind, span };
    }

    fn push(&mut self, kind: StatementKind, span: Span) {
        self.pending.push(Statement { kind, span });
    }

    fn assign(&mut self, local: LocalId, rvalue: Rvalue, span: Span) {
        self.push(StatementKind::assign(Place::local(local), rvalue), span);
    }

    fn goto(&mut self, target: BlockId, span: Span) {
        self.terminate(TerminatorKind::Goto(target), span);
    }

    /// Ends the current block with a switch on `value` and continues nowhere.
    fn switch(&mut self, value: Operand, targets: Vec<(u64, BlockId)>, otherwise: BlockId) {
        let span = self.span;
        self.terminate(
            TerminatorKind::SwitchInt {
                value,
                targets,
                otherwise,
            },
            span,
        );
    }

    /// Ends the current block with a call writing `destination`, and continues in a new block.
    fn call(
        &mut self,
        callee: Callee,
        arguments: Vec<Operand>,
        parameters: Vec<Type>,
        destination: Place,
        span: Span,
    ) {
        self.call_with(callee, arguments, parameters, Vec::new(), destination, span)
    }

    /// [`Self::call`] with type arguments.
    fn call_with(
        &mut self,
        callee: Callee,
        arguments: Vec<Operand>,
        parameters: Vec<Type>,
        type_arguments: Vec<Type>,
        destination: Place,
        span: Span,
    ) {
        let next = self.new_block();
        let parameter_kinds = vec![ParameterKind::Plain; parameters.len()];
        self.terminate(
            TerminatorKind::Call {
                callee,
                arguments,
                parameters,
                parameter_kinds,
                type_arguments,
                destination,
                target: Some(next),
                is_compose: false,
            },
            span,
        );
        self.current = next;
    }

    /// A call of a composer operation.
    fn operation(
        &mut self,
        operation: ComposerOperation,
        arguments: Vec<Operand>,
        parameters: Vec<Type>,
        destination: Place,
        span: Span,
    ) {
        self.call(
            Callee::Composer(operation),
            arguments,
            parameters,
            destination,
            span,
        );
    }

    // Locals.

    fn temporary(&mut self, ty: Type) -> LocalId {
        let local = self.body.locals.alloc(LocalDeclaration {
            ty,
            kind: LocalKind::Temporary,
            mutable: false,
            name: None,
            binding: None,
            parameter_kind: ParameterKind::Plain,
            span: self.span,
        });
        self.temporaries.push(local);
        local
    }

    fn unit(&mut self) -> Place {
        let local = match self.unit {
            Some(local) => local,
            None => {
                let local = self.temporary(Type::Unit);
                self.unit = Some(local);
                local
            }
        };
        Place::local(local)
    }

    fn composer(&self) -> Operand {
        Operand::Copy(
            Place::local(self.composer.expect("a composable body has a composer")),
            self.span,
        )
    }

    fn copy(&self, local: LocalId) -> Operand {
        Operand::Copy(Place::local(local), self.span)
    }

    fn int(value: i64) -> Operand {
        Operand::Constant(Constant::Int(value))
    }

    fn declared_type(&self, index: usize) -> Type {
        self.body.locals[LocalId((index + 1) as u32)].ty.clone()
    }

    /// The local holding what the body knows of declared parameter `index` (D4): the bits it
    /// was passed, or in a skippable body same or different after its comparison.
    fn parameter_bits(&mut self, index: usize) -> LocalId {
        if let Some(local) = self.bits[index] {
            return local;
        }
        let local = self.temporary(Type::Int);
        self.bits[index] = Some(local);
        local
    }

    // Replaying the body.

    fn replay(&mut self) {
        let original = &self.file.bodies[self.id];
        // The regions open at each block's entry, whose nesting the validator checked.
        let open = prism_mir::open_regions(original).unwrap_or_else(|message| {
            panic!(
                "internal error: the regions of {} are not nested: {}",
                self.file.body_name(self.id),
                message
            )
        });
        self.original = std::mem::take(&mut self.body.blocks).into_vec();
        for _ in 0..self.original.len() {
            self.new_block();
        }
        if self.body.is_compose {
            self.end = Some(self.new_block());
        }
        for (index, open) in open.into_iter().enumerate() {
            self.current = BlockId(index as u32);
            self.sources.clear();
            self.regions = open.unwrap_or_default();
            let block = std::mem::replace(
                &mut self.original[index],
                BasicBlock {
                    statements: Vec::new(),
                    terminator: Terminator {
                        kind: TerminatorKind::Unreachable,
                        span: self.span,
                    },
                },
            );
            for statement in block.statements {
                self.statement(statement);
            }
            self.terminator(block.terminator);
        }
    }

    fn statement(&mut self, statement: Statement) {
        let span = statement.span;
        match statement.kind {
            StatementKind::GroupStart(group) => {
                let key = GroupKey {
                    hash: self.keys.region_or_body(self.id, Some(group)),
                    label: self.file.bodies[self.id].groups[group].step.describe(),
                };
                let composer = self.composer();
                let unit = self.unit();
                self.operation(
                    ComposerOperation::StartGroup { key },
                    vec![composer],
                    vec![Type::Int],
                    unit,
                    span,
                );
                self.regions.push(group);
            }
            StatementKind::GroupEnd(_) => {
                let composer = self.composer();
                let unit = self.unit();
                self.operation(
                    ComposerOperation::EndGroup,
                    vec![composer],
                    vec![Type::Int],
                    unit,
                    span,
                );
                self.regions.pop();
            }
            StatementKind::Assign(place, mut rvalue) => {
                self.note_assignment(&place, &rvalue);
                // A composable closure also owns the versions of the references it borrows
                // from (D6 of `strong-skipping`).
                if let Rvalue::Closure { body, captures } = rvalue.as_mut() {
                    let hidden = self.hidden_capture_values(*body, captures, span);
                    captures.extend(hidden);
                }
                let version = place
                    .as_local()
                    .and_then(|local| self.version_locals.get(&local).copied())
                    .map(|version| (version, self.version_operand(self.assigned_source(&rvalue))));
                self.pending.push(Statement {
                    kind: StatementKind::Assign(place, rvalue),
                    span,
                });
                // A reference's version is assigned beside it, so they agree on every path.
                if let Some((version, operand)) = version {
                    self.assign(version, Rvalue::Use(operand), span);
                }
            }
            StatementKind::Destructure {
                source,
                variant,
                parts,
            } => {
                for (_, target) in &parts {
                    self.sources.remove(target);
                }
                let versions: Vec<LocalId> = parts
                    .iter()
                    .filter_map(|(_, target)| self.version_locals.get(target).copied())
                    .collect();
                self.pending.push(Statement {
                    kind: StatementKind::Destructure {
                        source,
                        variant,
                        parts,
                    },
                    span,
                });
                for version in versions {
                    self.assign(version, Rvalue::Use(Self::int(0)), span);
                }
            }
            kind => self.pending.push(Statement { kind, span }),
        }
    }

    /// Records what the assigned local copies, for the change bits of a later call (D4).
    fn note_assignment(&mut self, place: &Place, rvalue: &Rvalue) {
        // A local borrowed mutably, by a borrow or a closure, may be written through that
        // borrow later in the block, so what it copied is forgotten.
        match rvalue {
            Rvalue::Borrow {
                mutable: true,
                place: borrowed,
                ..
            } => {
                self.sources.remove(&borrowed.local);
            }
            Rvalue::Closure { captures, .. } => {
                for capture in captures {
                    if let Capture::Borrowed(borrowed) = capture {
                        self.sources.remove(&borrowed.local);
                    }
                }
            }
            _ => {}
        }
        let Some(local) = place.as_local() else {
            self.sources.remove(&place.local);
            return;
        };
        let source = match rvalue {
            Rvalue::Use(Operand::Constant(Constant::TopLevelValue(_))) => {
                Some(Source::TopLevelReference)
            }
            Rvalue::Use(Operand::Constant(_)) => Some(Source::Static),
            Rvalue::Use(Operand::Copy(source, _) | Operand::Move(source, _)) => {
                self.place_source(source)
            }
            // A borrow of a top-level value, or of a part of it named by fields alone, is
            // static, and a reborrow of a whole borrowed parameter forwards its bits (D10 of
            // `strong-skipping`). An element chosen by an index is another place whenever the
            // index changes, so it is uncertain.
            Rvalue::Borrow {
                mutable: false,
                place: borrowed,
                ..
            } if borrowed.projections.first() == Some(&Projection::Deref) => {
                if self.sources.get(&borrowed.local) == Some(&Source::TopLevelReference)
                    && !borrowed
                        .projections
                        .iter()
                        .any(|projection| matches!(projection, Projection::Index(_)))
                {
                    Some(Source::Static)
                } else if borrowed.projections.len() == 1
                    && borrowed.local.index() >= 1
                    && borrowed.local.index() <= self.declared
                {
                    Some(Source::Parameter(borrowed.local.index() - 1))
                } else {
                    None
                }
            }
            _ => None,
        };
        match source {
            Some(source) => {
                self.sources.insert(local, source);
            }
            None => {
                self.sources.remove(&local);
            }
        }
    }

    /// What reading `place` gives a copy of, when the transform knows it.
    fn place_source(&self, place: &Place) -> Option<Source> {
        match place.projections.as_slice() {
            [] if place.local.index() >= 1 && place.local.index() <= self.declared => {
                Some(Source::Parameter(place.local.index() - 1))
            }
            [] => self
                .sources
                .get(&place.local)
                .copied()
                .filter(|source| *source != Source::TopLevelReference),
            [Projection::Deref] => (self.sources.get(&place.local)
                == Some(&Source::TopLevelReference))
            .then_some(Source::Static),
            _ => None,
        }
    }

    fn terminator(&mut self, terminator: Terminator) {
        let span = terminator.span;
        match terminator.kind {
            TerminatorKind::Return => match self.end {
                Some(end) => self.goto(end, span),
                None => self.terminate(TerminatorKind::Return, span),
            },
            TerminatorKind::Call {
                callee: Callee::Builtin(builtin),
                arguments,
                parameters,
                parameter_kinds,
                type_arguments,
                destination,
                target,
                is_compose,
            } if builtin.is_compose_runtime() => {
                // The value a `remember` lends is permanent (D2 of `strong-skipping`).
                let target = match target {
                    Some(target) if builtin == BuiltinFunction::Remember => {
                        Some(self.version_after_call(&destination, target, 1))
                    }
                    target => target,
                };
                let call = IntrinsicCall {
                    builtin,
                    arguments,
                    parameters,
                    type_arguments,
                    destination,
                    target,
                    span,
                };
                if builtin == BuiltinFunction::Remember && self.body.is_compose {
                    let target = call.target.expect("a compose intrinsic returns");
                    if self.start_call_site_group(span) {
                        let after = self.new_block();
                        self.intrinsic(IntrinsicCall {
                            target: Some(after),
                            ..call
                        });
                        self.current = after;
                        self.end_call_site_group(target, span);
                    } else {
                        self.intrinsic(call);
                    }
                } else if is_composition_operation(builtin) || self.body.is_compose {
                    self.intrinsic(call);
                } else {
                    // A composable intrinsic outside a composable body never type-checks.
                    let IntrinsicCall {
                        arguments,
                        parameters,
                        type_arguments,
                        destination,
                        target,
                        ..
                    } = call;
                    self.terminate(
                        TerminatorKind::Call {
                            callee: Callee::Builtin(builtin),
                            arguments,
                            parameters,
                            parameter_kinds,
                            type_arguments,
                            destination,
                            target,
                            is_compose,
                        },
                        span,
                    );
                }
            }
            TerminatorKind::Call {
                callee,
                mut arguments,
                mut parameters,
                mut parameter_kinds,
                type_arguments,
                destination,
                target,
                is_compose: true,
            } if self.body.is_compose => {
                let wrapped = self.start_call_site_group(span);
                let words = self.change_words(&arguments, span);
                // One version per borrowed parameter, after the change words (D4 of
                // `strong-skipping`).
                let versions: Vec<Operand> = arguments
                    .iter()
                    .zip(parameters.iter())
                    .filter(|(_, parameter)| has_version_word(parameter))
                    .map(|(argument, _)| self.version_operand(operand_source(argument)))
                    .collect();
                arguments.push(self.composer());
                parameters.push(Type::Int);
                parameter_kinds.push(ParameterKind::Plain);
                for word in words.into_iter().chain(versions) {
                    arguments.push(word);
                    parameters.push(Type::Int);
                    parameter_kinds.push(ParameterKind::Plain);
                }
                let target = target.map(|target| self.version_after_call(&destination, target, 0));
                if let Some(local) = destination.as_local() {
                    self.sources.remove(&local);
                }
                // A call-site group ends when the call returns; a call that never returns
                // leaves it open, as it leaves its caller's groups.
                let (call_target, after) = match target {
                    Some(target) if wrapped => {
                        let after = self.new_block();
                        (Some(after), Some((after, target)))
                    }
                    target => (target, None),
                };
                self.terminate(
                    TerminatorKind::Call {
                        callee,
                        arguments,
                        parameters,
                        parameter_kinds,
                        type_arguments,
                        destination,
                        target: call_target,
                        is_compose: true,
                    },
                    span,
                );
                if let Some((after, target)) = after {
                    self.current = after;
                    self.end_call_site_group(target, span);
                }
            }
            mut kind => {
                if let TerminatorKind::Call {
                    callee,
                    arguments,
                    parameters,
                    type_arguments,
                    destination,
                    target,
                    ..
                } = &mut kind
                {
                    if let Some(local) = destination.as_local() {
                        self.sources.remove(&local);
                    }
                    let version = destination
                        .as_local()
                        .and_then(|local| self.version_locals.get(&local).copied());
                    match (callee, version) {
                        // A borrow of a state's contents has the state's version, read on
                        // the same state object right before the read (D2 of
                        // `strong-skipping`).
                        (Callee::State(prism_mir::StateOperation::Read), Some(version)) => {
                            let state = match &arguments[0] {
                                Operand::Move(place, span) | Operand::Copy(place, span) => {
                                    Operand::Copy(place.clone(), *span)
                                }
                                constant => constant.clone(),
                            };
                            let kind = prism_mir::StateOperation::kind_of(parameters);
                            self.call_with(
                                Callee::State(prism_mir::StateOperation::Version),
                                vec![state],
                                prism_mir::StateOperation::Version.parameters(kind),
                                type_arguments.clone(),
                                Place::local(version),
                                span,
                            );
                        }
                        (_, Some(_)) => {
                            if let Some(next) = *target {
                                let destination = destination.clone();
                                *target = Some(self.version_after_call(&destination, next, 0));
                            }
                        }
                        _ => {}
                    }
                }
                self.terminate(kind, span)
            }
        }
    }

    /// Starts the call-site group of a composable call or `remember` made directly in a loop
    /// that a `continue` targets, and returns whether it did (D26). A `continue` can skip a
    /// call in one iteration and not in another, so without it calls of one composable at two
    /// call sites, or two `remember`s, could match each other's groups and slots, and trust
    /// change bits meant for another call. The key is the call's ordinal among the loop's
    /// direct calls in block order.
    fn start_call_site_group(&mut self, span: Span) -> bool {
        let Some(&region) = self.regions.last() else {
            return false;
        };
        if !self.file.bodies[self.id].groups[region].continued {
            return false;
        }
        let ordinal = self.call_site_ordinals.entry(region).or_insert(0);
        let key = GroupKey {
            hash: crate::keys::call_site_key(
                self.keys.region_or_body(self.id, Some(region)),
                *ordinal,
            ),
            label: format!("call {}", ordinal),
        };
        *ordinal += 1;
        let composer = self.composer();
        let unit = self.unit();
        self.operation(
            ComposerOperation::StartGroup { key },
            vec![composer],
            vec![Type::Int],
            unit,
            span,
        );
        true
    }

    /// Ends a call-site group in the current block, then continues at `target`.
    fn end_call_site_group(&mut self, target: BlockId, span: Span) {
        let composer = self.composer();
        let unit = self.unit();
        self.operation(
            ComposerOperation::EndGroup,
            vec![composer],
            vec![Type::Int],
            unit,
            span,
        );
        self.goto(target, span);
    }

    /// The change words of a composable call with `arguments`, in the callee's parameter order
    /// (D4): static for a constant or a value of a top-level `val`, the caller's own bits for a
    /// copy of its parameter, uncertain otherwise. A word with no forwarded bits is a constant.
    fn change_words(&mut self, arguments: &[Operand], span: Span) -> Vec<Operand> {
        let words = word_count(arguments.len());
        let mut constants = vec![0i64; words];
        let mut forwarded: Vec<Vec<(LocalId, i64)>> = vec![Vec::new(); words];
        for (index, argument) in arguments.iter().enumerate() {
            let (word, shift) = bit_position(index);
            let source = match argument {
                Operand::Constant(_) => Some(Source::Static),
                Operand::Copy(place, _) | Operand::Move(place, _) => self.place_source(place),
            };
            match source {
                Some(Source::Static) => constants[word] |= STATIC << shift,
                Some(Source::Parameter(parameter)) => {
                    let bits = self.parameter_bits(parameter);
                    forwarded[word].push((bits, shift));
                }
                Some(Source::TopLevelReference) | None => {}
            }
        }
        let mut operands = Vec::with_capacity(words);
        for (constant, forwarded) in constants.into_iter().zip(forwarded) {
            if forwarded.is_empty() {
                operands.push(Self::int(constant));
                continue;
            }
            let word = self.temporary(Type::Int);
            self.assign(word, Rvalue::Use(Self::int(constant)), span);
            for (bits, shift) in forwarded {
                let part = self.temporary(Type::Int);
                self.assign(
                    part,
                    Rvalue::Bits(BitOperator::ShiftLeft, self.copy(bits), Self::int(shift)),
                    span,
                );
                self.assign(
                    word,
                    Rvalue::Bits(BitOperator::Or, self.copy(word), self.copy(part)),
                    span,
                );
            }
            operands.push(self.copy(word));
        }
        operands
    }

    // The intrinsics of `/prism/compose/runtime` (D12, D13, D15).

    fn intrinsic(&mut self, call: IntrinsicCall) {
        let IntrinsicCall {
            builtin,
            arguments,
            parameters,
            type_arguments,
            destination,
            target,
            span,
        } = call;
        let target = target.expect("a compose intrinsic returns");
        match builtin {
            // The body's own composer (D3 of `compose-library-in-prism`).
            BuiltinFunction::CurrentComposer => {
                let composer = self.composer();
                self.push(
                    StatementKind::assign(destination, Rvalue::Use(composer)),
                    span,
                );
                self.goto(target, span);
            }
            BuiltinFunction::Remember => self.remember(
                arguments,
                parameters,
                type_arguments,
                destination,
                target,
                span,
            ),
            BuiltinFunction::Key => self.key(arguments, target, span),
            // A side effect is moved to the composition's queue (D13 of `state-handles`).
            BuiltinFunction::SideEffect => {
                let composer = self.composer();
                let mut arguments = arguments;
                arguments.insert(0, composer);
                let mut parameters = parameters;
                parameters.insert(0, Type::Int);
                self.operation(
                    ComposerOperation::SideEffect,
                    arguments,
                    parameters,
                    destination,
                    span,
                );
                self.goto(target, span);
            }
            BuiltinFunction::CurrentRecomposeScope => {
                let composer = self.composer();
                self.operation(
                    ComposerOperation::CurrentScope,
                    vec![composer],
                    vec![Type::Int],
                    destination,
                    span,
                );
                self.goto(target, span);
            }
            BuiltinFunction::Composition | BuiltinFunction::SetContent => {
                let content_index = arguments.len() - 1;
                let content_type = parameters[content_index].clone();
                let mut arguments = arguments;
                let content = arguments.pop().expect("the content");
                let adapter = self.adapter_value(content, content_type, span);
                arguments.push(Operand::Move(Place::local(adapter), span));
                let mut parameters = parameters;
                parameters[content_index] = word_function();
                let operation = if builtin == BuiltinFunction::Composition {
                    ComposerOperation::StartComposition
                } else {
                    ComposerOperation::SetContent
                };
                self.operation(operation, arguments, parameters, destination, span);
                self.goto(target, span);
            }
            // `runApp` keeps its call and its composable content, which the UI runtime calls
            // with the composer and the force bit (D4 of `compose-library-in-prism`).
            BuiltinFunction::RunApp => {
                self.call(
                    Callee::Builtin(BuiltinFunction::RunApp),
                    arguments,
                    parameters,
                    destination,
                    span,
                );
                self.goto(target, span);
            }
            BuiltinFunction::UiNode | BuiltinFunction::UiLeafNode => {
                self.node(builtin, arguments, parameters, target, span)
            }
            BuiltinFunction::Recompose
            | BuiltinFunction::DumpComposition
            | BuiltinFunction::InvalidateScope => {
                let operation = match builtin {
                    BuiltinFunction::Recompose => ComposerOperation::Recompose,
                    BuiltinFunction::DumpComposition => ComposerOperation::Dump,
                    _ => ComposerOperation::Invalidate,
                };
                self.operation(operation, arguments, parameters, destination, span);
                self.goto(target, span);
            }
            other => panic!("internal error: {} is no compose intrinsic", other.name()),
        }
    }

    /// `remember(keys..., calculation)` (D12): each key compared with its slot, which always
    /// advances the cursor; when none changed, the slot of the value read; when it is missing
    /// or a key changed, the calculation run and its result stored, forgetting the old value.
    fn remember(
        &mut self,
        mut arguments: Vec<Operand>,
        parameters: Vec<Type>,
        type_arguments: Vec<Type>,
        destination: Place,
        target: BlockId,
        span: Span,
    ) {
        let value_type = type_arguments[0].clone();
        let calculation = arguments.pop().expect("a calculation");
        let calculation = self.held_in_local(calculation, parameters.last().unwrap(), span);
        let changed = self.temporary(Type::Bool);
        self.assign(
            changed,
            Rvalue::Use(Operand::Constant(Constant::Bool(false))),
            span,
        );
        for (index, key) in arguments.into_iter().enumerate() {
            let key_type = parameters[index].substitute(&type_arguments);
            let result = self.temporary(Type::Bool);
            let composer = self.composer();
            self.operation(
                ComposerOperation::Changed(key_type.clone()),
                vec![composer, key],
                vec![Type::Int, key_type],
                Place::local(result),
                span,
            );
            let set = self.new_block();
            let next = self.new_block();
            self.switch(self.copy(result), vec![(0, next)], set);
            self.current = set;
            self.assign(
                changed,
                Rvalue::Use(Operand::Constant(Constant::Bool(true))),
                span,
            );
            self.goto(next, span);
            self.current = next;
        }
        let read = self.new_block();
        let compute = self.new_block();
        let done = self.new_block();
        self.switch(self.copy(changed), vec![(0, read)], compute);
        self.current = read;
        let composer = self.composer();
        self.terminate(
            TerminatorKind::Remembered {
                composer,
                slot_type: value_type.clone(),
                destination: destination.clone(),
                found: done,
                missing: compute,
            },
            span,
        );
        self.current = compute;
        let value = self.temporary(value_type.clone());
        let calculation_type = self.body.locals[calculation].ty.clone();
        let calculation_parameters = match &calculation_type {
            Type::Fn { params, .. } => params.clone(),
            _ => Vec::new(),
        };
        // The window in which the calculation may create states (D15 of `owned-state`).
        let composer = self.composer();
        let unit = self.unit();
        self.operation(
            ComposerOperation::BeginRemember,
            vec![composer],
            vec![Type::Int],
            unit,
            span,
        );
        self.call(
            Callee::Value(Place::local(calculation), span),
            Vec::new(),
            calculation_parameters,
            Place::local(value),
            span,
        );
        // A `Copy` value is given by copy; any other is moved into its slot and lent from
        // there, the same borrow the found path gives (D6 of `owned-state`).
        let copied = self.types.is_copy(&value_type);
        let stored = if copied {
            self.push(
                StatementKind::assign(destination, Rvalue::Use(self.copy(value))),
                span,
            );
            self.unit()
        } else {
            destination
        };
        let composer = self.composer();
        let operand = if copied {
            self.copy(value)
        } else {
            Operand::Move(Place::local(value), span)
        };
        self.operation(
            ComposerOperation::SetSlot(value_type.clone()),
            vec![composer, operand],
            vec![Type::Int, value_type],
            stored,
            span,
        );
        let composer = self.composer();
        let unit = self.unit();
        self.operation(
            ComposerOperation::EndRemember,
            vec![composer],
            vec![Type::Int],
            unit,
            span,
        );
        self.goto(done, span);
        self.current = done;
        // The calculation was moved into the call, which would have dropped it.
        self.drop_then(Place::local(calculation), target, span);
    }

    /// `key(value) { content }` (D13): the content called in a movable group of the region's
    /// next `key` key, whose data key is `value`.
    fn key(&mut self, mut arguments: Vec<Operand>, target: BlockId, span: Span) {
        let content_type = match &arguments[1] {
            Operand::Copy(place, _) | Operand::Move(place, _) => place.ty(&self.body, self.types),
            Operand::Constant(_) => panic!("internal error: a constant content"),
        };
        let content = arguments.pop().expect("the content");
        let content = self.held_in_local(content, &content_type, span);
        let value = arguments.pop().expect("the key's value");
        let region = self.regions.last().copied();
        let ordinal = self.key_ordinals.entry(region).or_insert(0);
        let key = GroupKey {
            hash: crate::keys::movable_key(self.keys.region_or_body(self.id, region), *ordinal),
            label: format!("key {}", ordinal),
        };
        *ordinal += 1;
        let composer = self.composer();
        let unit = self.unit();
        self.operation(
            ComposerOperation::StartMovableGroup { key },
            vec![composer, value],
            vec![Type::Int, Type::Int],
            unit.clone(),
            span,
        );
        let words = self.change_words(&[], span);
        let mut call_arguments = vec![self.composer()];
        call_arguments.extend(words);
        let call_parameters = vec![Type::Int; call_arguments.len()];
        let calling = self.current;
        self.call(
            Callee::Value(Place::local(content), span),
            call_arguments,
            call_parameters,
            unit.clone(),
            span,
        );
        // The call of a composable function value is composable.
        if let TerminatorKind::Call { is_compose, .. } =
            &mut self.blocks[calling.index()].terminator.kind
        {
            *is_compose = true;
        }
        let composer = self.composer();
        self.operation(
            ComposerOperation::EndGroup,
            vec![composer],
            vec![Type::Int],
            unit,
            span,
        );
        self.drop_then(Place::local(content), target, span);
    }

    /// `Node(kind, update) { content }` and `LeafNode(kind, update)` (D3 of `ui-toolkit`): the
    /// node group of the region's next node key, started with `kind` and giving the node;
    /// `update` called with it; for `Node`, `content` called with the composer; the group
    /// ended. A node call adds no other group.
    fn node(
        &mut self,
        builtin: BuiltinFunction,
        mut arguments: Vec<Operand>,
        parameters: Vec<Type>,
        target: BlockId,
        span: Span,
    ) {
        let content = (builtin == BuiltinFunction::UiNode).then(|| {
            let content = arguments.pop().expect("the content");
            self.held_in_local(content, &parameters[2], span)
        });
        let update = arguments.pop().expect("the update function");
        let update = self.held_in_local(update, &parameters[1], span);
        let kind = arguments.pop().expect("the kind");
        let node_type = match &parameters[1] {
            Type::Fn { params, .. } => params[0].clone(),
            other => panic!("internal error: an update function of type {}", other),
        };
        let region = self.regions.last().copied();
        let ordinal = self.node_ordinals.entry(region).or_insert(0);
        let key = GroupKey {
            hash: crate::keys::node_key(self.keys.region_or_body(self.id, region), *ordinal),
            label: format!("node {}", ordinal),
        };
        *ordinal += 1;
        let node = self.temporary(node_type.clone());
        let composer = self.composer();
        self.operation(
            ComposerOperation::StartNodeGroup { key },
            vec![composer, kind],
            vec![Type::Int, Type::Int],
            Place::local(node),
            span,
        );
        let unit = self.unit();
        self.call(
            Callee::Value(Place::local(update), span),
            vec![self.copy(node)],
            vec![node_type],
            unit.clone(),
            span,
        );
        if let Some(content) = content {
            let words = self.change_words(&[], span);
            let mut call_arguments = vec![self.composer()];
            call_arguments.extend(words);
            let call_parameters = vec![Type::Int; call_arguments.len()];
            let calling = self.current;
            self.call(
                Callee::Value(Place::local(content), span),
                call_arguments,
                call_parameters,
                unit.clone(),
                span,
            );
            // The call of a composable function value is composable.
            if let TerminatorKind::Call { is_compose, .. } =
                &mut self.blocks[calling.index()].terminator.kind
            {
                *is_compose = true;
            }
        }
        let composer = self.composer();
        self.operation(
            ComposerOperation::EndGroup,
            vec![composer],
            vec![Type::Int],
            unit,
            span,
        );
        // The function values were moved into the call, which would have dropped them.
        match content {
            Some(content) => {
                let after = self.new_block();
                self.drop_then(Place::local(update), after, span);
                self.current = after;
                self.drop_then(Place::local(content), target, span);
            }
            None => self.drop_then(Place::local(update), target, span),
        }
    }

    /// `operand`, of type `ty`, in a whole local: its own when it moves one, else a temporary.
    fn held_in_local(&mut self, operand: Operand, ty: &Type, span: Span) -> LocalId {
        match &operand {
            Operand::Move(place, _) | Operand::Copy(place, _) if place.projections.is_empty() => {
                place.local
            }
            _ => {
                let local = self.temporary(ty.clone());
                self.assign(local, Rvalue::Use(operand), span);
                local
            }
        }
    }

    /// Drops `place` when its type needs a drop, then goes to `target`.
    fn drop_then(&mut self, place: Place, target: BlockId, span: Span) {
        let ty = place.ty(&self.body, self.types);
        if prism_mir::needs_drop(&ty, self.types) {
            self.terminate(TerminatorKind::Drop { place, target }, span);
        } else {
            self.goto(target, span);
        }
    }

    /// The root content adapter value built from `content` (D15), in a new temporary.
    fn adapter_value(&mut self, content: Operand, content_type: Type, span: Span) -> LocalId {
        let adapter = match &self.adapter {
            Some((adapter, _)) => *adapter,
            None => {
                let adapter = self.appended.reserve();
                self.adapter = Some((adapter, content_type.clone()));
                adapter
            }
        };
        let value = self.temporary(word_function());
        self.assign(
            value,
            Rvalue::Closure {
                body: adapter,
                captures: vec![Capture::Owned(content)],
            },
            span,
        );
        value
    }

    // The prologue and the end.

    fn finish(mut self) -> Body {
        if self.body.is_compose {
            self.composable_prologue_and_end();
        }
        if let Some((adapter, content_type)) = self.adapter.take() {
            let body =
                crate::restart::root_content_body(&self.body, self.id, content_type, self.span);
            self.appended.fill(adapter, body);
        }
        // Every new temporary is live from the entry to every return.
        let lives: Vec<Statement> = self
            .temporaries
            .iter()
            .map(|local| Statement {
                kind: StatementKind::StorageLive(*local),
                span: self.span,
            })
            .collect();
        let deads: Vec<Statement> = self
            .temporaries
            .iter()
            .rev()
            .map(|local| Statement {
                kind: StatementKind::StorageDead(*local),
                span: self.span,
            })
            .collect();
        for block in &mut self.blocks {
            if matches!(block.terminator.kind, TerminatorKind::Return) {
                block.statements.extend(deads.iter().cloned());
            }
        }
        // Version locals start with no identity, so that one read on a path where its
        // reference was never assigned is defined.
        let mut versions: Vec<LocalId> = self.version_locals.values().copied().collect();
        versions.sort();
        let initial: Vec<Statement> = versions
            .into_iter()
            .map(|local| Statement {
                kind: StatementKind::assign(Place::local(local), Rvalue::Use(Self::int(0))),
                span: self.span,
            })
            .collect();
        let entry = &mut self.blocks[0].statements;
        let rest = std::mem::take(entry);
        entry.extend(lives);
        entry.extend(initial);
        entry.extend(rest);
        let mut arena = Arena::new();
        for block in self.blocks {
            arena.alloc(block);
        }
        self.body.blocks = arena;
        self.body.groups = Arena::new();
        prism_mir::merge_goto_blocks(&mut self.body);
        self.body
    }

    /// Builds the entry and the end of a composable body: the replayed entry block moves out of
    /// `bb0`, which becomes the prologue starting the body group (D11).
    fn composable_prologue_and_end(&mut self) {
        // The replayed entry moves to a block of its own; nothing jumps to `bb0`.
        let moved = self.new_block();
        self.blocks.swap(0, moved.index());
        self.entry = moved;
        for block in &mut self.blocks {
            for successor in block.terminator.kind.successors_mut() {
                if *successor == BlockId(0) {
                    *successor = moved;
                }
            }
        }
        let (key, label) = self.keys.body(self.id);
        let key = GroupKey {
            hash: key,
            label: label.to_string(),
        };
        self.current = BlockId(0);
        let end = self.end.expect("a composable body has an end");
        let ret = self.new_block();
        if self.skippable {
            let restarted = self.skippable_prologue(key.clone());
            self.skippable_end(end, ret, restarted);
        } else {
            self.compute_passed_bits();
            let composer = self.composer();
            let unit = self.unit();
            self.operation(
                ComposerOperation::StartGroup { key },
                vec![composer],
                vec![Type::Int],
                unit.clone(),
                self.span,
            );
            self.goto(self.entry, self.span);
            self.current = end;
            let composer = self.composer();
            self.operation(
                ComposerOperation::EndGroup,
                vec![composer],
                vec![Type::Int],
                unit,
                self.span,
            );
            self.goto(ret, self.span);
        }
        self.current = ret;
        self.terminate(TerminatorKind::Return, self.span);
    }

    /// Assigns, in the current block, each parameter's bits as the body was passed them, for
    /// the parameters whose bits a call forwards.
    fn compute_passed_bits(&mut self) {
        for index in 0..self.declared {
            let Some(bits) = self.bits[index] else {
                continue;
            };
            let (word, shift) = bit_position(index);
            let word = self.changed[word];
            self.assign(
                bits,
                Rvalue::Bits(BitOperator::ShiftRight, self.copy(word), Self::int(shift)),
                self.span,
            );
            self.assign(
                bits,
                Rvalue::Bits(BitOperator::And, self.copy(bits), Self::int(3)),
                self.span,
            );
        }
    }

    /// The prologue of a skippable body (D7, D8, D11), from the current block: with function
    /// parameters, a test that each is keepable, else the replaceable path; the restart group;
    /// each argument kept by its bits; the skip decision. Returns the `Bool` recording whether
    /// the restart group was started, when there is a replaceable path.
    fn skippable_prologue(&mut self, key: GroupKey) -> Option<LocalId> {
        let span = self.span;
        let functions: Vec<usize> = (0..self.declared)
            .filter(|index| is_function_parameter(&self.declared_type(*index)))
            .collect();
        let versions: Vec<LocalId> = (0..self.declared)
            .filter_map(|index| self.identity_version(index))
            .collect();
        for index in 0..self.declared {
            self.parameter_bits(index);
        }
        let restarted =
            (!functions.is_empty() || !versions.is_empty()).then(|| self.temporary(Type::Bool));
        let replaceable = restarted.map(|_| self.new_block());
        if let Some(replaceable) = replaceable {
            // A borrow without identity may be of the caller's frame, which a restart could
            // not read later: the call runs in the replaceable group (D8 of `strong-skipping`).
            for &version in &versions {
                let next = self.new_block();
                self.switch(self.copy(version), vec![(0, replaceable)], next);
                self.current = next;
            }
            for &index in &functions {
                let keepable = self.temporary(Type::Bool);
                let borrow = self.parameter_borrow(index);
                self.operation(
                    ComposerOperation::Keepable,
                    vec![Operand::Move(Place::local(borrow), span)],
                    vec![self.body.locals[borrow].ty.clone()],
                    Place::local(keepable),
                    span,
                );
                let next = self.new_block();
                self.switch(self.copy(keepable), vec![(0, replaceable)], next);
                self.current = next;
            }
        }
        let composer = self.composer();
        let unit = self.unit();
        self.operation(
            ComposerOperation::StartRestartGroup { key: key.clone() },
            vec![composer],
            vec![Type::Int],
            unit.clone(),
            span,
        );
        if let Some(restarted) = restarted {
            self.assign(
                restarted,
                Rvalue::Use(Operand::Constant(Constant::Bool(true))),
                span,
            );
        }
        // `KEEP_CHANGED` is or-ed into `dirty` for each argument that changed.
        let dirty = self.temporary(Type::Int);
        self.assign(dirty, Rvalue::Use(Self::int(0)), span);
        for index in 0..self.declared {
            self.keep_parameter(index, dirty);
        }
        self.assign(
            dirty,
            Rvalue::Bits(BitOperator::And, self.copy(dirty), Self::int(KEEP_CHANGED)),
            span,
        );
        // Skip when nothing forces the body, nothing changed and the scope is valid.
        let body = self.new_block();
        let check = self.new_block();
        let ask = self.new_block();
        let skip = self.new_block();
        let force = self.temporary(Type::Int);
        self.assign(
            force,
            Rvalue::Bits(
                BitOperator::And,
                self.copy(self.changed[0]),
                Self::int(FORCE),
            ),
            span,
        );
        self.switch(self.copy(force), vec![(0, check)], body);
        self.current = check;
        self.switch(self.copy(dirty), vec![(0, ask)], body);
        self.current = ask;
        let skipping = self.temporary(Type::Bool);
        let composer = self.composer();
        self.operation(
            ComposerOperation::Skipping,
            vec![composer],
            vec![Type::Int],
            Place::local(skipping),
            span,
        );
        self.switch(self.copy(skipping), vec![(0, body)], skip);
        self.current = skip;
        // The skip path drops the function values the body owns, which nothing moved yet.
        for &index in &functions {
            let next = self.new_block();
            self.drop_then(Place::local(LocalId((index + 1) as u32)), next, span);
            self.current = next;
        }
        let composer = self.composer();
        self.operation(
            ComposerOperation::SkipToGroupEnd,
            vec![composer],
            vec![Type::Int],
            unit.clone(),
            span,
        );
        let end = self.end.expect("an end");
        self.goto(end, span);
        self.current = body;
        self.goto(self.entry, span);
        if let (Some(replaceable), Some(restarted)) = (replaceable, restarted) {
            // An unkeepable function argument runs the call in a replaceable group of the same
            // key, which never matches the restart group (D7).
            self.current = replaceable;
            let composer = self.composer();
            self.operation(
                ComposerOperation::StartGroup { key },
                vec![composer],
                vec![Type::Int],
                unit,
                span,
            );
            self.assign(
                restarted,
                Rvalue::Use(Operand::Constant(Constant::Bool(false))),
                span,
            );
            self.compute_passed_bits();
            self.goto(self.entry, span);
        }
        restarted
    }

    /// A shared borrow of declared parameter `index`, in a new temporary.
    fn parameter_borrow(&mut self, index: usize) -> LocalId {
        let ty = self.declared_type(index);
        let borrow = self.temporary(Type::Reference {
            mutable: false,
            inner: Box::new(ty),
        });
        self.assign(
            borrow,
            Rvalue::Borrow {
                mutable: false,
                origin: BorrowOrigin::Argument,
                place: Place::local(LocalId((index + 1) as u32)),
            },
            self.span,
        );
        borrow
    }

    /// The value of declared parameter `index` to keep in a slot: a copy, or a duplicate of a
    /// function value, which the body keeps (D8).
    fn kept_value(&mut self, index: usize) -> Operand {
        let ty = self.declared_type(index);
        if !is_function_parameter(&ty) {
            return self.copy(LocalId((index + 1) as u32));
        }
        let borrow = self.parameter_borrow(index);
        let duplicate = self.temporary(ty.clone());
        let borrow_type = self.body.locals[borrow].ty.clone();
        self.operation(
            ComposerOperation::Duplicate,
            vec![Operand::Move(Place::local(borrow), self.span)],
            vec![borrow_type],
            Place::local(duplicate),
            self.span,
        );
        Operand::Move(Place::local(duplicate), self.span)
    }

    /// Keeps declared parameter `index` in the next slot of the restart group by its bits
    /// (D8): uncertain compares it, different stores it, same and static read past the kept
    /// slot, storing it only when there is none. Sets its bits to what the body now knows, and
    /// or-s `KEEP_CHANGED` into `dirty` when it changed. A `Copy` value is kept by one call of
    /// the runtime's `keep`, which makes that decision; a function value, which is duplicated
    /// only when it is stored or compared, by a switch over its bits here.
    fn keep_parameter(&mut self, index: usize, dirty: LocalId) {
        let span = self.span;
        let ty = self.declared_type(index);
        let bits = self.parameter_bits(index);
        let (word, shift) = bit_position(index);
        let word = self.changed[word];
        let passed = self.temporary(Type::Int);
        self.assign(
            passed,
            Rvalue::Bits(BitOperator::ShiftRight, self.copy(word), Self::int(shift)),
            span,
        );
        self.assign(
            passed,
            Rvalue::Bits(BitOperator::And, self.copy(passed), Self::int(3)),
            span,
        );
        if let Some(version) = self.identity_version(index) {
            // A borrow is kept as its identity, its address and version, compared by its two
            // words (D7 of `strong-skipping`).
            let result = self.temporary(Type::Int);
            let composer = self.composer();
            self.operation(
                ComposerOperation::Keep(ty.clone()),
                vec![
                    composer,
                    self.copy(LocalId((index + 1) as u32)),
                    self.copy(version),
                    self.copy(passed),
                ],
                vec![Type::Int, ty, Type::Int, Type::Int],
                Place::local(result),
                span,
            );
            self.assign(
                bits,
                Rvalue::Bits(BitOperator::And, self.copy(result), Self::int(3)),
                span,
            );
            self.assign(
                dirty,
                Rvalue::Bits(BitOperator::Or, self.copy(dirty), self.copy(result)),
                span,
            );
            return;
        }
        if !is_function_parameter(&ty) {
            // A borrowed state is kept as its weak handle, compared by identity (D13 of
            // `owned-state`).
            let (kept, value) = if crate::stability::is_borrowed_state(&ty, self.types) {
                let kept = crate::stability::kept_type(&ty, self.types);
                let weak = self.temporary(kept.clone());
                let (state_kind, value_type) = state_object(&ty);
                let parameters = prism_mir::StateOperation::Weak.parameters(state_kind);
                self.call_with(
                    Callee::State(prism_mir::StateOperation::Weak),
                    vec![self.copy(LocalId((index + 1) as u32))],
                    parameters,
                    vec![value_type],
                    Place::local(weak),
                    span,
                );
                (kept, self.copy(weak))
            } else {
                (ty.clone(), self.copy(LocalId((index + 1) as u32)))
            };
            let result = self.temporary(Type::Int);
            let composer = self.composer();
            self.operation(
                ComposerOperation::Keep(kept.clone()),
                vec![composer, value, self.copy(passed)],
                vec![Type::Int, kept, Type::Int],
                Place::local(result),
                span,
            );
            self.assign(
                bits,
                Rvalue::Bits(BitOperator::And, self.copy(result), Self::int(3)),
                span,
            );
            self.assign(
                dirty,
                Rvalue::Bits(BitOperator::Or, self.copy(dirty), self.copy(result)),
                span,
            );
            return;
        }
        let uncertain = self.new_block();
        let same = self.new_block();
        let different = self.new_block();
        let constant = self.new_block();
        let next = self.new_block();
        self.switch(
            self.copy(passed),
            vec![
                (UNCERTAIN as u64, uncertain),
                (SAME as u64, same),
                (DIFFERENT as u64, different),
            ],
            constant,
        );
        let set_bits = |rewriter: &mut Self, value: i64, changed: bool| {
            rewriter.assign(bits, Rvalue::Use(Self::int(value)), span);
            if changed {
                rewriter.assign(
                    dirty,
                    Rvalue::Bits(
                        BitOperator::Or,
                        rewriter.copy(dirty),
                        Self::int(KEEP_CHANGED),
                    ),
                    span,
                );
            }
            rewriter.goto(next, span);
        };
        // Uncertain: compared through the slot type's equality.
        self.current = uncertain;
        let value = self.kept_value(index);
        let result = self.temporary(Type::Bool);
        let composer = self.composer();
        self.operation(
            ComposerOperation::Changed(ty.clone()),
            vec![composer, value],
            vec![Type::Int, ty.clone()],
            Place::local(result),
            span,
        );
        let equal = self.new_block();
        let unequal = self.new_block();
        self.switch(self.copy(result), vec![(0, equal)], unequal);
        self.current = equal;
        set_bits(self, SAME, false);
        self.current = unequal;
        set_bits(self, DIFFERENT, true);
        // Different: stored.
        self.current = different;
        self.store_parameter(index);
        set_bits(self, DIFFERENT, true);
        // Same and static: read past the kept slot, stored when missing.
        for (block, value) in [(same, SAME), (constant, STATIC)] {
            self.current = block;
            let scratch = self.temporary(ty.clone());
            let found = self.new_block();
            let missing = self.new_block();
            let composer = self.composer();
            self.terminate(
                TerminatorKind::Remembered {
                    composer,
                    slot_type: ty.clone(),
                    destination: Place::local(scratch),
                    found,
                    missing,
                },
                span,
            );
            self.current = missing;
            self.store_parameter(index);
            self.goto(found, span);
            self.current = found;
            set_bits(self, value, false);
        }
        self.current = next;
    }

    /// Stores declared parameter `index`'s value in the slot at the cursor.
    fn store_parameter(&mut self, index: usize) {
        let ty = self.declared_type(index);
        let value = self.kept_value(index);
        let composer = self.composer();
        let unit = self.unit();
        self.operation(
            ComposerOperation::SetSlot(ty.clone()),
            vec![composer, value],
            vec![Type::Int, ty],
            unit,
            self.span,
        );
    }

    /// The end of a skippable body: its restart group ended with its restart function, or,
    /// after the replaceable path, its replaceable group ended.
    fn skippable_end(&mut self, end: BlockId, ret: BlockId, restarted: Option<LocalId>) {
        let span = self.span;
        let restart = self.appended.reserve();
        let versioned: Vec<bool> = self.version_arguments.iter().map(Option::is_some).collect();
        let restart_body = crate::restart::restart_body(
            &self.file.bodies[self.id],
            self.id,
            &self.body,
            self.declared,
            &versioned,
            self.types,
            span,
        );
        self.appended.fill(restart, restart_body);
        let end_restart = self.new_block();
        self.current = end;
        match restarted {
            Some(restarted) => {
                let end_replaceable = self.new_block();
                self.switch(
                    self.copy(restarted),
                    vec![(0, end_replaceable)],
                    end_restart,
                );
                self.current = end_replaceable;
                let composer = self.composer();
                let unit = self.unit();
                self.operation(
                    ComposerOperation::EndGroup,
                    vec![composer],
                    vec![Type::Int],
                    unit,
                    span,
                );
                self.goto(ret, span);
            }
            None => self.goto(end_restart, span),
        }
        self.current = end_restart;
        let function = self.temporary(word_function());
        self.assign(
            function,
            Rvalue::Closure {
                body: restart,
                captures: Vec::new(),
            },
            span,
        );
        let composer = self.composer();
        let unit = self.unit();
        self.operation(
            ComposerOperation::EndRestartGroup,
            vec![composer, Operand::Move(Place::local(function), span)],
            vec![Type::Int, word_function()],
            unit,
            span,
        );
        self.goto(ret, span);
    }
}

/// A call of a compose intrinsic, taken apart.
struct IntrinsicCall {
    builtin: BuiltinFunction,
    arguments: Vec<Operand>,
    parameters: Vec<Type>,
    type_arguments: Vec<Type>,
    destination: Place,
    target: Option<BlockId>,
    span: Span,
}

/// Renumbers every local of `body` from `first` on by `by`, in its blocks: places, operands,
/// indices, storage markers, destructurings and captures. Linear in the body.
pub(crate) fn renumber_locals(body: &mut Body, first: usize, by: usize) {
    let shift = |local: &mut LocalId| {
        if local.index() >= first {
            *local = LocalId((local.index() + by) as u32);
        }
    };
    fn place(place: &mut Place, shift: &impl Fn(&mut LocalId)) {
        shift(&mut place.local);
        for projection in &mut place.projections {
            if let Projection::Index(local) = projection {
                shift(local);
            }
        }
    }
    fn operand(operand: &mut Operand, shift: &impl Fn(&mut LocalId)) {
        match operand {
            Operand::Copy(p, _) | Operand::Move(p, _) => place(p, shift),
            Operand::Constant(_) => {}
        }
    }
    for (_, block) in body.blocks.iter_mut() {
        for statement in &mut block.statements {
            match &mut statement.kind {
                StatementKind::Assign(target, rvalue) => {
                    place(target, &shift);
                    match rvalue.as_mut() {
                        Rvalue::Use(o)
                        | Rvalue::Unary(_, o)
                        | Rvalue::WrapNullable(o)
                        | Rvalue::Subtype(o) => operand(o, &shift),
                        Rvalue::Binary(_, a, b) | Rvalue::Bits(_, a, b) => {
                            operand(a, &shift);
                            operand(b, &shift);
                        }
                        Rvalue::Borrow { place: p, .. } | Rvalue::Discriminant(p) => {
                            place(p, &shift)
                        }
                        Rvalue::Aggregate(_, operands) => {
                            operands.iter_mut().for_each(|o| operand(o, &shift))
                        }
                        Rvalue::Closure { captures, .. } => {
                            for capture in captures {
                                match capture {
                                    Capture::Borrowed(p) | Capture::Weak(p) => place(p, &shift),
                                    Capture::Owned(o) => operand(o, &shift),
                                }
                            }
                        }
                        Rvalue::Template(parts) => {
                            for part in parts {
                                if let prism_mir::TemplatePart::Value(o) = part {
                                    operand(o, &shift);
                                }
                            }
                        }
                    }
                }
                StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
                    shift(local)
                }
                StatementKind::Read(p) => place(p, &shift),
                StatementKind::Destructure { source, parts, .. } => {
                    shift(source);
                    for (_, target) in parts {
                        shift(target);
                    }
                }
                StatementKind::GroupStart(_) | StatementKind::GroupEnd(_) => {}
            }
        }
        match &mut block.terminator.kind {
            TerminatorKind::SwitchInt { value, .. } => operand(value, &shift),
            TerminatorKind::Call {
                callee,
                arguments,
                destination,
                ..
            } => {
                if let Callee::Value(p, _) = callee {
                    place(p, &shift);
                }
                arguments.iter_mut().for_each(|o| operand(o, &shift));
                place(destination, &shift);
            }
            TerminatorKind::Drop { place: p, .. } => place(p, &shift),
            TerminatorKind::Remembered {
                composer,
                destination,
                ..
            } => {
                operand(composer, &shift);
                place(destination, &shift);
            }
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
        }
    }
}
