//! The MIR data model (D2 of `mir-lowering`): one control-flow graph per body, over typed locals
//! and places.
//!
//! The invariants a lowered file carries, checked by [`MirFile::validate`]:
//!
//! - **Locals** (D3). `_0` is the return place, `_1` to `_n` the arguments in parameter order
//!   (`this` first), then a closure's capture locals, then bindings and temporaries in order of
//!   creation. Every local is typed.
//! - **Blocks** (D9). `bb0` is the entry, every block is reachable from it and ends in a
//!   terminator; every jump targets a block of the body.
//! - **Types**. Every assignment's rvalue has its place's type, except [`Rvalue::Subtype`],
//!   whose operand's type differs from it; projections are well typed; a call's operands have
//!   its parameter types, and it has no successor exactly when its destination is `Nothing`.
//! - **Storage** (D6). Every local except the return place, the arguments, the captures and
//!   drop flags is marked live before its uses and dead on every path to the return, never live
//!   twice; only values whose type needs a drop ([`crate::needs_drop`]) are dropped.
//! - **Phase**. Lowering builds [`MirPhase::Built`] MIR; the borrow checker returns
//!   [`MirPhase::Elaborated`] MIR, whose drops are refined by moves and may be guarded by drop
//!   flags ([`LocalKind::DropFlag`]), and whose closure bodies record how they take each
//!   capture ([`Body::capture_modes`]), which only elaborated MIR has. Which captures a closure
//!   owns is already decided in MIR as lowered: the closure value moves or copies them
//!   ([`Capture::Owned`], D2 of `escaping-closures`).

use prism_ast::Span;
use prism_typed_tree::{
    define_id, Arena, BinaryOperator, BuiltinFunction, DeclarationPath, NominalId, OwnershipReason,
    Type, TypeParameter, TypeTable, UnaryOperator,
};

define_id!(
    /// A local of a body: `_0` is the return place.
    LocalId
);
define_id!(
    /// A basic block of a body: `bb0` is the entry.
    BlockId
);
define_id!(
    /// A body of a file.
    BodyId
);
define_id!(
    /// A region of a composable body that runs in a group of its own (D6 of
    /// `compose-transform`).
    GroupId
);

impl LocalId {
    /// The return place `_0`.
    pub const RETURN: LocalId = LocalId(0);
}

/// The MIR of one file: a body per function with a body, top-level `val`, parameter default
/// and closure (D3), in item order, each followed by the closures lowered inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct MirFile {
    /// The file's module path (`app/main`).
    pub module: String,
    /// Whether the file's drops are as lowered or elaborated by the borrow checker.
    pub phase: MirPhase,
    /// The file's type table, in which the built-in `Range` also has the field `endInclusive`
    /// (D4).
    pub types: TypeTable,
    pub bodies: Arena<BodyId, Body>,
    /// The `external` functions the file declares, which have no body (D5 of
    /// `standard-library`).
    pub externals: Vec<ExternalDeclaration>,
}

/// An `external fun` a file declares: a runtime entry point named by the C symbol equal to the
/// name, with the Prism signature that gives its machine signature (D5 of `standard-library`).
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalDeclaration {
    pub path: DeclarationPath,
    pub parameters: Vec<Type>,
    pub result: Type,
    /// The declaration, where `E6005` points.
    pub span: Span,
}

/// The stage a file's MIR is at (D12 of `borrowck-moves`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirPhase {
    /// As lowered: every value whose type needs a drop is dropped where its scope ends, whether
    /// or not it was moved (D6 of `mir-lowering`).
    Built,
    /// Borrow-checked, with drops elaborated: a drop runs only where its value is present,
    /// statically or under a drop flag.
    Elaborated,
    /// Elaborated MIR the compose transform rewrote (D19 of `compose-transform`): composable
    /// bodies take the composer and their change words and run in groups, composable calls pass
    /// them, region markers became composer operations, and the compose runtime's intrinsics
    /// are composer operations too. What records, instances and the backend read.
    Composed,
}

/// What a body is the code of.
#[derive(Debug, Clone, PartialEq)]
pub enum BodyOwner {
    /// A function or extension.
    Function(DeclarationPath),
    /// A top-level `val`'s initializer.
    Value(DeclarationPath),
    /// The generator of the default value of declared parameter `parameter` (counted without
    /// the receiver) of `function` (D11). `name` is the parameter's name.
    Default {
        function: DeclarationPath,
        parameter: usize,
        name: String,
    },
    /// The `index`-th closure lowered in the body `parent` (D13).
    Closure { parent: BodyId, index: usize },
    /// The restart function of the skippable composable `function`, a closure capturing
    /// nothing of type `(Int) -> Unit` that reads the kept arguments and calls the composable
    /// with the force bit (D10 of `compose-transform`).
    Restart { function: BodyId },
    /// The adapter through which the runtime calls a composition's content, given to
    /// `composition` and `setContent` in the body `parent`: a closure of type `(Int) -> Unit`
    /// owning the content, which it calls with the composer and the force bit (D15).
    RootContent { parent: BodyId },
}

