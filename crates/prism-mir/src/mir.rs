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
//!   flags ([`LocalKind::DropFlag`]), and whose closure bodies record how they borrow each
//!   capture ([`Body::capture_modes`]), which only elaborated MIR has.

use prism_ast::Span;
use prism_typed_tree::{
    define_id, Arena, BinaryOperator, BuiltinFunction, DeclarationPath, NominalId, Type, TypeTable,
    UnaryOperator,
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
}

/// The control-flow graph of one body.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    pub owner: BodyOwner,
    /// Whether the body is a `compose fun` or a composable closure.
    pub is_compose: bool,
    /// `_0` the return place, then the arguments, then the captures, then bindings and
    /// temporaries in order of creation.
    pub locals: Arena<LocalId, LocalDeclaration>,
    /// The number of arguments: they are `_1` to `_argument_count`.
    pub argument_count: usize,
    /// The number of capture locals, which follow the arguments. Zero outside closures.
    pub capture_count: usize,
    /// How the closure borrows each capture local, in capture order: empty in MIR as lowered,
    /// which leaves the decision to the borrow checker, and one mode per capture local in
    /// elaborated MIR (D10 of `borrowck-exclusivity`).
    pub capture_modes: Vec<CaptureMode>,
    /// The blocks; `bb0` is the entry.
    pub blocks: Arena<BlockId, BasicBlock>,
    pub span: Span,
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

/// How a closure borrows one of its captures, as the borrow checker infers it (D10 of
/// `borrowck-exclusivity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureMode {
    /// A shared borrow: the closure only reads the binding.
    Shared,
    /// A mutable borrow: the closure writes the binding, writes through it, borrows it
    /// mutably, or holds a nested closure that captures it mutably.
    Mutable,
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
        /// The type of each parameter, as the callee declares it.
        parameters: Vec<Type>,
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
}

impl TerminatorKind {
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
    /// The character of a `String` at the index held in the local.
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
    /// A top-level function, as a value.
    Function(DeclarationPath),
    /// A variant with a payload, as its constructor function.
    VariantConstructor {
        enum_id: NominalId,
        index: usize,
    },
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
    /// The value of the closure `body`, capturing the places of the captured bindings, in
    /// capture order (D13).
    Closure {
        body: BodyId,
        captures: Vec<Place>,
    },
}

/// What an aggregate builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregateKind {
    Struct(NominalId),
    Variant { enum_id: NominalId, index: usize },
}

/// What a call calls (D10).
#[derive(Debug, Clone, PartialEq)]
pub enum Callee {
    /// A top-level function or extension, of the file or of a direct import.
    Function(DeclarationPath),
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
    /// (D14): `==`, `!=` or string concatenation.
    Operator,
    /// An input of a default value's generator, or the slot it may fill (D11).
    Default,
}

impl From<prism_typed_tree::BorrowOrigin> for BorrowOrigin {
    fn from(origin: prism_typed_tree::BorrowOrigin) -> Self {
        match origin {
            prism_typed_tree::BorrowOrigin::Written => BorrowOrigin::Written,
            prism_typed_tree::BorrowOrigin::Argument => BorrowOrigin::Argument,
            prism_typed_tree::BorrowOrigin::Receiver => BorrowOrigin::Receiver,
            prism_typed_tree::BorrowOrigin::Conversion => BorrowOrigin::Conversion,
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
            BorrowOrigin::Operator => "operator",
            BorrowOrigin::Default => "default",
        }
    }
}