impl BodyOwner {
    /// Whether the body's code is a closure's, called through a function value with its
    /// environment: a closure, a restart function or a root content adapter.
    pub fn is_closure(&self) -> bool {
        matches!(
            self,
            BodyOwner::Closure { .. } | BodyOwner::Restart { .. } | BodyOwner::RootContent { .. }
        )
    }

    /// Whether the body's first argument is a receiver: exactly for the body of an extension,
    /// whose declaration path names its receiver type (D3 of `borrowck-elision`).
    pub fn has_receiver(&self) -> bool {
        matches!(self, BodyOwner::Function(path) if path.receiver.is_some())
    }

    /// Whether the body's first argument is a context parameter, which precedes the receiver
    /// (D9 of `receiver-lambdas`).
    pub fn has_context(&self) -> bool {
        matches!(self, BodyOwner::Function(path) if path.context.is_some())
    }
}

/// The control-flow graph of one body.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub owner: BodyOwner,
    /// Whether the body is a `compose fun` or a composable closure.
    pub is_compose: bool,
    /// The type parameters the body's types may mention: its function's, also for the
    /// function's default generators and closures, and none for a top-level `val`. A generic
    /// body is lowered once, polymorphically (D9 of `generics`).
    pub type_parameters: Vec<TypeParameter>,
    /// The type arguments of an instance of a generic body (D6 of `monomorphization`), which
    /// replace its type parameters: empty for every body lowered from source, and never
    /// together with type parameters.
    pub type_arguments: Vec<Type>,
    /// `_0` the return place, then the arguments, then the captures, then bindings and
    /// temporaries in order of creation.
    pub locals: Arena<LocalId, LocalDeclaration>,
    /// The number of arguments: they are `_1` to `_argument_count`.
    pub argument_count: usize,
    /// The number of capture locals, which follow the arguments. Zero outside closures.
    pub capture_count: usize,
    /// How the closure takes each capture local, in capture order: empty in MIR as lowered,
    /// which leaves how a borrowed capture is borrowed to the borrow checker, and one mode per
    /// capture local in elaborated MIR (D10 of `borrowck-exclusivity`): `Owned` for the captures
    /// the closure value moves or copies, as lowered.
    pub capture_modes: Vec<CaptureMode>,
    /// For a closure that owns some of its captures, why it does, as the typed tree records it
    /// (D2 of `escaping-closures`), for diagnostics.
    pub ownership_reason: Option<OwnershipReason>,
    /// The blocks; `bb0` is the entry.
    pub blocks: Arena<BlockId, BasicBlock>,
    /// The regions of a composable body that contain composable calls, each run in a group of
    /// its own, which [`StatementKind::GroupStart`] and [`StatementKind::GroupEnd`] mark (D6 of
    /// `compose-transform`). Empty for every other body, and in composed MIR.
    pub groups: Arena<GroupId, GroupRegion>,
    /// For a composable closure, where it is written in its parent: the region and the step
    /// its group's key is derived from (D5 of `compose-transform`). `None` for every other
    /// body.
    pub origin: Option<ClosureOrigin>,
    pub span: Span,
}

/// A region of a composable body that runs in a replaceable group of its own: a branch, a
/// conditional operand, the part of a safe call after its test, or a loop around all its
/// iterations, containing a composable call (D5 and D6 of `compose-transform`).
#[derive(Debug, Clone, PartialEq)]
pub struct GroupRegion {
    /// The region it is written in, `None` for the body's own group.
    pub parent: Option<GroupId>,
    /// The construct that makes it a region, with its ordinal among the sibling constructs of
    /// that kind.
    pub step: GroupStep,
    pub span: Span,
    /// For a loop, whether a `continue` targets it. The composable calls and `remember`s made
    /// directly in such a loop's group then run in call-site groups of their own, since a
    /// `continue` can skip some of them in one iteration and not in another.
    pub continued: bool,
}

/// The construct of a region, with its ordinal among the regions of constructs of the same
/// kind in its parent region, in source order. A group's key hashes its parent's key and its
/// step, never a source position (D5 of `compose-transform`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupStep {
    /// Branch `branch` of an `if`: 0 for `then`, 1 for `else`.
    If { ordinal: u32, branch: u32 },
    /// Branch `branch` of a `when`, in branch order.
    When { ordinal: u32, branch: u32 },
    /// A whole `when` whose patterns or guards contain a composable call, around its tests and
    /// its branches' regions, so that the tests that run vary only inside it.
    WhenTests { ordinal: u32 },
    /// The right operand of `&&`.
    And { ordinal: u32 },
    /// The right operand of `||`.
    Or { ordinal: u32 },
    /// The fallback of `?:`.
    Elvis { ordinal: u32 },
    /// A `?.` chain.
    SafeCall { ordinal: u32 },
    /// A loop, around all its iterations.
    Loop { ordinal: u32 },
}

impl GroupStep {
    /// The step as the dump prints it: `if 0 then`, `when 1 branch 2`, `loop 0`.
    pub fn describe(&self) -> String {
        match self {
            GroupStep::If { ordinal, branch } => {
                format!(
                    "if {} {}",
                    ordinal,
                    if *branch == 0 { "then" } else { "else" }
                )
            }
            GroupStep::When { ordinal, branch } => format!("when {} branch {}", ordinal, branch),
            GroupStep::WhenTests { ordinal } => format!("when {} tests", ordinal),
            GroupStep::And { ordinal } => format!("and {}", ordinal),
            GroupStep::Or { ordinal } => format!("or {}", ordinal),
            GroupStep::Elvis { ordinal } => format!("elvis {}", ordinal),
            GroupStep::SafeCall { ordinal } => format!("safe call {}", ordinal),
            GroupStep::Loop { ordinal } => format!("loop {}", ordinal),
        }
    }
}

/// Where a composable closure is written in its parent body (D5 of `compose-transform`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClosureOrigin {
    /// The parent's region it is written in, `None` at the parent's top level.
    pub region: Option<GroupId>,
    pub step: ClosureStep,
}

/// How a composable closure is reached from its region.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ClosureStep {
    /// Written as argument `parameter` of a call of `callee` (its declaration path, the
    /// built-in's name, or `<value>` for a function value), the `ordinal`-th such argument of
    /// that callee and parameter in the region.
    Argument {
        callee: String,
        ordinal: u32,
        parameter: u32,
    },
    /// Any other composable closure, the `ordinal`-th such one of the region.
    Value { ordinal: u32 },
}

impl ClosureStep {
    /// The step as the dump prints it.
    pub fn describe(&self) -> String {
        match self {
            ClosureStep::Argument {
                callee,
                ordinal,
                parameter,
            } => format!("argument {} of {} call {}", parameter, callee, ordinal),
            ClosureStep::Value { ordinal } => format!("value {}", ordinal),
        }
    }
}

impl Body {
    /// The type of the return place.
    pub fn return_type(&self) -> &Type {
        &self.locals[LocalId::RETURN].ty
    }

    /// The argument locals, in parameter order.
    pub fn arguments(&self) -> impl Iterator<Item = LocalId> {
        (1..=self.argument_count).map(|i| LocalId(i as u32))
    }

    /// Whether `local` is the return place, an argument, a capture or a drop flag, none of which
    /// has storage markers.
    pub fn is_unmarked(&self, local: LocalId) -> bool {
        self.locals.get(local).is_some_and(|declaration| {
            matches!(
                declaration.kind,
                LocalKind::Return | LocalKind::Argument | LocalKind::Capture | LocalKind::DropFlag
            )
        })
    }
}

/// How a closure takes one of its captures: owned, as lowering decided it, or borrowed, as the
/// borrow checker infers it (D10 of `borrowck-exclusivity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureMode {
    /// A shared borrow: the closure only reads the binding.
    Shared,
    /// A mutable borrow: the closure writes the binding, writes through it, borrows it
    /// mutably, or holds a nested closure that captures it mutably.
    Mutable,
    /// The closure owns the binding's value, moved or copied into it when it was built (D2 of
    /// `escaping-closures`).
    Owned,
}

/// A local of a body.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalDeclaration {
    pub ty: Type,
    pub kind: LocalKind,
    /// Whether the binding is a `var`.
    pub mutable: bool,
    /// The name of the binding the local stands for.
    pub name: Option<String>,
    /// The typed-tree binding the local stands for, if any.
    pub binding: Option<prism_typed_tree::LocalId>,
    /// Whether an argument is an `escaping` parameter, which holds no borrow of the caller's
    /// (D4 of `escaping-closures`). Always false for other locals.
    pub escaping: bool,
    pub span: Span,
}

/// What a local is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    /// `_0`, the return place.
    Return,
    /// An argument: a parameter, `this`, a closure parameter, or a default generator's input.
    Argument,
    /// A closure's capture local, which stands for the captured binding (D13).
    Capture,
    /// A local `val` or `var`, or a `for` loop's variable.
    User,
    /// A value the lowering introduced.
    Temporary,
    /// A `Bool` that drop elaboration introduced, true exactly when the local whose drop it
    /// guards holds a value (D12 of `borrowck-moves`). Only elaborated MIR has drop flags; they
    /// follow every other local and have no storage markers.
    DropFlag,
}

/// A basic block: straight-line statements, then one terminator.
#[derive(Debug, Clone, PartialEq)]
pub struct BasicBlock {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

/// A statement, with the span of the source it comes from.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
}

/// What a statement does.
#[derive(Debug, Clone, PartialEq)]
pub enum StatementKind {
    /// Writes the rvalue's value to the place.
    Assign(Place, Box<Rvalue>),
    /// The local's storage starts.
    StorageLive(LocalId),
    /// The local's storage ends.
    StorageDead(LocalId),
    /// The place is read and its value discarded, so the borrow checker sees the use.
    Read(Place),
    /// Moves the whole local `source`, an enum holding variant `variant` or a struct (`None`),
    /// into one local per listed part, `(index, target)`: payload or field `index` goes to the
    /// whole local `target` (D15 of `payload-patterns`). Every non-`Copy` part is listed exactly
    /// once, so nothing is left behind; `Copy` parts may be listed or not. It is one whole-value
    /// move and one initialization per part, which is what the borrow checker sees.
    Destructure {
        source: LocalId,
        variant: Option<usize>,
        parts: Vec<(usize, LocalId)>,
    },
    /// The region `GroupId` of a composable body starts (D6 of `compose-transform`). It reads,
    /// writes, moves and drops nothing.
    GroupStart(GroupId),
    /// The region `GroupId` ends, on a path that leaves it: falling through, or by `return`,
    /// `break` or `continue`.
    GroupEnd(GroupId),
}

impl StatementKind {
    /// The assignment of `rvalue` to `place`.
    pub fn assign(place: Place, rvalue: Rvalue) -> Self {
        StatementKind::Assign(place, Box::new(rvalue))
    }
}

/// The end of a block, with the span of the source it comes from.
#[derive(Debug, Clone, PartialEq)]
pub struct Terminator {
    pub kind: TerminatorKind,
    pub span: Span,
}

/// How a block ends.
#[derive(Debug, Clone, PartialEq)]
pub enum TerminatorKind {
    Goto(BlockId),
    /// Jumps to the target of the first value equal to `value`, else to `otherwise`. `value` is
    /// a `Bool` (`0` is `false`) or an `Int`.
    SwitchInt {
        value: Operand,
        targets: Vec<(u64, BlockId)>,
        otherwise: BlockId,
    },
    /// Calls `callee` with one operand per parameter, in parameter order (the receiver first),
    /// writes its result to `destination` and continues at `target`, or never returns when
    /// `target` is `None` (the callee returns `Nothing`).
    Call {
        callee: Callee,
        arguments: Vec<Operand>,
        /// The type of each parameter, as the callee declares it: it mentions a generic callee's
        /// type parameters. The operand of parameter `i` has the type `parameters[i]` with
        /// `type_arguments` substituted ([`TerminatorKind::parameter_type`]).
        parameters: Vec<Type>,
        /// Whether each parameter is `escaping`, in the same order: what is passed for it escapes
        /// (D4 of `escaping-closures`).
        escaping: Vec<bool>,
        /// The type arguments of a generic callee, in the order of its type parameters; empty
        /// for any other callee. Inside a generic body they may mention the body's own type
        /// parameters (D9 of `generics`).
        type_arguments: Vec<Type>,
        destination: Place,
        target: Option<BlockId>,
        is_compose: bool,
    },
    /// Drops the value of `place` and continues at `target`. As lowered, drops are not refined
    /// by moves (D6); in elaborated MIR a drop runs only where its value is present.
    Drop {
        place: Place,
        target: BlockId,
    },
    Return,
    Unreachable,
    /// Reads the slot at the composer's cursor (D19 of `compose-transform`): when it holds a
    /// value of `slot_type`, copies it to `destination`, moves the cursor past it and continues
    /// at `found`; otherwise continues at `missing`. Only in composed MIR.
    Remembered {
        composer: Operand,
        slot_type: Type,
        destination: Place,
        found: BlockId,
        missing: BlockId,
    },
}

impl TerminatorKind {
    /// For a call, the type of the operand of parameter `index`: its declared type with the
    /// call's type arguments substituted.
    pub fn parameter_type(&self, index: usize) -> Option<Type> {
        match self {
            TerminatorKind::Call {
                parameters,
                type_arguments,
                ..
            } => parameters
                .get(index)
                .map(|ty| ty.substitute(type_arguments)),
            _ => None,
        }
    }

    /// The blocks this terminator may jump to, in order.
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            TerminatorKind::Goto(target) => vec![*target],
            TerminatorKind::SwitchInt {
                targets, otherwise, ..
            } => targets
                .iter()
                .map(|(_, target)| *target)
                .chain(std::iter::once(*otherwise))
                .collect(),
            TerminatorKind::Call { target, .. } => target.iter().copied().collect(),
            TerminatorKind::Drop { target, .. } => vec![*target],
            TerminatorKind::Remembered { found, missing, .. } => vec![*found, *missing],
            TerminatorKind::Return | TerminatorKind::Unreachable => Vec::new(),
        }
    }

    /// The blocks this terminator may jump to, mutably, in the order of [`Self::successors`].
    pub fn successors_mut(&mut self) -> Vec<&mut BlockId> {
        match self {
            TerminatorKind::Goto(target) => vec![target],
            TerminatorKind::SwitchInt {
                targets, otherwise, ..
            } => targets
                .iter_mut()
                .map(|(_, target)| target)
                .chain(std::iter::once(otherwise))
                .collect(),
            TerminatorKind::Call { target, .. } => target.iter_mut().collect(),
            TerminatorKind::Drop { target, .. } => vec![target],
            TerminatorKind::Remembered { found, missing, .. } => vec![found, missing],
            TerminatorKind::Return | TerminatorKind::Unreachable => Vec::new(),
        }
    }
}

/// A memory location: a local followed by projections.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Place {
    pub local: LocalId,
    pub projections: Vec<Projection>,
}

impl Place {
    /// The whole local `local`.
    pub fn local(local: LocalId) -> Self {
        Self {
            local,
            projections: Vec::new(),
        }
    }

    /// The return place `_0`.
    pub fn return_place() -> Self {
        Self::local(LocalId::RETURN)
    }

    /// This place followed by `projection`.
    pub fn project(&self, projection: Projection) -> Self {
        let mut place = self.clone();
        place.projections.push(projection);
        place
    }

    /// The local, when the place is a whole local.
    pub fn as_local(&self) -> Option<LocalId> {
        self.projections.is_empty().then_some(self.local)
    }
}

/// One step from a place to a part of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Projection {
    /// The referent of a `&T`, `&mut T` or `Heap<T>`.
    Deref,
    /// Field `index` of a struct, in declaration order.
    Field(usize),
    /// The payload of a nullable that is not `null`.
    Payload,
    /// Payload `field` of variant `variant` of an enum, typed with the enum's type arguments
    /// substituted (D15 of `payload-patterns`). Payloads of different variants share storage.
    VariantField { variant: usize, field: usize },
    /// The element of a `List<T>` at the index held in the `Int` local, which is never moved
    /// out of (D16 of `collections`). A string is never indexed: its character is computed by
    /// the built-in `get` (D5 of `runtime-strings-and-drops`).
    Index(LocalId),
}

/// A value read by an rvalue or a terminator. An operand that reads a place records the span of
/// the source that reads it, so that the borrow checker can point at the operand rather than at
/// its whole statement (both `s` of `Pair(s, s)`).
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// The place's value, copied out of it (its type is `Copy`).
    Copy(Place, Span),
    /// The place's value, moved out of it.
    Move(Place, Span),
    Constant(Constant),
}

/// A constant value.
#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    Int(i64),
    Float(f64),
    Bool(bool),
    Char(char),
    /// A string literal, which evaluates to a fresh owned `String`.
    String(String),
    Unit,
    /// `null`, of the nullable type it is used at.
    Null(Type),
    /// A shared borrow `&T` of a top-level `val` (D5).
    TopLevelValue(DeclarationPath),
}

/// A value computed by an assignment.
#[derive(Debug, Clone, PartialEq)]
pub enum Rvalue {
    Use(Operand),
    /// A borrow of the place, shared or mutable, and why it exists.
    Borrow {
        mutable: bool,
        origin: BorrowOrigin,
        place: Place,
    },
    /// An arithmetic or comparison operator; both operands have one type.
    Binary(BinaryOperator, Operand, Operand),
    Unary(UnaryOperator, Operand),
    /// The variant index of an enum place, or the tag of a nullable place (`0` for `null`, `1`
    /// otherwise), as an `Int`.
    Discriminant(Place),
    /// A struct or variant built from its fields, in declaration order.
    Aggregate(AggregateKind, Vec<Operand>),
    /// `T` to `T?`: sets the tag.
    WrapNullable(Operand),
    /// Any other subtyping step, to the type of the assigned place (D2).
    Subtype(Operand),
    /// The value of the closure `body`, built from its captures in capture order (D13): the
    /// place of each binding it borrows, and a move or copy of each one it owns.
    Closure {
        body: BodyId,
        captures: Vec<Capture>,
    },
    /// A string template's `String`, built from its parts in order (D10 of
    /// `string-templates`): constant text, and values, each a `&String` (a shared `Operator`
    /// borrow) or an `Int`, `Double`, `Bool` or `Char`.
    Template(Vec<TemplatePart>),
    /// A bitwise operation on two `Int`s, without overflow checks: how composed MIR builds
    /// change words (D19 of `compose-transform`). No source operator lowers to it yet.
    Bits(BitOperator, Operand, Operand),
}

/// A bitwise operator of [`Rvalue::Bits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BitOperator {
    And,
    Or,
    ShiftLeft,
    /// A logical shift: zeros come in from the left.
    ShiftRight,
}

impl BitOperator {
    /// The operator as the dump prints it.
    pub fn as_str(self) -> &'static str {
        match self {
            BitOperator::And => "&",
            BitOperator::Or => "|",
            BitOperator::ShiftLeft => "<<",
            BitOperator::ShiftRight => ">>>",
        }
    }
}

/// How a closure value takes one captured binding (D2 of `escaping-closures`).
#[derive(Debug, Clone, PartialEq)]
pub enum Capture {
    /// The place of a binding the closure borrows, shared or mutably as the borrow checker
    /// infers.
    Borrowed(Place),
    /// A move or a copy of a binding the closure owns.
    Owned(Operand),
}

impl Capture {
    /// The captured binding's place.
    pub fn place(&self) -> Option<&Place> {
        match self {
            Capture::Borrowed(place) => Some(place),
            Capture::Owned(Operand::Copy(place, _) | Operand::Move(place, _)) => Some(place),
            Capture::Owned(Operand::Constant(_)) => None,
        }
    }

    /// Whether the closure owns the captured value.
    pub fn is_owned(&self) -> bool {
        matches!(self, Capture::Owned(_))
    }
}

/// A part of an [`Rvalue::Template`].
#[derive(Debug, Clone, PartialEq)]
pub enum TemplatePart {
    /// Constant text.
    Text(String),
    /// A value inserted as its text: a `&String`, or a scalar.
    Value(Operand),
}

impl Rvalue {
    /// The value parts of a template rvalue, in order; none for any other rvalue.
    pub fn template_operands(&self) -> impl Iterator<Item = &Operand> {
        let parts: &[TemplatePart] = match self {
            Rvalue::Template(parts) => parts,
            _ => &[],
        };
        parts.iter().filter_map(|part| match part {
            TemplatePart::Value(operand) => Some(operand),
            TemplatePart::Text(_) => None,
        })
    }
}

/// What an aggregate builds, with the type arguments of a generic struct or enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregateKind {
    Struct {
        id: NominalId,
        arguments: Vec<Type>,
    },
    Variant {
        enum_id: NominalId,
        index: usize,
        arguments: Vec<Type>,
    },
}

impl AggregateKind {
    /// The aggregate of the non-generic struct `id`.
    pub fn plain_struct(id: NominalId) -> Self {
        AggregateKind::Struct {
            id,
            arguments: Vec::new(),
        }
    }

    /// The type of the value the aggregate builds.
    pub fn ty(&self) -> Type {
        match self {
            AggregateKind::Struct { id, arguments } => Type::Struct {
                id: id.clone(),
                arguments: arguments.clone(),
            },
            AggregateKind::Variant {
                enum_id, arguments, ..
            } => Type::Enum {
                id: enum_id.clone(),
                arguments: arguments.clone(),
            },
        }
    }
}

/// What a call calls (D10).
#[derive(Debug, Clone, PartialEq)]
pub enum Callee {
    /// A top-level function or extension, of the file or of a direct import.
    Function(DeclarationPath),
    /// An `external fun` of the bundled library, called directly by the C symbol equal to its
    /// name (D5 of `standard-library`). It is never generic and never an extension.
    External(DeclarationPath),
    /// A function the compiler provides.
    Builtin(BuiltinFunction),
    /// A function-typed value, read in place, with the span of the source that names it.
    Value(Place, Span),
    /// The generator of the default value of declared parameter `parameter` (named `name`) of
    /// `function` (D11).
    Default {
        function: DeclarationPath,
        parameter: usize,
        name: String,
    },
    /// An operation of the composition protocol, a call of a compose runtime entry point (D19
    /// of `compose-transform`). Only in composed MIR.
    Composer(ComposerOperation),
}

/// A group key: its 64-bit hash, and what it is the key of, for dumps (D5 of
/// `compose-transform`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupKey {
    pub hash: u64,
    /// `body main::Counter`, `if 0 then`, `key 0`.
    pub label: String,
}

/// An operation of the composition protocol (D19 of `compose-transform`). The composer is the
/// first operand of every operation of a pass; the composition operations take the
/// composition or the scope instead.
#[derive(Debug, Clone, PartialEq)]
pub enum ComposerOperation {
    /// `(composer)`: starts the restart group of a skippable body.
    StartRestartGroup { key: GroupKey },
    /// `(composer)`: starts a replaceable group.
    StartGroup { key: GroupKey },
    /// `(composer, data key)`: starts the movable group of a `key` call.
    StartMovableGroup { key: GroupKey },
    /// `(composer)`: ends a replaceable or movable group.
    EndGroup,
    /// `(composer, restart function moved)`: ends a restart group, which keeps the function.
    EndRestartGroup,
    /// `(composer) -> Bool`: whether the innermost restart group may skip its body.
    Skipping,
    /// `(composer)`: keeps the rest of the innermost group as it is.
    SkipToGroupEnd,
    /// `(composer, value moved) -> Bool`: compares the value with the slot at the cursor,
    /// storing it when it differs.
    Changed(Type),
    /// `(composer, value moved)`: stores the value in the slot at the cursor.
    SetSlot(Type),
    /// `(composer, value moved, passed bits) -> Int`: keeps an argument of a skippable body in
    /// the slot at the cursor by the change bits its caller passed: compared when uncertain,
    /// stored when different, read past when same or static (stored when missing). Gives the
    /// bits the body now knows, plus 4 when the value changed (D8 of `compose-transform`).
    Keep(Type),
    /// `(composer) -> T`: during a restart, the value kept in slot `index` of the scope's
    /// group, copied out or duplicated.
    KeptParameter { index: u32, ty: Type },
    /// `(&function value) -> Bool`: whether the function value can be kept and compared.
    Keepable,
    /// `(&function value) -> function value`: a duplicate of a keepable function value.
    Duplicate,
    /// `(composer) -> RecomposeScope`: the current scope.
    CurrentScope,
    /// `(root content adapter moved) -> Composition`: starts a composition.
    StartComposition,
    /// `(&Composition, root content adapter moved)`: sets a composition's content.
    SetContent,
    /// `(&Composition) -> Bool`: recomposes a composition's invalid scopes.
    Recompose,
    /// `(&Composition) -> String`: dumps a composition.
    Dump,
    /// `(&RecomposeScope)`: invalidates a scope.
    Invalidate,
}

impl ComposerOperation {
    /// The operation's name in dumps.
    pub fn name(&self) -> &'static str {
        match self {
            ComposerOperation::StartRestartGroup { .. } => "start_restart_group",
            ComposerOperation::StartGroup { .. } => "start_group",
            ComposerOperation::StartMovableGroup { .. } => "start_movable_group",
            ComposerOperation::EndGroup => "end_group",
            ComposerOperation::EndRestartGroup => "end_restart_group",
            ComposerOperation::Skipping => "skipping",
            ComposerOperation::SkipToGroupEnd => "skip_to_group_end",
            ComposerOperation::Changed(_) => "changed",
            ComposerOperation::SetSlot(_) => "set_slot",
            ComposerOperation::Keep(_) => "keep",
            ComposerOperation::KeptParameter { .. } => "kept_parameter",
            ComposerOperation::Keepable => "keepable",
            ComposerOperation::Duplicate => "duplicate",
            ComposerOperation::CurrentScope => "current_scope",
            ComposerOperation::StartComposition => "start_composition",
            ComposerOperation::SetContent => "set_content",
            ComposerOperation::Recompose => "recompose",
            ComposerOperation::Dump => "dump",
            ComposerOperation::Invalidate => "invalidate",
        }
    }

    /// Whether the operation works on a composition or a scope rather than on a pass, so it
    /// may be called outside composable bodies.
    pub fn is_composition_operation(&self) -> bool {
        matches!(
            self,
            ComposerOperation::StartComposition
                | ComposerOperation::SetContent
                | ComposerOperation::Recompose
                | ComposerOperation::Dump
                | ComposerOperation::Invalidate
                | ComposerOperation::Keepable
                | ComposerOperation::Duplicate
        )
    }

    /// The slot type the operation names, if any.
    pub fn slot_type(&self) -> Option<&Type> {
        match self {
            ComposerOperation::Changed(ty)
            | ComposerOperation::SetSlot(ty)
            | ComposerOperation::Keep(ty)
            | ComposerOperation::KeptParameter { ty, .. } => Some(ty),
            _ => None,
        }
    }
}

impl Callee {
    /// Whether the call's first operand is a receiver: exactly for a call of an extension,
    /// whose declaration path names its receiver type, and of the built-in members: `clone`,
    /// `length` and `get` (a string's character), which take a shared receiver borrow (D3 of
    /// `borrowck-elision`), `toString`, which takes the receiver copied out, and `unbox`, which
    /// takes the box moved, and the list members, which borrow the list as their mode says (D16
    /// of `collections`). The arguments of `print`, `println`, `Heap` and `listOf` are not
    /// receivers. A function value is never an extension, since an extension cannot be named as
    /// a value.
    pub fn has_receiver(&self) -> bool {
        match self {
            Callee::Function(path) => path.receiver.is_some(),
            Callee::Builtin(builtin) => builtin.has_receiver(),
            Callee::External(_)
            | Callee::Value(..)
            | Callee::Default { .. }
            | Callee::Composer(_) => false,
        }
    }

    /// Whether the call's first operand is a context argument, before the receiver: for a call
    /// of an extension with a context parameter (D9 of `receiver-lambdas`). A plain function's
    /// context is an ordinary input borrow.
    pub fn has_context(&self) -> bool {
        matches!(self, Callee::Function(path) if path.context.is_some())
    }
}

/// Why a borrow exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowOrigin {
    /// Written `&e` or `&mut e`.
    Written,
    /// Implied by a reference parameter, or a reborrow of an optional mutable borrow (D12).
    Argument,
    /// Implied by an extension's receiver mode, taken after the other arguments.
    Receiver,
    /// Made by a nullable conversion (`&(T?)` read as `(&T)?`).
    Conversion,
    /// An operand of an operator over a type that is not `Copy`, which reads it in place
    /// (D14): `==`, `!=` or string concatenation, or a `String` part of a string template (D10
    /// of `string-templates`); or the pinned target of a compound assignment whose value may
    /// re-point it (D5 of `compound-assignment`).
    Operator,
    /// An input of a default value's generator, or the slot it may fill (D11).
    Default,
    /// A pattern binding of a borrowed subject's part (D16 of `payload-patterns`).
    Pattern,
    /// The subject of a guarded pattern that takes it apart, held while the guard runs so that
    /// the guard cannot assign, move or mutably borrow what the branch then takes apart (D9 of
    /// `payload-patterns`).
    Guard,
    /// The borrow of the list a `for` loop iterates, and the borrows of its elements the loop
    /// variable takes (D16 of `collections`).
    Iteration,
}

impl From<prism_typed_tree::BorrowOrigin> for BorrowOrigin {
    fn from(origin: prism_typed_tree::BorrowOrigin) -> Self {
        match origin {
            prism_typed_tree::BorrowOrigin::Written => BorrowOrigin::Written,
            prism_typed_tree::BorrowOrigin::Argument => BorrowOrigin::Argument,
            prism_typed_tree::BorrowOrigin::Receiver => BorrowOrigin::Receiver,
            prism_typed_tree::BorrowOrigin::Conversion => BorrowOrigin::Conversion,
            prism_typed_tree::BorrowOrigin::Iteration => BorrowOrigin::Iteration,
        }
    }
}

impl BorrowOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            BorrowOrigin::Written => "written",
            BorrowOrigin::Argument => "argument",
            BorrowOrigin::Receiver => "receiver",
            BorrowOrigin::Conversion => "conversion",
            BorrowOrigin::Iteration => "iteration",
            BorrowOrigin::Operator => "operator",
            BorrowOrigin::Default => "default",
            BorrowOrigin::Pattern => "pattern",
            BorrowOrigin::Guard => "guard",
        }
    }
}
