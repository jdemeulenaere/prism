//! Main TypeChecker implementation: Pass 1 declaration registration and Pass 2 body checking.
//!
//! Pass 2 builds the file's typed tree while it checks (D1 of `typed-tree`): every `infer_*` and
//! `check_*` function builds the node of the expression it checks and returns its id, and the
//! point that decides something (a coercion, a move, a borrow, a receiver pass) records it.

use crate::builder::{BodyBuilder, FileBuilder};
use crate::captures::{decide_captures, BodyKind};
use crate::error::{MissingImplicitParameter, NoTextFormReason, TypeError};
use crate::exhaustiveness;
use crate::inference::Unknowns;
use crate::scope::{
    is_type_symbol, DetachedLocals, ImportLookup, Mutability, ScopeStack, Symbol, SymbolKind,
};
use crate::types::{
    type_from_abi, type_parameters_from_abi, NominalId, Type, TypeDef, TypeParameter,
    TypeRelations, BUILTIN_MODULE,
};
use prism_abi::{AbiType, ExportedSymbol, FileAbi, FunctionKind};
use prism_ast::decl::{Declaration, FunctionBody, FunctionDecl, ReceiverMode, TopLevelVarDecl};
use prism_ast::expr::{
    BinaryOp, CallArg, Expr, ExprKind, Literal, PatternKind, TemplatePart, UnaryOp, WhenBranch,
    WhenPattern,
};
use prism_ast::module::SourceFile;
use prism_ast::statement::{Block, Statement, StatementKind};
use prism_ast::{Span, TypeMarker, Visibility};
use prism_typed_tree::{
    Argument, BinaryOperator, BorrowOrigin, BuiltinFunction, Call, Callee, CoercionKind,
    ConstructField, ConstructKind, DeclarationPath, EnumItem, ExpressionId, ExpressionKind,
    ForIteration, FunctionItem, Item, Local, LocalKind, LogicalOperator, Pattern,
    SafeNavigationOrigin, StatementId, StatementKind as TreeStatementKind, StructItem,
    TemplatePart as TreeTemplatePart, TypeTable, TypedFile, UnaryOperator, ValueItem,
};
use std::collections::{HashMap, HashSet};

mod patterns;

/// The help of a type argument that cannot be inferred (`E4013`, D8 of `generics`).
const TYPE_ARGUMENTS_HELP: &str =
    "write the type arguments, as in 'name<...>(...)', or annotate the expected type";

/// The name of the built-in box type and of its constructor (D2 of `heap-boxes`).
const HEAP: &str = "Heap";

/// The name of the built-in list type (D2 of `collections`), reserved like [`HEAP`].
const LIST: &str = "List";

/// The name of the built-in list constructor (D4 of `collections`), which declarations of the
/// name shadow, like the output built-ins.
const LIST_OF: &str = "listOf";

/// The help of an element type `listOf` cannot infer (`E4013`, D4 of `collections`).
const LIST_ELEMENT_HELP: &str =
    "write the element type, as in 'listOf<Int>()', or annotate the expected type, as in 'val xs: List<Int> = listOf()'";

/// The name the receiver of an extension function is bound to in its body. It is a keyword, so no
/// user declaration can use it.
const THIS: &str = "this";

/// The name a call records for a context parameter (D9 of `receiver-lambdas`).
const CONTEXT: &str = "context";
/// The name of a closure's implicit parameter (D1 of `implicit-it`).
const IMPLICIT_PARAMETER: &str = "it";

/// Helper to peel reference (`&T`, `&mut T`) and `Heap<T>` indirection layers to access the underlying nominal type.
pub fn peel_indirection(mut ty: &Type) -> &Type {
    loop {
        match ty {
            Type::Reference { inner, .. } => ty = inner,
            Type::Heap(inner) => ty = inner,
            _ => return ty,
        }
    }
}

/// The type constructor of an ABI receiver type, as the intrinsic catalog names receivers:
/// `Int`, `String`, `List`, or a struct or enum's name.
fn abi_constructor_name(ty: &AbiType) -> String {
    match ty {
        AbiType::Nominal { name, .. } | AbiType::Unbound { name, .. } => name.clone(),
        AbiType::List(_) => LIST.to_string(),
        AbiType::Heap(_) => HEAP.to_string(),
        AbiType::Prim(prism_abi::PrimKind::Float) => "Double".to_string(),
        other => other.to_string(),
    }
}

/// Every type recorded in an exported symbol's signature or definition.
fn abi_symbol_types(sym: &ExportedSymbol) -> Vec<&AbiType> {
    match sym {
        ExportedSymbol::Function {
            receiver,
            params,
            return_type,
            ..
        } => receiver
            .iter()
            .map(|r| &r.ty)
            .chain(params.iter().map(|parameter| &parameter.ty))
            .chain(std::iter::once(return_type))
            .collect(),
        ExportedSymbol::Struct { fields, .. } => fields.iter().map(|(_, _, ty)| ty).collect(),
        ExportedSymbol::Enum { variants, .. } => variants.iter().flat_map(|(_, p)| p).collect(),
        ExportedSymbol::Var { ty, .. } => vec![ty],
        ExportedSymbol::ExtensionProperty { receiver, ty, .. } => vec![receiver, ty],
    }
}

/// Names parameter `index` of a callee in a diagnostic: `'content'` for a declared function, or
/// `parameter 2` (counting from 1) for a function-typed value or variant, whose parameters have
/// no names.
fn describe_parameter(param_names: &[String], index: usize) -> String {
    match param_names.get(index) {
        Some(name) => format!("'{}'", name),
        None => format!("parameter {}", index + 1),
    }
}

/// Peels reference (`&T`, `&mut T`) layers, keeping `Heap<T>`.
fn peel_references(mut ty: &Type) -> &Type {
    while let Type::Reference { inner, .. } = ty {
        ty = inner;
    }
    ty
}

/// Where the expected type of the value being checked comes from. It picks the code of an
/// assignability mismatch: a returned value reports `E4005`, an argument `E4004`, and any other
/// expectation (annotation, assignment, operand, default value) `E4002`.
///
/// The origin is inherited by `if`/`when` branches and a block's final expression, which produce
/// the checked value, and reset to `Value` by every other nested expectation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Value,
    Return,
    Argument,
}

/// How a value is made to fit the type expected where it is used, when its own type is not
/// assignable as it is (the `references` spec). [`TypeChecker::apply_adjustment`] turns it into
/// typed-tree nodes (D5 of `typed-tree`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Adjustment {
    /// The value is used as it is: moved, or copied when it is `Copy`.
    None,
    /// A `Copy` value read out of `layers` references and boxes, inside a `?` when
    /// `through_nullable` (`(&Int)?` used as an `Int?`).
    CopyOut {
        layers: usize,
        through_nullable: bool,
    },
    /// An argument place borrowed where it lives (`greet(name)`).
    Borrow,
    /// An argument that is not a place, evaluated into a temporary that is borrowed until the end
    /// of the statement (`greet("Ada")`).
    BorrowTemporary,
    /// An argument place of type `T?` borrowed as a `(&T)?` (`show(u.nick)`).
    BorrowContents,
    /// An argument place of type `&mut T` reborrowed for the call instead of moved.
    Reborrow,
    /// A borrowed nullable `&(T?)` read as an optional borrow `(&T)?`.
    NullableView,
    /// The contents of a box borrowed (D4 of `heap-boxes`), reached by `steps` dereferences of
    /// references and boxes and, when `through_nullable`, the payload of one nullable: an
    /// argument lending them to a reference parameter (`eval(h)` for `h: Heap<Expr>`), or a
    /// borrowed box viewed as borrowed contents (`&Heap<T>` as `&T`, `&(Heap<T>?)` as `(&T)?`).
    BoxContents {
        steps: usize,
        through_nullable: bool,
        mutable: bool,
        origin: BorrowOrigin,
    },
}

/// Where a value is checked against an expected type, which decides the implicit borrows
/// allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Site {
    /// Any value position other than an argument: no implicit borrow.
    Value,
    /// A call argument whose parameter is a reference, and whether the argument is a place.
    Argument { is_place: bool },
}

/// What the body of a closure is checked for, in [`TypeChecker::check_closure`].
#[derive(Debug, Clone, Copy)]
enum ClosureReturn<'t> {
    /// The body is checked against the expected return type.
    Expected(&'t Type),
    /// The body's type is the closure's return type.
    Inferred,
    /// The body's type is the closure's return type, which solves a by-value type parameter of
    /// a generic call. Type arguments are borrow-free, so a borrowed result is copied out of
    /// its borrows, or rejected as a move out of a borrow (`E4017`), and the closure returns
    /// the referent (`generics`, "Type Arguments Are Borrow-Free").
    InferredReferent,
}

/// The outcome of [`TypeChecker::coerce`].
#[derive(Debug, Clone, PartialEq)]
enum Coercion {
    /// The value fits after the adjustment.
    Fits(Adjustment),
    /// The value fits only by moving `ty` out of a borrow, which is not `Copy` (`E4017`).
    /// `nullable` is true when the borrow is inside a `?`, and `through_box` when every layer
    /// around `ty` is a `Heap<T>`, whose contents move out only through `unbox()`.
    MovesOutOfBorrow {
        ty: Type,
        nullable: bool,
        through_box: bool,
    },
    /// The value does not fit.
    NoFit,
}

/// The arguments of a call normalized to its parameters (D8 of `typed-tree`).
struct NormalizedArguments {
    /// One argument per parameter, in parameter order, with a default slot for each omitted one.
    arguments: Vec<Argument>,
    /// The parameter indices of the written arguments, in source order.
    evaluation_order: Vec<usize>,
}

/// The nodes of the arguments checked before a call was found not to match its callee.
struct CheckedArguments(Vec<ExpressionId>);

/// The declared signature of a generic callee (a function, extension, struct constructor or
/// payload variant), whose types mention its `type_parameters`.
struct GenericSignature {
    /// How diagnostics name the callee ("function 'id'").
    owner: String,
    type_parameters: Vec<TypeParameter>,
    /// The declared parameter types.
    params: Vec<Type>,
    /// The declared return type.
    ret: Type,
    /// For an extension: its declared receiver type and the type it is called on.
    receiver: Option<(Type, Type)>,
}

/// The state of the arguments of a generic call while its type arguments are inferred (D8 of
/// `generics`).
struct GenericArguments<'a> {
    owner: String,
    type_parameters: Vec<TypeParameter>,
    params: Vec<Type>,
    unknowns: Unknowns,
    /// Whether the type arguments were written, so nothing is inferred.
    explicit: bool,
    /// Arguments inferred before the unknowns their parameter types mention were solved, to fit
    /// once they are.
    synthesized: Vec<SynthesizedArgument<'a>>,
    /// Closure arguments whose parameter types mention unsolved unknowns, checked last.
    deferred: Vec<(usize, &'a Expr)>,
}

/// A call `target.member<type_arguments>(args)` whose target is a value, checked against
/// `expected` when the call is checked against a type.
struct MethodCall<'a> {
    callee: &'a Expr,
    target: &'a Expr,
    member: &'a str,
    is_safe_nav: bool,
    type_arguments: &'a [prism_ast::types::Type],
    args: &'a [CallArg],
    expected: Option<&'a Type>,
}

/// A call `target.unbox()` whose receiver, already typed as the node `receiver`, is a box.
struct UnboxCall<'a> {
    target: &'a Expr,
    receiver: ExpressionId,
    /// Whether the receiver is a place, read in place, rather than a consumed temporary.
    is_place: bool,
    is_safe_nav: bool,
    type_arguments: &'a [prism_ast::types::Type],
    args: &'a [CallArg],
}

/// An argument inferred before its parameter type was known (D8 of `generics`, step 4).
struct SynthesizedArgument<'a> {
    index: usize,
    expr: &'a Expr,
    found: ExpressionId,
    site: Site,
    /// Whether the argument is a place whose move check waits for the fit.
    deferred_move: bool,
}

/// Checks the written argument `value` of parameter `index` of a call, returning its node, or
/// `None` when the argument is deferred (a closure whose parameter types are not inferred yet).
type ArgumentChecker<'a, 'b> =
    dyn FnMut(&mut TypeChecker, usize, &'a Expr) -> Option<ExpressionId> + 'b;

/// An extension function known to the file (the `extension-functions` spec): declared in it, or
/// exported by a direct import or a prelude module. Extensions live in their own tables, keyed
/// by receiver type and name, apart from the name scopes, so `fun greet()` and
/// `fun User.greet()` coexist. An intrinsic extension of the bundled library is a member of its
/// receiver type instead (D7 of `standard-library`), in the members table.
#[derive(Debug, Clone)]
pub(crate) struct Extension {
    pub(crate) mode: ReceiverMode,
    /// The declared receiver type, which may mention the extension's type parameters
    /// (`Box<T>` in `fun <T> Box<T>.get()`).
    pub(crate) receiver: Type,
    /// The defining module.
    pub(crate) module: String,
    /// The function's signature, parameter names, defaults and visibility. Its name is the
    /// method's name. An intrinsic property's symbol is a function of no parameters returning
    /// the property's type.
    pub(crate) symbol: Symbol,
    /// The built-in function an intrinsic member is a call of (D6 of `standard-library`);
    /// `None` for a declared extension.
    pub(crate) builtin: Option<BuiltinFunction>,
    /// Whether the member is an intrinsic property, read without parentheses (`s.length`).
    pub(crate) property: bool,
    /// Whether the intrinsic property is declared `intrinsic var`, so it can be assigned (D4
    /// of `state-handles`).
    pub(crate) writable: bool,
    /// The type of the extension's context parameter, if it has one (D8 of `receiver-lambdas`).
    pub(crate) context: Option<Type>,
}

/// The extensions of one receiver type constructor and name in one layer (the file's own, the
/// imports', the prelude's), by context type, `None` for the one without a context, each with
/// the modules declaring it (D8 of `receiver-lambdas`). A call finds its extension with one map
/// lookup per context candidate, whatever the number of contexts.
#[derive(Debug, Clone, Default)]
pub(crate) struct ExtensionsOfName {
    by_context: HashMap<Option<Type>, Vec<(String, Extension)>>,
    /// The contexts in the order they were first declared, for diagnostics.
    order: Vec<Option<Type>>,
}

impl ExtensionsOfName {
    /// Adds `extension` of `module`, unless that module already declares one of its context;
    /// returns whether it was added.
    pub(crate) fn insert(&mut self, module: &str, extension: Extension) -> bool {
        if !self.by_context.contains_key(&extension.context) {
            self.order.push(extension.context.clone());
        }
        let declared = self
            .by_context
            .entry(extension.context.clone())
            .or_default();
        if declared.iter().any(|(m, _)| m == module) {
            return false;
        }
        declared.push((module.to_string(), extension));
        true
    }

    /// Whether an extension of the context `context` is declared.
    fn declares(&self, context: &Option<Type>) -> bool {
        self.by_context.contains_key(context)
    }

    /// The extensions of the context `context`, with their modules.
    fn of_context(&self, context: &Option<Type>) -> &[(String, Extension)] {
        self.by_context.get(context).map_or(&[], Vec::as_slice)
    }

    /// The extension of the context `context`, mutably, when one module declares it.
    fn of_context_mut(&mut self, context: &Option<Type>) -> Option<&mut Extension> {
        match self.by_context.get_mut(context)?.as_mut_slice() {
            [(_, extension)] => Some(extension),
            _ => None,
        }
    }

    /// The context types declared, in the order they were first declared.
    fn contexts(&self) -> Vec<Type> {
        self.order.iter().flatten().cloned().collect()
    }
}

impl Extension {
    /// The extension's declaration path.
    fn path(&self, name: &str) -> DeclarationPath {
        DeclarationPath::extension(self.module.clone(), self.receiver.clone(), name)
            .with_context(self.context.clone())
    }
}

/// What extensions are keyed by: the type constructor of their receiver, without type arguments
/// (D10 of `generics`), so `fun Box<Int>.sum()` and `fun <T> Box<T>.sum()` share a key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ReceiverKey {
    /// A struct or enum, by identity.
    Nominal(NominalId),
    /// Any list, whatever its element type (D2 of `collections`).
    List,
    /// Any other receiver type, as it is.
    Other(Type),
}

impl ReceiverKey {
    pub(crate) fn of(ty: &Type) -> Self {
        match ty {
            Type::List(_) => ReceiverKey::List,
            _ => match ty.nominal_id() {
                Some(id) => ReceiverKey::Nominal(id.clone()),
                None => ReceiverKey::Other(ty.clone()),
            },
        }
    }
}

/// The result of looking an extension up by receiver type and name.
enum ExtensionLookup {
    /// No visible extension matches.
    None,
    /// The file's own extension, or the single import exporting one.
    Found {
        extension: Box<Extension>,
        /// Whether the extension is the file's own, whose type may be inferred on demand.
        own: bool,
    },
    /// Several imports export a matching extension; holds their module paths (`E4012`).
    Ambiguous(Vec<String>),
    /// The first layer declaring an extension of the receiver and name declares it only for
    /// contexts the call does not have; holds those context types (`E4034`, D8 of
    /// `receiver-lambdas`).
    MissingContext(Vec<Type>),
}

/// How the receiver of an extension call is passed (D8 of `extension-methods`).
/// [`TypeChecker::apply_receiver_adjustment`] turns it into the call's first argument (D5 of
/// `typed-tree`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReceiverAdjustment {
    /// The number of `&`, `&mut` and `Heap<T>` layers looked through to reach the extension's
    /// receiver type.
    dereferences: usize,
    /// Whether the receiver is the payload of a nullable reached by a `?.` call.
    through_nullable: bool,
    pass: ReceiverPass,
    /// Whether the receiver is a temporary, which lives until the end of the statement, rather
    /// than a place.
    temporary: bool,
}

/// What a call does with the receiver reached by [`ReceiverAdjustment::dereferences`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiverPass {
    /// `fun`: a shared borrow, or a reborrow through the looked-through references.
    SharedBorrow,
    /// `mut fun`: a mutable borrow, or a reborrow of a `&mut`.
    MutableBorrow,
    /// `consuming fun`: the receiver is moved in (or copied, when it is `Copy`).
    Move,
    /// `consuming fun` through references or boxes: a `Copy` receiver is copied out.
    CopyOut,
    /// `consuming fun` through boxes only: the outermost box is moved and unboxed once per box
    /// layer (D7 of `heap-boxes`).
    Unbox,
}

/// A layer looked through to reach an extension's receiver type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Reference { mutable: bool },
    Heap,
}

/// A candidate of a position's context (D7 of `receiver-lambdas`): a binding, its type (a
/// borrow of the receiver or context, or an owned `consuming` receiver), the type it refers
/// to, and where it comes from, for diagnostics.
#[derive(Debug, Clone)]
struct ContextCandidate {
    local: prism_typed_tree::LocalId,
    ty: Type,
    referent: Type,
    source: ContextSource,
}

/// Where a context candidate comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContextSource {
    /// The `this` of the innermost lambda with a receiver.
    LambdaReceiver,
    /// The receiver of the enclosing extension.
    ExtensionReceiver,
    /// The context parameter of the enclosing function, by name.
    ContextParameter(String),
}

/// A function value with a receiver that a call passes a receiver to (D5 of
/// `receiver-lambdas`): whether its receiver is mutable, the layers looked through to reach the
/// receiver's type, and that type.
struct ReceiverValue {
    mutable: bool,
    layers: Vec<Layer>,
    reached: Type,
}

/// The receiver of an extension call, as [`TypeChecker::pass_receiver`] sees it.
struct ReceiverSite<'a> {
    expr: &'a Expr,
    /// The receiver's type, before any layer is looked through.
    ty: &'a Type,
    /// Whether the receiver is a place (or an optional place), rather than a temporary.
    is_place: bool,
    /// Whether the place itself may be written (a `var` binding, a mutable field path); a
    /// temporary is writable.
    writable: bool,
    /// The layers looked through to reach the extension's receiver type, outermost first.
    layers: &'a [Layer],
    /// Whether the extension is called on the payload of a nullable (`u?.greet()`).
    through_nullable: bool,
}

/// Identifies an unannotated private declaration whose type is inferred on demand.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum InferenceKey {
    /// A top-level function or `val`, by name.
    Declaration(String),
    /// An extension function, by receiver type constructor, name and context type.
    Extension {
        receiver: ReceiverKey,
        name: String,
        context: Option<Type>,
    },
}

/// Inference state of an unannotated private top-level declaration, whose type comes from its
/// body or initializer. Such a declaration is inferred on demand, the first time it is used, so
/// its type does not depend on declaration order (D5 of `typechecker-soundness`).
#[derive(Debug, Clone)]
enum Inference {
    /// Not inferred yet.
    Pending(Box<PendingDecl>),
    /// Being inferred: reaching it again means its type depends on itself.
    InProgress,
    /// Inferred; its symbol holds the type.
    Done,
}

/// A declaration waiting for on-demand inference.
#[derive(Debug, Clone)]
enum PendingDecl {
    Function(FunctionDecl),
    Var(TopLevelVarDecl),
}

/// The state of the body being checked, swapped out while another declaration is inferred on
/// demand.
struct BodyContext {
    locals: DetachedLocals,
    in_compose_stack: Vec<bool>,
    current_expected_return: Option<Type>,
    missing_implicit_parameter: Option<MissingImplicitParameter>,
    in_function: bool,
    origin: Origin,
    consuming: bool,
    /// The tree of the body being checked, with its open closures and loops.
    builder: BodyBuilder,
}

/// What [`TypeChecker::enter_closure`] saves and [`TypeChecker::exit_closure`] restores.
struct EnclosingClosureContext {
    in_function: bool,
    loops: Vec<StatementId>,
    expected_return: Option<Type>,
    missing_implicit_parameter: Option<MissingImplicitParameter>,
}

/// How the value of a `when` is used.
#[derive(Debug, Clone, Copy)]
enum WhenUse<'a> {
    /// The value is discarded: no `else` or common branch type is needed.
    Statement,
    /// The value is used, and its type is the join of the branch types.
    Infer,
    /// The value is used, and every branch is checked against the expected type.
    Check(&'a Type),
}

/// Static type checker and semantic analyzer for Prism source modules.
pub struct TypeChecker {
    /// Lexical scope environment.
    pub env: ScopeStack,
    /// Canonical package path of the current file (e.g. `Some("app.ui")`).
    pub current_package: Option<String>,
    /// Module path of the current file: its root-relative path without extension (`app/main`).
    /// Nominal types declared in this file are identified by it.
    pub module_path: String,
    /// Every nominal type known to this file, keyed by identity: the definitions of its own
    /// declarations and of the structs and enums its imports export, and the marker (`Copy`,
    /// `Clone` or none) of those and of every type an imported signature references, even from a
    /// module the file does not import. Handed to the typed tree (D4 of `mir-lowering`).
    types: TypeTable,
    /// Origin of the expected type currently being checked against.
    origin: Origin,
    /// Set just before checking an expression whose value is consumed (moved into a binding, an
    /// argument or a return), and taken by the next `infer_expr`/`check_expr` or `check_block`.
    /// See the `ownership` spec, "Consuming Positions".
    consuming: bool,
    /// Whether the expression currently being typed is consumed. Its pass-through children
    /// (`if`/`when` branches, a block tail, `?:` operands) are consumed too.
    parent_consuming: bool,
    /// Unannotated private top-level declarations and extensions, and how far their inference is.
    inference: HashMap<InferenceKey, Inference>,
    /// Declarations already reported as depending on themselves (`E4013`).
    recursive_inference: HashSet<InferenceKey>,
    /// The file's own extension functions, by receiver type and name.
    own_extensions: HashMap<(ReceiverKey, String), ExtensionsOfName>,
    /// The candidates of the current position's context, the first matching one passed to a
    /// function with a context parameter (D7 of `receiver-lambdas`): the innermost receiver
    /// lambda's `this`, or the enclosing function's receiver and context parameter.
    context_candidates: Vec<ContextCandidate>,
    /// Extension functions exported by direct imports, by receiver type and name, with one
    /// candidate per exporting module.
    imported_extensions: HashMap<(ReceiverKey, String), ExtensionsOfName>,
    /// The intrinsic extensions of the bundled library the file declares or imports, members of
    /// their receiver types (D7 of `standard-library`), by receiver type and name. The
    /// prelude's members are in [`Self::prelude`].
    members: HashMap<(ReceiverKey, String), Extension>,
    /// The plain intrinsic functions the file declares or imports, by defining module and name,
    /// with the built-in their calls lower to (D6 of `standard-library`).
    intrinsic_functions: HashMap<(String, String), BuiltinFunction>,
    /// The external functions the file declares or imports (D5 of `standard-library`).
    externals: Vec<DeclarationPath>,
    /// The prelude every workspace file sees without an import, consulted after the file's own
    /// declarations and its imports (D4 of `standard-library`); `None` for a library file.
    prelude: Option<std::sync::Arc<crate::prelude::PreludeScope>>,
    /// Module paths of the file's direct imports, whose ABIs this checker has ingested.
    direct_imports: HashSet<String>,
    /// Every struct and enum named by an ingested ABI, by bare name. Some are defined in modules
    /// the file does not import, which can be named in an `E4014` hint.
    seen_nominals: HashMap<String, Vec<NominalId>>,
    /// Every struct and enum an ingested ABI exports as a symbol. A definition an ABI records
    /// only because its exports reach the type (a private type of the import, or a type of a
    /// module it imports) is in the type table for layout only, and its members stay hidden
    /// (D4 of `codegen-aggregates`).
    exported_nominals: HashSet<NominalId>,
    /// While a parameter's default value is checked: the bindings it borrows (`this` and the
    /// earlier parameters), and the number of closures open around the default. Moving one of
    /// them outside a closure of the default is a move out of a borrow (D11 of `borrowck-moves`).
    default_inputs: Option<(Vec<prism_typed_tree::LocalId>, usize)>,
    /// Accumulated semantic errors.
    pub errors: Vec<TypeError>,
    /// The type parameters in scope: those of the generic declaration whose signature, fields or
    /// body is being registered or checked (D12 of `generics`).
    type_parameter_scope: Vec<TypeParameter>,
    /// Set while the callee of a call is inferred, where a generic function or variant
    /// constructor may be named; anywhere else it is a value whose type arguments nothing
    /// infers (`E4013`). Taken by the first name typed.
    generic_callee: bool,
    /// Ambient composable calling context stack (tracks `is_compose` across functions and closures).
    pub in_compose_stack: Vec<bool>,
    /// Whether a parameter's default value is being checked, for `E4008`'s note (D17 of
    /// `compose-transform`).
    in_default: bool,
    /// Whether the callee being inferred is a signature-typed intrinsic function in a call,
    /// which `symbol_node` then names instead of rejecting it as a value.
    intrinsic_call: bool,
    /// Expected return type of the currently enclosing function.
    pub current_expected_return: Option<Type>,
    /// Why the innermost enclosing closure has no implicit `it`, for the help of an unbound
    /// `it`; `None` when it has one (D6 of `implicit-it`).
    missing_implicit_parameter: Option<MissingImplicitParameter>,
    /// Private functions with omitted return types needing inference in Pass 2.
    pub unannotated_private_fns: Vec<String>,
    /// Whether the checker is currently evaluating within a function body.
    pub in_function: bool,
    /// The typed tree of the body being checked. Its open loops are the loops enclosing the
    /// current point of the current function or closure body; a `break` or `continue` with none
    /// reports `E4015`.
    builder: BodyBuilder,
    /// The bodies and items checked so far.
    file: FileBuilder,
    /// The enum that owns each module-level bare variant name (`Red` for `Color.Red`).
    variant_owners: HashMap<String, String>,
    /// Bare variant names declared by more than one enum. They are not bound at module level and
    /// must be qualified (`B.None`), except as a `when` pattern over one of those enums.
    ambiguous_variants: HashSet<String>,
    /// While a guard is checked: the bindings of its branch and their modes, which the guard may
    /// neither move nor mutate (D9 of `payload-patterns`).
    guard_bindings: Vec<(prism_typed_tree::LocalId, prism_typed_tree::BindingMode)>,
    /// The variables of `for` loops over lists borrowed shared, by the span of their
    /// declaration, with the iterable's source, so that a write through one suggests iterating
    /// `&mut` (D9 of `collections`).
    shared_loop_variables: HashMap<Span, String>,
}

impl TypeChecker {
    /// Creates a new `TypeChecker` for the file with module path `module_path` (its root-relative
    /// path without extension, such as `app/main`) in the given package.
    pub fn new(current_package: Option<String>, module_path: impl Into<String>) -> Self {
        Self {
            env: ScopeStack::new(),
            current_package,
            module_path: module_path.into(),
            types: TypeTable::with_builtins(),
            origin: Origin::Value,
            consuming: false,
            parent_consuming: false,
            inference: HashMap::new(),
            recursive_inference: HashSet::new(),
            own_extensions: HashMap::new(),
            context_candidates: Vec::new(),
            imported_extensions: HashMap::new(),
            members: HashMap::new(),
            intrinsic_functions: HashMap::new(),
            externals: Vec::new(),
            prelude: None,
            direct_imports: HashSet::new(),
            seen_nominals: HashMap::new(),
            exported_nominals: HashSet::new(),
            default_inputs: None,
            type_parameter_scope: Vec::new(),
            generic_callee: false,
            errors: Vec::new(),
            in_compose_stack: vec![false],
            in_default: false,
            intrinsic_call: false,
            current_expected_return: None,
            missing_implicit_parameter: Some(MissingImplicitParameter::OutsideClosures),
            unannotated_private_fns: Vec::new(),
            in_function: false,
            builder: BodyBuilder::new(DeclarationPath::new("", "<none>")),
            file: FileBuilder::default(),
            variant_owners: HashMap::new(),
            ambiguous_variants: HashSet::new(),
            guard_bindings: Vec::new(),
            shared_loop_variables: HashMap::new(),
        }
    }

    /// The checker, with `prelude` in scope after the file's own declarations and its imports
    /// (D4 of `standard-library`). Installing it costs a pointer copy, plus the definitions of
    /// the types the prelude declares, of which today's prelude has none.
    pub fn with_prelude(mut self, prelude: std::sync::Arc<crate::prelude::PreludeScope>) -> Self {
        self.env.set_prelude(prelude.names());
        prelude.install_types(&mut self.types);
        for module in prelude.modules() {
            self.direct_imports.insert(module.clone());
        }
        for (name, ids) in prelude.seen_nominals() {
            self.seen_nominals
                .entry(name.clone())
                .or_default()
                .extend(ids.iter().cloned());
        }
        self.exported_nominals
            .extend(prelude.exported_nominals().iter().cloned());
        self.prelude = Some(prelude);
        self
    }

    /// The parts of this checker's ingested imports that [`crate::prelude::PreludeScope`]
    /// keeps: the import layer's names, the extensions, the members, the plain intrinsics, the
    /// externals and the type table.
    #[allow(clippy::type_complexity)]
    pub(crate) fn into_prelude_parts(
        self,
    ) -> (
        HashMap<String, Symbol>,
        HashMap<(ReceiverKey, String), ExtensionsOfName>,
        HashMap<(ReceiverKey, String), Extension>,
        HashMap<(String, String), BuiltinFunction>,
        Vec<DeclarationPath>,
        TypeTable,
        HashMap<String, Vec<NominalId>>,
        HashSet<NominalId>,
        HashSet<String>,
    ) {
        let names = self.env.imported_names();
        let extensions = self.imported_extensions;
        (
            names,
            extensions,
            self.members,
            self.intrinsic_functions,
            self.externals,
            self.types,
            self.seen_nominals,
            self.exported_nominals,
            self.direct_imports,
        )
    }

    /// The intrinsic member named `name` of the type constructor of `receiver`, declared or
    /// imported by the file, else of the prelude (D7 of `standard-library`).
    fn member(&self, receiver: &Type, name: &str) -> Option<&Extension> {
        let key = (ReceiverKey::of(receiver), name.to_string());
        self.members
            .get(&key)
            .or_else(|| self.prelude.as_ref()?.member(&key))
    }

    /// Whether `receiver` has an intrinsic property named `name` (`length` of `String`, `size`
    /// and `indices` of `List<T>`), which reads like a field.
    fn has_intrinsic_property(&self, receiver: &Type, name: &str) -> bool {
        self.member(receiver, name)
            .is_some_and(|member| member.property)
    }

    /// Whether `receiver` has an intrinsic member function named `name`.
    fn has_intrinsic_function(&self, receiver: &Type, name: &str) -> bool {
        self.member(receiver, name)
            .is_some_and(|member| !member.property)
    }

    /// The built-in a call of the plain function `module::name` lowers to, when that function
    /// is an intrinsic the file declares or imports, or one of the prelude.
    fn intrinsic_function(&self, module: &str, name: &str) -> Option<BuiltinFunction> {
        let key = (module.to_string(), name.to_string());
        self.intrinsic_functions
            .get(&key)
            .copied()
            .or_else(|| self.prelude.as_ref()?.intrinsic_function(&key))
    }

    /// Whether the plain function `path` is an external function the file declares or imports.
    fn is_external(&self, path: &DeclarationPath) -> bool {
        self.externals.contains(path)
            || self
                .prelude
                .as_ref()
                .is_some_and(|prelude| prelude.externals().contains(path))
    }

    /// The built-in a bare callee names when it resolves to an intrinsic function: the file's
    /// own, an import's or the prelude's, found as any name is (D4 of `standard-library`), so a
    /// local, parameter, declaration or import of the name shadows the prelude's.
    fn intrinsic_callee(&self, callee: &Expr) -> Option<BuiltinFunction> {
        let ExprKind::Ident(name) = &callee.kind else {
            return None;
        };
        match &self.env.lookup(name)?.kind {
            SymbolKind::Function { module, .. } => self.intrinsic_function(module, name),
            _ => None,
        }
    }

    /// Returns `true` if the current evaluation context is composable (`compose fun` or composable closure).
    pub fn is_in_compose(&self) -> bool {
        *self.in_compose_stack.last().unwrap_or(&false)
    }

    /// Pushes a composable context flag when entering a function or closure.
    pub fn push_compose_context(&mut self, is_compose: bool) {
        self.in_compose_stack.push(is_compose);
    }

    /// Pops the composable context flag when exiting a function or closure.
    pub fn pop_compose_context(&mut self) {
        if self.in_compose_stack.len() > 1 {
            self.in_compose_stack.pop();
        }
    }

    /// Reports `E4008` for a call of the composable `callee` outside a composable context,
    /// noting when the call is in a default value (D17 of `compose-transform`).
    fn report_compose_context(&mut self, callee: String, span: Span) {
        self.errors.push(TypeError::InvalidComposeContext {
            callee,
            span,
            in_default: self.in_default,
        });
    }

    /// Reports `E4030` for a call of the composition operation `operation` in a composable
    /// context (D17 of `compose-transform`). A closure that is not composable, a handler
    /// written in a composable, is no composable context, so it may start compositions.
    fn reject_composition_in_composable(&mut self, builtin: BuiltinFunction, span: Span) {
        let starts_or_drives = matches!(
            builtin,
            BuiltinFunction::Composition | BuiltinFunction::SetContent | BuiltinFunction::Recompose
        );
        if starts_or_drives && self.is_in_compose() {
            self.errors.push(TypeError::CompositionInComposable {
                operation: builtin.name().to_string(),
                span,
            });
        }
    }

    /// The type of node `id` of the body being built.
    pub fn type_of(&self, id: ExpressionId) -> &Type {
        self.builder.type_of(id)
    }

    fn ty(&self, id: ExpressionId) -> Type {
        self.builder.type_of(id).clone()
    }

    fn add(&mut self, kind: ExpressionKind, ty: Type, span: Span) -> ExpressionId {
        self.builder.add(kind, ty, span)
    }

    /// An error node keeping `children`, the nodes checked before checking gave up (D12).
    fn error_node(&mut self, children: Vec<ExpressionId>, span: Span) -> ExpressionId {
        self.add(ExpressionKind::Error { children }, Type::Error, span)
    }

    /// An error node around `id`, for a value that does not fit where it is used.
    fn wrap_error(&mut self, id: ExpressionId) -> ExpressionId {
        let span = self.builder.span_of(id);
        self.error_node(vec![id], span)
    }

    /// `id` as a place: itself when it is one, or a temporary holding its value (D5). The payload
    /// of a nullable value is a place once the nullable is held in a temporary.
    fn as_place(&mut self, id: ExpressionId) -> ExpressionId {
        match self.builder.kind(id).clone() {
            ExpressionKind::NonNullPayload { nullable } => {
                let place = self.as_place(nullable);
                if place != nullable {
                    self.builder.expression_mut(id).kind =
                        ExpressionKind::NonNullPayload { nullable: place };
                }
                id
            }
            kind if kind.is_place() => id,
            _ => {
                let (ty, span) = (self.ty(id), self.builder.span_of(id));
                self.add(ExpressionKind::Temporary { value: id }, ty, span)
            }
        }
    }

    /// The referent of `id`, a `&T`, `&mut T` or `Heap<T>`.
    fn deref(&mut self, id: ExpressionId) -> ExpressionId {
        let referent = match self.builder.type_of(id) {
            Type::Reference { inner, .. } | Type::Heap(inner) => inner.as_ref().clone(),
            other => other.clone(),
        };
        let span = self.builder.span_of(id);
        self.add(ExpressionKind::Deref { pointer: id }, referent, span)
    }

    /// Dereferences every `&` and `&mut` layer of `id`, as operators read through references.
    fn peel_reference_nodes(&mut self, mut id: ExpressionId) -> ExpressionId {
        while matches!(self.builder.type_of(id), Type::Reference { .. }) {
            id = self.deref(id);
        }
        id
    }

    /// Dereferences every `&`, `&mut` and `Heap<T>` layer of `id`, as field access does.
    fn peel_indirection_nodes(&mut self, mut id: ExpressionId) -> ExpressionId {
        while matches!(
            self.builder.type_of(id),
            Type::Reference { .. } | Type::Heap(_)
        ) {
            id = self.deref(id);
        }
        id
    }

    /// Dereferences `&`, `&mut` and `Heap<T>` layers of `id` until its type is `target`.
    fn deref_until(&mut self, mut id: ExpressionId, target: &Type) -> ExpressionId {
        while self.builder.type_of(id) != target
            && matches!(
                self.builder.type_of(id),
                Type::Reference { .. } | Type::Heap(_)
            )
        {
            id = self.deref(id);
        }
        id
    }

    /// The payload of `id` reached through its references, when it is a nullable (D7): the
    /// projection that leaves the enclosing chain with `null`.
    fn non_null(&mut self, id: ExpressionId) -> ExpressionId {
        let id = self.peel_reference_nodes(id);
        match self.builder.type_of(id) {
            Type::Nullable(inner) => {
                let (inner, span) = (inner.as_ref().clone(), self.builder.span_of(id));
                self.add(ExpressionKind::NonNullPayload { nullable: id }, inner, span)
            }
            _ => id,
        }
    }

    /// Opens a safe navigation over `id`: returns the written chain `id` is, which the new
    /// navigation joins (D7), and the payload the navigation reads.
    fn open_chain(&mut self, id: ExpressionId) -> (Option<ExpressionId>, ExpressionId) {
        match self.builder.kind(id) {
            ExpressionKind::SafeNavigation {
                chain,
                origin: SafeNavigationOrigin::Written,
            } => {
                let chain = chain.to_owned();
                (Some(id), self.non_null(chain))
            }
            _ => (None, self.non_null(id)),
        }
    }

    /// Closes a safe navigation whose body is `body`: re-points the joined chain `joined` at
    /// it, or makes a new chain.
    fn close_chain(
        &mut self,
        joined: Option<ExpressionId>,
        body: ExpressionId,
        origin: SafeNavigationOrigin,
        span: Span,
    ) -> ExpressionId {
        let ty = self.ty(body).nullable();
        let kind = ExpressionKind::SafeNavigation {
            chain: body,
            origin,
        };
        match joined {
            Some(chain) => {
                self.builder.replace(chain, kind, ty, span);
                chain
            }
            None => self.add(kind, ty, span),
        }
    }

    /// Wraps the consumed node `id` in a move, or in a copy when its type is `Copy`, when it is a
    /// place (D5); a consumed safe navigation moves or copies its final place inside the chain.
    /// Any other value is fresh, so consuming it needs no node.
    fn consume(&mut self, id: ExpressionId) -> ExpressionId {
        match self.builder.kind(id).clone() {
            ExpressionKind::SafeNavigation { chain, origin } => {
                let inner = self.consume(chain);
                if inner != chain {
                    self.builder.expression_mut(id).kind = ExpressionKind::SafeNavigation {
                        chain: inner,
                        origin,
                    };
                }
                id
            }
            ExpressionKind::Error { .. } => id,
            kind if kind.is_place() => {
                let place = self.as_place(id);
                let (ty, span) = (self.ty(place), self.builder.span_of(place));
                let kind = if self.is_copy(&ty) {
                    ExpressionKind::Copy { place }
                } else {
                    ExpressionKind::Move { place }
                };
                self.add(kind, ty, span)
            }
            _ => id,
        }
    }

    /// Wraps `id` in a coercion to `expected` when it fits only by assignability (D6), so its
    /// type is the one its position expects.
    fn coerce_to(&mut self, id: ExpressionId, expected: &Type) -> ExpressionId {
        let found = self.ty(id);
        if &found == expected
            || found == Type::Error
            || expected == &Type::Error
            || !found.is_assignable_to(expected)
        {
            return id;
        }
        let span = self.builder.span_of(id);
        if found == Type::Nothing {
            let kind = ExpressionKind::Coerce {
                value: id,
                kind: CoercionKind::Diverge,
            };
            return self.add(kind, expected.clone(), span);
        }
        match expected {
            Type::Nullable(inner) if !matches!(found, Type::Nullable(_)) => {
                let value = self.coerce_to(id, inner);
                let kind = ExpressionKind::Coerce {
                    value,
                    kind: CoercionKind::WrapNullable,
                };
                self.add(kind, expected.clone(), span)
            }
            _ => {
                // A join of branches that would change a function value's representation (D8
                // of `closures`); `fit` reports the other sites with their own codes.
                if !self.converts_function_value(id, &found, expected) {
                    self.errors.push(TypeError::TypeMismatch {
                        expected: expected.clone(),
                        found,
                        span,
                        help: None,
                    });
                    return self.wrap_error(id);
                }
                let kind = ExpressionKind::Coerce {
                    value: id,
                    kind: CoercionKind::Subtype,
                };
                self.add(kind, expected.clone(), span)
            }
        }
    }

    /// Whether the node `id`, of the function type (or nullable of one) `found`, may be used as
    /// an `expected` (D8 of `closures`). Any value converts in place. A named function or
    /// variant constructor, which MIR lowers as an adapter closure, converts its own parameters
    /// and result through any step, except those of function type, which the adapter would
    /// have to convert as values. Values of other types are not this rule's.
    fn converts_function_value(&self, id: ExpressionId, found: &Type, expected: &Type) -> bool {
        // Wrapping a value into a nullable keeps the value as it is, as the payload.
        let expected = match (found, expected) {
            (Type::Fn { .. }, Type::Nullable(inner)) => inner.as_ref(),
            _ => expected,
        };
        if !found.is_function_value() || found.converts_in_place(expected) {
            return true;
        }
        if !matches!(
            self.builder.kind(id),
            ExpressionKind::FunctionReference(_) | ExpressionKind::VariantConstructor { .. }
        ) {
            return false;
        }
        let (
            Type::Fn { params, ret, .. },
            Type::Fn {
                params: expected_params,
                ret: expected_ret,
                ..
            },
        ) = (found, expected)
        else {
            return true;
        };
        let adapts = |from: &Type, to: &Type| {
            !(from.is_function_value() || to.is_function_value()) || from.converts_in_place(to)
        };
        expected_params
            .iter()
            .zip(params)
            .all(|(from, to)| adapts(from, to))
            && adapts(ret, expected_ret)
    }

    /// The definition of the nominal type `id`, if this file knows it.
    pub fn type_def(&self, id: &NominalId) -> Option<&TypeDef> {
        self.types.definition(id)
    }

    /// Whether values of `ty` are duplicated implicitly by a bitwise copy (the `ownership` spec,
    /// "Built-In Copy Types"). The error type counts as `Copy`, so an earlier error is not
    /// reported again as a move.
    pub fn is_copy(&self, ty: &Type) -> bool {
        self.types.is_copy(ty)
    }

    /// Whether values of `ty` can be duplicated by an explicit `.clone()` (the `ownership` spec,
    /// "Built-In Clone Types"). Every `Copy` type is `Clone`.
    pub fn is_clone(&self, ty: &Type) -> bool {
        self.types.is_clone(ty)
    }

    /// Checks every `: Copy` and `: Clone` declaration of the file structurally: each field or
    /// variant payload must itself be `Copy` (or `Clone`), else `E4016` is reported once per
    /// offending member. Markers are shallow, so this needs no recursion or cycle guard.
    fn check_marker_declarations(&mut self, source_file: &SourceFile) {
        for decl in &source_file.declarations {
            let (marker, type_name, type_parameters, members) = match decl {
                Declaration::Struct(s) => (
                    s.marker,
                    &s.name,
                    &s.type_parameters,
                    s.fields
                        .iter()
                        .map(|f| (format!("field '{}'", f.name), &f.ty, f.span))
                        .collect::<Vec<_>>(),
                ),
                Declaration::Enum(e) => (
                    e.marker,
                    &e.name,
                    &e.type_parameters,
                    e.variants
                        .iter()
                        .flat_map(|v| {
                            v.fields.iter().enumerate().map(|(i, ty)| {
                                (
                                    format!("payload {} of variant '{}'", i + 1, v.name),
                                    ty,
                                    ty.span,
                                )
                            })
                        })
                        .collect(),
                ),
                _ => continue,
            };
            let Some(marker) = marker else { continue };
            // Each marker parameter counts as satisfying the marker (D6 of `generics`): it is
            // lowered as a type parameter bounded by it.
            let member_types: Vec<&prism_ast::types::Type> =
                members.iter().map(|(_, ty, _)| *ty).collect();
            let marker_parameters = prism_abi::marker_parameters(type_parameters, &member_types);
            let mut parameters = type_parameters_of(type_parameters);
            for parameter in &mut parameters {
                if marker_parameters.contains(&parameter.index) {
                    parameter.bound = Some(marker);
                }
            }
            let member_types: Vec<Type> = self.with_type_parameters(parameters, |c| {
                member_types.iter().map(|ty| c.lower_type(ty)).collect()
            });
            for ((member, _, span), member_type) in members.into_iter().zip(member_types) {
                let accepted = match marker {
                    TypeMarker::Copy => self.is_copy(&member_type),
                    TypeMarker::Clone => self.is_clone(&member_type),
                };
                if !accepted {
                    let member_is_clone = self.is_clone(&member_type);
                    self.errors.push(TypeError::InvalidMarkerDeclaration {
                        marker,
                        type_name: type_name.clone(),
                        member,
                        member_type,
                        member_is_clone,
                        span,
                    });
                }
            }
        }
    }

    /// The fields of struct `id`, if its definition is known and its members are visible.
    fn struct_fields(&self, id: &NominalId) -> Option<Vec<(String, bool, Type)>> {
        if self.is_layout_only(id) {
            return None;
        }
        self.types.struct_fields(id).map(<[_]>::to_vec)
    }

    /// The variants of enum `id`, if its definition is known and its members are visible.
    fn enum_variants(&self, id: &NominalId) -> Option<Vec<(String, Vec<Type>)>> {
        if self.is_layout_only(id) {
            return None;
        }
        self.types.enum_variants(id).map(<[_]>::to_vec)
    }

    /// Whether the definition of struct or enum `id` is known and its members are visible.
    fn members_visible(&self, id: &NominalId) -> bool {
        !self.is_layout_only(id) && self.types.definition(id).is_some()
    }

    /// The position of the variant `name` of enum `id`, if its definition is known, its members
    /// are visible and it has one.
    fn variant_index(&self, id: &NominalId, name: &str) -> Option<usize> {
        if self.is_layout_only(id) {
            return None;
        }
        self.types.variant_index(id, name)
    }

    /// Whether the type checker knows the definition of `id` for layout only: the type is
    /// neither the file's own, built in, nor exported by an import, so it reached the file's
    /// type table through an import's reachable type definitions (D4 of `codegen-aggregates`).
    /// Its members are hidden, as if its definition were unknown.
    fn is_layout_only(&self, id: &NominalId) -> bool {
        id.module != self.module_path
            && id.module != BUILTIN_MODULE
            && !self.exported_nominals.contains(id)
    }

    /// Binds a type name in the current scope: a declared or imported struct or enum gives its
    /// identity, and `Range` with no user declaration gives the built-in range type.
    /// An ambiguous imported type name gives the error type (`E4012` is reported by
    /// [`Self::check_annotation_names`]).
    fn bind_type_name(&self, name: &str) -> Option<Type> {
        if let Some(sym) = self.env.lookup_type(name) {
            return Some(sym.ty());
        }
        match self.env.lookup_import(name, is_type_symbol) {
            ImportLookup::Ambiguous(_) => Some(Type::Error),
            _ if name == "Range" => Some(Type::plain_struct(NominalId::builtin_range())),
            // Reported as `E4014` by `check_annotation_names`; `Error` avoids a follow-on `E4006`.
            _ if self.unimported_type_named(name).is_some() => Some(Type::Error),
            _ => None,
        }
    }

    /// Lowers an annotation to a semantic type, binding each name where the annotation appears:
    /// first to a type parameter in scope, then to a declared or imported struct or enum, with
    /// its type arguments (D12 of `generics`). Names with no declaration stay opaque. A type
    /// written with the wrong number of type arguments is the error type; that and the other
    /// annotation errors are reported once, by [`Self::check_annotation_names`].
    pub fn lower_type(&self, ast_ty: &prism_ast::types::Type) -> Type {
        use prism_ast::types::TypeKind;
        match &ast_ty.kind {
            TypeKind::Named { name, type_args } => {
                if let Some(parameter) = self.scoped_type_parameter(name) {
                    return if type_args.is_empty() {
                        parameter.as_type()
                    } else {
                        Type::Error
                    };
                }
                match name.as_str() {
                    "Int" => return Type::Int,
                    "Float" | "Double" => return Type::Float,
                    "Bool" => return Type::Bool,
                    "Char" => return Type::Char,
                    "String" => return Type::String,
                    "Unit" => return Type::Unit,
                    "Nothing" => return Type::Nothing,
                    "Heap" => {
                        return match type_args.as_slice() {
                            [inner] => Type::Heap(Box::new(self.lower_type(inner))),
                            _ => Type::Error,
                        }
                    }
                    LIST => {
                        return match type_args.as_slice() {
                            [inner] => Type::List(Box::new(self.lower_type(inner))),
                            _ => Type::Error,
                        }
                    }
                    _ => {}
                }
                let arguments: Vec<Type> = type_args.iter().map(|a| self.lower_type(a)).collect();
                match self.bind_type_name(name) {
                    Some(Type::Struct { id, .. }) => {
                        if self.types.parameters(&id).len() != arguments.len() {
                            return Type::Error;
                        }
                        Type::Struct { id, arguments }
                    }
                    Some(Type::Enum { id, .. }) => {
                        if self.types.parameters(&id).len() != arguments.len() {
                            return Type::Error;
                        }
                        Type::Enum { id, arguments }
                    }
                    Some(other) => other,
                    None => Type::Opaque(name.clone()),
                }
            }
            TypeKind::Nullable(inner) => Type::Nullable(Box::new(self.lower_type(inner))),
            TypeKind::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.lower_type(inner)),
            },
            TypeKind::Unit => Type::Unit,
            TypeKind::Function {
                is_compose,
                receiver,
                params,
                escaping,
                return_type,
            } => {
                // The receiver is the first parameter, a borrow of its type.
                let mut lowered = Vec::with_capacity(params.len() + 1);
                let mut flags = Vec::with_capacity(params.len() + 1);
                if let Some(receiver) = receiver {
                    lowered.push(Type::Reference {
                        mutable: receiver.mutable,
                        inner: Box::new(self.lower_type(&receiver.ty)),
                    });
                    flags.push(false);
                }
                lowered.extend(params.iter().map(|p| self.lower_type(p)));
                flags.extend(escaping.iter().copied());
                Type::Fn {
                    is_compose: *is_compose,
                    receiver: receiver.is_some(),
                    params: lowered,
                    escaping: flags,
                    ret: Box::new(self.lower_type(return_type)),
                }
            }
        }
    }

    /// The type parameter named `name` in scope, if any.
    fn scoped_type_parameter(&self, name: &str) -> Option<&TypeParameter> {
        self.type_parameter_scope.iter().find(|p| p.name == name)
    }

    /// Runs `f` with `parameters` as the type parameters in scope, restoring the outer scope
    /// afterwards. Generic declarations do not nest, so the scope is replaced, not extended.
    fn with_type_parameters<T>(
        &mut self,
        parameters: Vec<TypeParameter>,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let outer = std::mem::replace(&mut self.type_parameter_scope, parameters);
        let result = f(self);
        self.type_parameter_scope = outer;
        result
    }

    /// Reports a duplicate type parameter of one declaration, and one named like a built-in
    /// type the lowering fixes, with `E4010` (D12 of `generics`).
    fn check_type_parameter_names(&mut self, parameters: &[prism_ast::TypeParameter]) {
        const BUILT_IN: &[&str] = &[
            "Int", "Float", "Double", "Bool", "Char", "String", "Unit", "Nothing", "Heap", "List",
        ];
        let mut seen = HashSet::new();
        for parameter in parameters {
            if BUILT_IN.contains(&parameter.name.as_str()) {
                self.errors.push(TypeError::InvalidTypeParameterName {
                    message: format!(
                        "type parameter '{}' would hide the built-in type '{}'; choose another name",
                        parameter.name, parameter.name
                    ),
                    span: parameter.span,
                });
            } else if !seen.insert(parameter.name.as_str()) {
                self.errors.push(TypeError::InvalidTypeParameterName {
                    message: format!(
                        "type parameter '{}' is declared twice in one list",
                        parameter.name
                    ),
                    span: parameter.span,
                });
            }
        }
    }

    /// Resolves an identifier, reporting `E4001` when it is unknown and `E4007` when it names a
    /// `package` symbol of another package.
    fn resolve_ident(&mut self, name: &str, span: Span) -> Option<Symbol> {
        match self.env.resolve(name, span) {
            Ok(sym) => {
                let sym = sym.clone();
                self.check_visibility(&sym, span);
                Some(sym)
            }
            Err(TypeError::UnresolvedSymbol { .. }) if name == HEAP || name == LIST => {
                // The box constructor can only be called, and `List` is a type.
                self.errors.push(TypeError::BuiltinFunctionAsValue {
                    name: name.to_string(),
                    span,
                });
                None
            }
            Err(TypeError::UnresolvedSymbol { .. }) if name == IMPLICIT_PARAMETER => {
                // No closure around this use binds `it`: say why the innermost one has none (D6
                // of `implicit-it`).
                let missing = match self
                    .missing_implicit_parameter
                    .clone()
                    .unwrap_or(MissingImplicitParameter::OutsideClosures)
                {
                    // Name the closure's type, from the return type and compose flag its body is
                    // checked with, rather than keep it for every closure.
                    MissingImplicitParameter::NoExpectedParameters {
                        receiver,
                        function_type: None,
                    } => {
                        let function_type = self.current_expected_return.clone().map(|ret| {
                            let params: Vec<Type> = receiver.iter().cloned().collect();
                            Type::Fn {
                                is_compose: self.is_in_compose(),
                                receiver: receiver.is_some(),
                                escaping: vec![false; params.len()],
                                params,
                                ret: Box::new(ret),
                            }
                        });
                        MissingImplicitParameter::NoExpectedParameters {
                            receiver,
                            function_type,
                        }
                    }
                    other => other,
                };
                self.errors
                    .push(TypeError::UnresolvedImplicitParameter { missing, span });
                None
            }
            Err(TypeError::UnresolvedSymbol { .. }) if self.outer_receiver_with(name).is_some() => {
                // Only the innermost receiver is implicit (D4 of `receiver-lambdas`): say which
                // outer one has the name, computed on this error path only.
                let (receiver, field) = self.outer_receiver_with(name).expect("checked above");
                self.errors.push(TypeError::OuterReceiverName {
                    name: name.to_string(),
                    receiver,
                    field,
                    span,
                });
                None
            }
            Err(TypeError::UnresolvedSymbol { .. })
                if self.unimported_type_named(name).is_some() =>
            {
                // The name is a type that reached this file through an import's signature: to
                // construct it or name its variants, the file must import its module.
                let id = self.unimported_type_named(name).expect("checked above");
                self.errors.push(TypeError::MemberOfUnimportedType {
                    type_name: id.name,
                    module: id.module,
                    span,
                });
                None
            }
            Err(err) => {
                self.errors.push(err);
                None
            }
        }
    }

    /// The first receiver of an enclosing extension or lambda, hidden by an inner lambda with a
    /// receiver, that has a field (`true`) or an extension (`false`) named `name`.
    fn outer_receiver_with(&self, name: &str) -> Option<(Type, bool)> {
        self.env.shadowed(THIS).find_map(|symbol| {
            let SymbolKind::Variable { ty, .. } = &symbol.kind else {
                return None;
            };
            let receiver = peel_references(ty).clone();
            if self.has_field(&receiver, name) {
                return Some((receiver, true));
            }
            let extension = self.has_intrinsic_function(&receiver, name)
                || !matches!(self.find_extension(&receiver, name), ExtensionLookup::None);
            extension.then_some((receiver, false))
        })
    }

    /// Records every struct and enum `ty` names, so `E4014` can name types that reached this
    /// file through an import's signature.
    fn record_nominals(&mut self, ty: &AbiType) {
        match ty {
            AbiType::Nominal {
                module,
                name,
                args,
                marker,
                marker_parameters,
                ..
            } => {
                let id = NominalId::new(module.clone(), name.clone());
                self.types
                    .insert_marker(id.clone(), *marker, marker_parameters.clone());
                let ids = self.seen_nominals.entry(name.clone()).or_default();
                if !ids.contains(&id) {
                    ids.push(id);
                }
                args.iter().for_each(|a| self.record_nominals(a));
            }
            AbiType::Unbound { args, .. } => args.iter().for_each(|a| self.record_nominals(a)),
            AbiType::Nullable(inner)
            | AbiType::Heap(inner)
            | AbiType::List(inner)
            | AbiType::Ref { inner, .. } => self.record_nominals(inner),
            AbiType::Fn { params, ret, .. } => {
                params.iter().for_each(|p| self.record_nominals(p));
                self.record_nominals(ret);
            }
            AbiType::Prim(_) | AbiType::Unit | AbiType::Range | AbiType::Parameter { .. } => {}
        }
    }

    /// Whether the module defining `id` is out of this file's reach: neither the file itself,
    /// the built-in module, nor a direct import (D9 of `typechecker-soundness`).
    fn is_unimported(&self, id: &NominalId) -> bool {
        id.module != self.module_path
            && id.module != BUILTIN_MODULE
            && !self.direct_imports.contains(&id.module)
    }

    /// The single type named `name` that reached this file through an import's signature but is
    /// defined in a module the file does not import.
    fn unimported_type_named(&self, name: &str) -> Option<NominalId> {
        let ids: Vec<&NominalId> = self
            .seen_nominals
            .get(name)?
            .iter()
            .filter(|id| self.is_unimported(id))
            .collect();
        match ids.as_slice() {
            [id] => Some((*id).clone()),
            _ => None,
        }
    }

    /// Reports `E4014` and returns `true` if `ty` is a struct or enum whose defining module is
    /// not a direct import, so its members cannot be used.
    fn reject_unimported_members(&mut self, ty: &Type, span: Span) -> bool {
        match ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } if self.is_unimported(id) => {
                self.errors.push(TypeError::MemberOfUnimportedType {
                    type_name: id.name.clone(),
                    module: id.module.clone(),
                    span,
                });
                true
            }
            _ => false,
        }
    }

    /// Reports `E4007` if `sym` has `package` visibility and is defined in another package
    /// (single-file compilation unit invariant). Returns whether the symbol is visible.
    fn check_visibility(&mut self, sym: &Symbol, span: Span) -> bool {
        if sym.visibility() == Visibility::Package {
            let def_pkg = sym.defining_package().unwrap_or("");
            let curr_pkg = self.current_package.as_deref().unwrap_or("");
            if def_pkg != curr_pkg {
                self.errors.push(TypeError::VisibilityViolation {
                    symbol: sym.name.clone(),
                    defining_package: def_pkg.to_string(),
                    accessing_package: curr_pkg.to_string(),
                    span,
                });
                return false;
            }
        }
        true
    }

    /// Reports the errors of an annotation, which [`Self::lower_type`] lowers silently: `E4012`
    /// for every nominal type name that several imports export, `E4007` for every one that has
    /// `package` visibility in another package, `E4014` for one only an import's signature
    /// names, `E4020` for a generic type, `Heap` or type parameter written with the wrong number
    /// of type arguments, and `E4022` for a type argument holding a borrow (D7 of `generics`).
    fn check_annotation_names(&mut self, ast_ty: &prism_ast::types::Type) {
        use prism_ast::types::TypeKind;
        match &ast_ty.kind {
            TypeKind::Named { name, type_args } => {
                let expected_arguments = if self.scoped_type_parameter(name).is_some() {
                    Some((format!("type parameter '{}'", name), 0))
                } else if name == "Heap" {
                    Some(("'Heap'".to_string(), 1))
                } else if name == LIST {
                    Some(("'List'".to_string(), 1))
                } else if is_primitive_name(name) {
                    None
                } else {
                    if let Some(sym) = self.env.lookup_type(name).cloned() {
                        self.check_visibility(&sym, ast_ty.span);
                    } else if let ImportLookup::Ambiguous(modules) =
                        self.env.lookup_import(name, is_type_symbol)
                    {
                        self.errors.push(TypeError::AmbiguousImport {
                            name: name.clone(),
                            modules,
                            span: ast_ty.span,
                        });
                    } else if let Some(id) = self.unimported_type_named(name) {
                        // The name is a type this file only knows through an import's
                        // signature: naming it requires importing its module, like using its
                        // members.
                        self.errors.push(TypeError::MemberOfUnimportedType {
                            type_name: id.name,
                            module: id.module,
                            span: ast_ty.span,
                        });
                    }
                    match self.bind_type_name(name) {
                        Some(Type::Struct { id, .. }) | Some(Type::Enum { id, .. }) => {
                            Some((format!("type '{}'", name), self.types.parameters(&id).len()))
                        }
                        _ => None,
                    }
                };
                if let Some((owner, expected)) = expected_arguments {
                    if expected != type_args.len() {
                        self.errors.push(TypeError::WrongTypeArgumentCount {
                            owner,
                            expected,
                            found: type_args.len(),
                            span: ast_ty.span,
                        });
                    } else {
                        for arg in type_args {
                            self.check_borrow_free_annotation(arg, name);
                        }
                    }
                }
                for arg in type_args {
                    self.check_annotation_names(arg);
                }
            }
            TypeKind::Nullable(inner) | TypeKind::Reference { inner, .. } => {
                self.check_annotation_names(inner)
            }
            TypeKind::Function {
                receiver,
                params,
                return_type,
                ..
            } => {
                if let Some(receiver) = receiver {
                    self.check_annotation_names(&receiver.ty);
                }
                for param in params {
                    self.check_annotation_names(param);
                }
                self.check_annotation_names(return_type);
            }
            TypeKind::Unit => {}
        }
    }

    /// Reports `E4022` when the written type argument `arg` of `owner` holds a borrow outside a
    /// function type.
    fn check_borrow_free_annotation(&mut self, arg: &prism_ast::types::Type, owner: &str) {
        let lowered = self.lower_type(arg);
        if lowered.reference_outside_functions().is_some() {
            self.errors.push(TypeError::BorrowInTypeArgument {
                argument: lowered,
                owner: format!("'{}'", owner),
                span: arg.span,
            });
        }
    }

    /// Returns the struct or enum symbol `expr` names, if `expr` is an identifier whose innermost
    /// binding is a type declaration rather than a value.
    fn type_name_symbol(&self, expr: &Expr) -> Option<Symbol> {
        if let ExprKind::Ident(name) = &expr.kind {
            if let Some(sym) = self.env.lookup(name) {
                if matches!(
                    sym.kind,
                    SymbolKind::Struct { .. } | SymbolKind::Enum { .. }
                ) {
                    return Some(sym.clone());
                }
            }
        }
        None
    }

    /// Types `member` on `target` when `target` names a type: an enum name gives access to its
    /// variants, while a struct name has no members (`E4006`). Returns `None` when `target` is
    /// not a type name.
    fn type_name_member(
        &mut self,
        target: &Expr,
        member: &str,
        span: Span,
    ) -> Option<(ExpressionId, Option<bool>)> {
        let sym = self.type_name_symbol(target)?;
        if !self.check_visibility(&sym, target.span) {
            return Some((self.error_node(Vec::new(), span), None));
        }
        let generic_callee = std::mem::take(&mut self.generic_callee);
        let ty = sym.ty();
        if let SymbolKind::Struct { .. } = sym.kind {
            self.errors.push(TypeError::UnknownField {
                target_type: ty,
                field_name: member.to_string(),
                span,
                help: None,
            });
            return Some((self.error_node(Vec::new(), span), None));
        }
        let (member_ty, is_var) = self.member_type(&ty, member, false, span);
        // A variant of a generic enum needs its type arguments: a call infers them for a
        // payload variant, and an expected type for a payload-free one (D8 of `generics`).
        if member_ty.mentions_parameter() {
            let qualified = format!("{}.{}", sym.name, member);
            if !matches!(member_ty, Type::Fn { .. }) {
                return Some((self.uninferred_unit_variant(&qualified, span), is_var));
            }
            if !generic_callee {
                self.errors.push(TypeError::CannotInferType {
                    help: Some("call it, or wrap the call in a closure".to_string()),
                    message: format!(
                        "cannot infer the type arguments of the generic '{}' used as a value",
                        qualified
                    ),
                    span,
                });
                return Some((self.error_node(Vec::new(), span), is_var));
            }
        }
        Some((self.variant_node(&ty, member, member_ty, span), is_var))
    }

    /// Reports `E4013` for the payload-free variant `name` of a generic enum, used where no
    /// expected type gives its type arguments (`val o = Option.None`).
    fn uninferred_unit_variant(&mut self, name: &str, span: Span) -> ExpressionId {
        self.errors.push(TypeError::CannotInferType {
            help: Some(format!(
                "annotate the expected type, as in 'val x: {}<...> = {}'",
                name.split('.').next().unwrap_or(name),
                name
            )),
            message: format!("cannot infer the type arguments of '{}'", name),
            span,
        });
        self.error_node(Vec::new(), span)
    }

    /// The enum and name of the payload-free variant of a generic enum that `expr` names
    /// (`Option.None`, or a bare `None`), whose type arguments come from the expected type.
    fn generic_unit_variant(&self, expr: &Expr) -> Option<(NominalId, String)> {
        let (id, member) = match &expr.kind {
            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav: false,
            } => match self.type_name_symbol(target)?.kind {
                SymbolKind::Enum { id, .. } => (id, member.clone()),
                _ => return None,
            },
            ExprKind::Ident(name)
                if self.variant_owners.contains_key(name) && !self.env.resolves_to_local(name) =>
            {
                match &self.env.lookup(name)?.kind {
                    SymbolKind::Variable {
                        ty: Type::Enum { id, .. },
                        local: None,
                        ..
                    } => (id.clone(), name.clone()),
                    _ => return None,
                }
            }
            _ => return None,
        };
        if self.types.parameters(&id).is_empty() {
            return None;
        }
        let index = self.variant_index(&id, &member)?;
        let payload_free = self.types.enum_variants(&id)?[index].1.is_empty();
        payload_free.then_some((id, member))
    }

    /// Checks the payload-free variant `member` of the generic enum `id` against `expected`,
    /// which gives its type arguments, or reports `E4013`.
    fn check_generic_unit_variant(
        &mut self,
        expr: &Expr,
        id: &NominalId,
        member: &str,
        expected: &Type,
    ) -> ExpressionId {
        let parameters = self.types.parameters(id).to_vec();
        let pattern = Type::Enum {
            id: id.clone(),
            arguments: parameter_types(&parameters),
        };
        let mut unknowns = Unknowns::new(parameters.len());
        unknowns.match_exact(&pattern, expected);
        let visible = match &expr.kind {
            ExprKind::MemberAccess { target, .. } => match self.type_name_symbol(target) {
                Some(sym) => self.check_visibility(&sym, target.span),
                None => true,
            },
            ExprKind::Ident(name) => self.resolve_ident(name, expr.span).is_some(),
            _ => true,
        };
        if !visible {
            return self.error_node(Vec::new(), expr.span);
        }
        if !(0..parameters.len()).all(|index| unknowns.is_exact(index)) {
            let name = format!("{}.{}", id.name, member);
            return self.uninferred_unit_variant(&name, expr.span);
        }
        let ty = pattern.substitute(&unknowns.partial());
        let node = self.variant_node(&ty, member, ty.clone(), expr.span);
        self.fit(expr, node, expected, Site::Value, false)
    }

    /// The node of variant `member` of the enum `enum_ty`, whose type `member_ty` is the enum for
    /// a payload-free variant (a construction with no fields) and the constructor's function type
    /// for a variant with a payload.
    fn variant_node(
        &mut self,
        enum_ty: &Type,
        member: &str,
        member_ty: Type,
        span: Span,
    ) -> ExpressionId {
        let Type::Enum { id: enum_id, .. } = peel_indirection(enum_ty).clone() else {
            return self.error_node(Vec::new(), span);
        };
        let index = self.variant_index(&enum_id, member);
        let Some(index) = index else {
            return self.error_node(Vec::new(), span);
        };
        let name = member.to_string();
        let kind = match &member_ty {
            Type::Error => return self.error_node(Vec::new(), span),
            Type::Fn { .. } => ExpressionKind::VariantConstructor {
                enum_id,
                index,
                name,
            },
            _ => ExpressionKind::Construct {
                kind: ConstructKind::Variant {
                    enum_id,
                    index,
                    name,
                },
                fields: Vec::new(),
                evaluation_order: Vec::new(),
            },
        };
        self.add(kind, member_ty, span)
    }

    /// Binds the bare name of a variant of `enum_name` at module level.
    ///
    /// Variants are namespaced by their enum, so when two enums declare the same variant name the
    /// bare name is unbound and recorded as ambiguous rather than reported as a duplicate. Any
    /// other clash is a duplicate symbol, returned as `E4010`.
    #[allow(clippy::result_large_err)]
    fn define_variant(&mut self, enum_name: &str, symbol: Symbol) -> Result<(), TypeError> {
        let name = symbol.name.clone();
        if self.ambiguous_variants.contains(&name) {
            return Ok(());
        }
        if let Some(owner) = self.variant_owners.get(&name) {
            if owner != enum_name {
                self.env.remove(&name);
                self.variant_owners.remove(&name);
                self.ambiguous_variants.insert(name);
                return Ok(());
            }
        }
        self.env.define(symbol)?;
        self.variant_owners.insert(name, enum_name.to_string());
        Ok(())
    }

    /// Ingests exported symbols from an imported file's Phase 1 FileAbi.
    ///
    /// Each exported struct and enum is identified by the exporter's module path. Types in the
    /// exported signatures are already bound to their defining modules by Phase 1, so they are
    /// converted as they are.
    pub fn ingest_file_abi(&mut self, abi: &FileAbi) {
        self.direct_imports.insert(abi.module.clone());
        // The definitions of the types the import's exports reach are recorded for layout only
        // (D4 of `codegen-aggregates`): no name is defined, so a private type stays unnameable,
        // and using a member still needs its module imported (`E4014`).
        for definition in &abi.type_definitions {
            let (id, type_definition, marker) = crate::types::type_definition_from_abi(definition);
            self.types
                .insert_marker(id.clone(), marker.marker, marker.parameters);
            self.types.insert_definition(id, type_definition);
        }
        for ty in abi.symbols.iter().flat_map(abi_symbol_types) {
            self.record_nominals(ty);
        }
        for sym in &abi.symbols {
            if let ExportedSymbol::Struct {
                name,
                marker,
                marker_parameters,
                ..
            }
            | ExportedSymbol::Enum {
                name,
                marker,
                marker_parameters,
                ..
            } = sym
            {
                let id = NominalId::new(abi.module.clone(), name.clone());
                self.types
                    .insert_marker(id.clone(), *marker, marker_parameters.clone());
                self.exported_nominals.insert(id);
            }
            let symbol = match sym {
                ExportedSymbol::Struct {
                    visibility, name, ..
                } => Symbol::struct_decl(
                    NominalId::new(abi.module.clone(), name.clone()),
                    *visibility,
                    abi.package.clone(),
                    Span::dummy(),
                ),
                ExportedSymbol::Enum {
                    visibility, name, ..
                } => Symbol::enum_decl(
                    NominalId::new(abi.module.clone(), name.clone()),
                    *visibility,
                    abi.package.clone(),
                    Span::dummy(),
                ),
                _ => continue,
            };
            self.env.define_import(&abi.module, symbol);
        }
        for sym in &abi.symbols {
            match sym {
                ExportedSymbol::Function {
                    visibility,
                    kind,
                    is_compose,
                    receiver,
                    context,
                    name,
                    type_parameters,
                    params,
                    return_type,
                } => {
                    let type_parameters = type_parameters_from_abi(type_parameters);
                    let param_types = params
                        .iter()
                        .map(|parameter| type_from_abi(&parameter.ty, &type_parameters))
                        .collect();
                    let param_names = params.iter().map(|p| p.name.clone()).collect();
                    let has_default = params.iter().map(|p| p.default.is_some()).collect();
                    let fn_ty = Type::Fn {
                        is_compose: *is_compose,
                        receiver: false,
                        params: param_types,
                        escaping: params.iter().map(|p| p.escaping).collect(),
                        ret: Box::new(type_from_abi(return_type, &type_parameters)),
                    };
                    let symbol = Symbol::function_with_defaults(
                        name.clone(),
                        fn_ty,
                        param_names,
                        has_default,
                        *visibility,
                        abi.package.clone(),
                        Span::dummy(),
                    )
                    .with_module(abi.module.clone())
                    .with_type_parameters(type_parameters.clone());
                    // An intrinsic is a declaration the catalog knows (checked in its own
                    // file, D6 of `standard-library`); one it does not know is left out.
                    let intrinsic = (*kind == FunctionKind::Intrinsic).then(|| {
                        let constructor = receiver.as_ref().map(|r| abi_constructor_name(&r.ty));
                        crate::intrinsics::lookup(&abi.module, constructor.as_deref(), name)
                    });
                    match receiver {
                        // An intrinsic extension is a member of its receiver type (D7).
                        Some(receiver) if intrinsic.is_some() => {
                            let Some(Some(entry)) = intrinsic else {
                                continue;
                            };
                            let receiver_ty = type_from_abi(&receiver.ty, &type_parameters);
                            let key = (ReceiverKey::of(&receiver_ty), name.clone());
                            self.members.entry(key).or_insert(Extension {
                                mode: receiver.mode,
                                receiver: receiver_ty,
                                module: abi.module.clone(),
                                symbol,
                                builtin: Some(entry.builtin),
                                property: false,
                                writable: false,
                                context: None,
                            });
                        }
                        // An extension is reached through its receiver, never by its bare name.
                        Some(receiver) => {
                            let receiver_ty = type_from_abi(&receiver.ty, &type_parameters);
                            let context_ty = context
                                .as_ref()
                                .map(|context| type_from_abi(context, &type_parameters));
                            let key = (ReceiverKey::of(&receiver_ty), name.clone());
                            let extension = Extension {
                                mode: receiver.mode,
                                receiver: receiver_ty,
                                module: abi.module.clone(),
                                symbol: symbol.with_context(context_ty.clone()),
                                builtin: None,
                                property: false,
                                writable: false,
                                context: context_ty,
                            };
                            self.imported_extensions
                                .entry(key)
                                .or_default()
                                .insert(&abi.module, extension);
                        }
                        None => {
                            match intrinsic {
                                Some(Some(entry)) => {
                                    self.intrinsic_functions
                                        .insert((abi.module.clone(), name.clone()), entry.builtin);
                                }
                                Some(None) => continue,
                                None => {}
                            }
                            if *kind == FunctionKind::External {
                                let path = DeclarationPath::new(abi.module.clone(), name.clone());
                                if !self.externals.contains(&path) {
                                    self.externals.push(path);
                                }
                            }
                            let context_ty = context
                                .as_ref()
                                .map(|context| type_from_abi(context, &type_parameters));
                            self.env
                                .define_import(&abi.module, symbol.with_context(context_ty))
                        }
                    }
                }
                ExportedSymbol::ExtensionProperty {
                    type_parameters,
                    receiver,
                    name,
                    ty,
                    visibility,
                    writable,
                } => {
                    let constructor = abi_constructor_name(receiver);
                    let Some(entry) =
                        crate::intrinsics::lookup(&abi.module, Some(&constructor), name)
                    else {
                        continue;
                    };
                    let type_parameters = type_parameters_from_abi(type_parameters);
                    let receiver_ty = type_from_abi(receiver, &type_parameters);
                    let property_ty = type_from_abi(ty, &type_parameters);
                    let symbol = Symbol::function_with_defaults(
                        name.clone(),
                        Type::function(false, Vec::new(), property_ty),
                        Vec::new(),
                        Vec::new(),
                        *visibility,
                        abi.package.clone(),
                        Span::dummy(),
                    )
                    .with_module(abi.module.clone())
                    .with_type_parameters(type_parameters);
                    let key = (ReceiverKey::of(&receiver_ty), name.clone());
                    self.members.entry(key).or_insert(Extension {
                        mode: ReceiverMode::Shared,
                        receiver: receiver_ty,
                        module: abi.module.clone(),
                        symbol,
                        builtin: Some(entry.builtin),
                        property: true,
                        writable: *writable,
                        context: None,
                    });
                }
                ExportedSymbol::Struct {
                    name,
                    type_parameters,
                    fields,
                    ..
                } => {
                    let parameters = type_parameters_from_abi(type_parameters);
                    let fields = fields
                        .iter()
                        .map(|(f_name, is_var, f_ty)| {
                            (f_name.clone(), *is_var, type_from_abi(f_ty, &parameters))
                        })
                        .collect();
                    let id = NominalId::new(abi.module.clone(), name.clone());
                    self.types
                        .insert_definition(id, TypeDef::Struct { parameters, fields });
                }
                ExportedSymbol::Enum {
                    visibility,
                    name,
                    type_parameters,
                    variants,
                    ..
                } => {
                    let parameters = type_parameters_from_abi(type_parameters);
                    let variant_types: Vec<(String, Vec<Type>)> = variants
                        .iter()
                        .map(|(v_name, f_types)| {
                            let payloads = f_types.iter().map(|ty| type_from_abi(ty, &parameters));
                            (v_name.clone(), payloads.collect())
                        })
                        .collect();
                    let id = NominalId::new(abi.module.clone(), name.clone());
                    let enum_ty = Type::Enum {
                        id: id.clone(),
                        arguments: parameter_types(&parameters),
                    };
                    self.types.insert_definition(
                        id,
                        TypeDef::Enum {
                            parameters: parameters.clone(),
                            variants: variant_types.clone(),
                        },
                    );

                    for (v_name, f_types) in &variant_types {
                        let v_sym = if f_types.is_empty() {
                            Symbol::top_level_variable(
                                v_name.clone(),
                                enum_ty.clone(),
                                Mutability::Val,
                                *visibility,
                                abi.package.clone(),
                                Span::dummy(),
                            )
                        } else {
                            Symbol::function(
                                v_name.clone(),
                                Type::function(false, f_types.clone(), enum_ty.clone()),
                                *visibility,
                                abi.package.clone(),
                                Span::dummy(),
                            )
                            .with_type_parameters(parameters.clone())
                        }
                        .with_module(abi.module.clone());
                        self.define_variant(name, v_sym).ok();
                    }
                }
                ExportedSymbol::Var {
                    visibility,
                    name,
                    ty,
                } => {
                    let symbol = Symbol::top_level_variable(
                        name.clone(),
                        type_from_abi(ty, &[]),
                        Mutability::Val,
                        *visibility,
                        abi.package.clone(),
                        Span::dummy(),
                    )
                    .with_module(abi.module.clone());
                    self.env.define_import(&abi.module, symbol);
                }
            }
        }
    }

    /// Reports `E4023` for each struct or enum of the file that contains itself by value (D20 of
    /// `codegen-aggregates`): through its fields or payloads, nullables and other structs and
    /// enums with their type arguments substituted, but not through `Heap<T>`, references or
    /// function types, which hold their values elsewhere.
    fn check_finite_sizes(&mut self, source_file: &SourceFile) {
        for decl in &source_file.declarations {
            let (name, name_span, type_parameters) = match decl {
                Declaration::Struct(s) => (&s.name, s.name_span, &s.type_parameters),
                Declaration::Enum(e) => (&e.name, e.name_span, &e.type_parameters),
                _ => continue,
            };
            let id = NominalId::new(self.module_path.clone(), name.clone());
            let Some(definition) = self.types.definition(&id) else {
                continue;
            };
            let arguments = parameter_types(&type_parameters_of(type_parameters));
            let declared = match definition {
                TypeDef::Struct { .. } => Type::Struct {
                    id: id.clone(),
                    arguments,
                },
                TypeDef::Enum { .. } => Type::Enum {
                    id: id.clone(),
                    arguments,
                },
            };
            let mut stack = vec![declared.clone()];
            let mut path = Vec::new();
            if self.contains_by_value(&declared, &id, &mut stack, &mut path) {
                let steps: Vec<String> = path.iter().map(|ty| format!("'{}'", ty)).collect();
                self.errors.push(TypeError::InfinitelySizedType {
                    type_name: name.clone(),
                    path: format!("'{}' contains {}", name, steps.join(", which contains ")),
                    span: name_span,
                });
            }
        }
    }

    /// Whether a value of the struct or enum `ty` contains a value of the struct or enum `target`
    /// by value, leaving in `path` the member types leading to it. `stack` holds the types being
    /// expanded: a type already on it, other than `target`, is on a cycle `target` is not on, and
    /// is not expanded again. Neither is a type deeper than [`MAXIMUM_EXPANSION_DEPTH`] or larger
    /// than [`MAXIMUM_EXPANDED_TYPE_SIZE`], which only a generic type growing its own arguments
    /// reaches; that type's own declaration reaches itself at once, so it is reported there.
    fn contains_by_value(
        &self,
        ty: &Type,
        target: &NominalId,
        stack: &mut Vec<Type>,
        path: &mut Vec<Type>,
    ) -> bool {
        let members: Vec<Type> = match ty {
            Type::Struct { id, arguments } => self
                .types
                .instantiated_fields(id, arguments)
                .unwrap_or_default()
                .into_iter()
                .map(|(_, _, ty)| ty)
                .collect(),
            Type::Enum { id, arguments } => self
                .types
                .instantiated_variants(id, arguments)
                .unwrap_or_default()
                .into_iter()
                .flat_map(|(_, payloads)| payloads)
                .collect(),
            _ => return false,
        };
        for member in members {
            // A nullable holds its payload inline; `Heap<T>`, references and functions do not.
            let mut held = &member;
            while let Type::Nullable(inner) = held {
                held = inner;
            }
            let Some(id) = held.nominal_id() else {
                continue;
            };
            path.push(member.clone());
            if id == target {
                return true;
            }
            let expandable = stack.len() <= MAXIMUM_EXPANSION_DEPTH
                && type_size(held) <= MAXIMUM_EXPANDED_TYPE_SIZE;
            if expandable && !stack.contains(held) {
                stack.push(held.clone());
                let found = self.contains_by_value(held, target, stack, path);
                stack.pop();
                if found {
                    return true;
                }
            }
            path.pop();
        }
        false
    }

    /// Pass 1: Registers all top-level symbols (functions, structs, enums, variables) in module scope.
    ///
    /// Enables forward references so functions can call sibling functions declared later in the file.
    /// Every struct and enum name is declared before any annotation is lowered, so field and
    /// signature types may name types declared later in the file.
    pub fn register_module_declarations(&mut self, source_file: &SourceFile) {
        // `Heap(...)` always constructs a box, so a top-level declaration named `Heap` could
        // never be called or named (D2 of `heap-boxes`), and `List` is the built-in list type
        // (D2 of `collections`). An extension is named by its receiver.
        for decl in &source_file.declarations {
            let (name, span) = match decl {
                Declaration::Function(f) if f.receiver.is_none() => (&f.name, f.span),
                Declaration::Struct(s) => (&s.name, s.span),
                Declaration::Enum(e) => (&e.name, e.span),
                Declaration::Var(v) => (&v.name, v.span),
                Declaration::Function(_) | Declaration::ExtensionProperty(_) => continue,
            };
            if name == HEAP || name == LIST {
                self.errors.push(TypeError::BuiltinNameDeclared {
                    name: name.clone(),
                    span,
                });
            }
        }
        for decl in &source_file.declarations {
            let symbol = match decl {
                Declaration::Struct(s) => {
                    let mut seen_fields = HashSet::new();
                    for f in &s.fields {
                        if !seen_fields.insert(f.name.clone()) {
                            self.errors.push(TypeError::DuplicateSymbol {
                                name: format!(
                                    "duplicate field '{}' in struct '{}'",
                                    f.name, s.name
                                ),
                                span: f.span,
                            });
                        }
                    }
                    self.check_type_parameter_names(&s.type_parameters);
                    if s.is_intrinsic() {
                        self.check_intrinsic_struct(s);
                    }
                    let id = NominalId::new(self.module_path.clone(), s.name.clone());
                    let members: Vec<&prism_ast::types::Type> =
                        s.fields.iter().map(|f| &f.ty).collect();
                    let marker_parameters =
                        prism_abi::marker_parameters(&s.type_parameters, &members);
                    self.types
                        .insert_marker(id.clone(), s.marker, marker_parameters);
                    // The parameters are known before any annotation is lowered; the fields
                    // follow below.
                    self.types.insert_definition(
                        id.clone(),
                        TypeDef::Struct {
                            parameters: type_parameters_of(&s.type_parameters),
                            fields: Vec::new(),
                        },
                    );
                    Symbol::struct_decl(id, s.visibility, self.current_package.clone(), s.span)
                }
                Declaration::Enum(e) => {
                    let mut seen_variants = HashSet::new();
                    for v in &e.variants {
                        if !seen_variants.insert(v.name.clone()) {
                            self.errors.push(TypeError::DuplicateSymbol {
                                name: format!(
                                    "duplicate variant '{}' in enum '{}'",
                                    v.name, e.name
                                ),
                                span: v.span,
                            });
                        }
                    }
                    self.check_type_parameter_names(&e.type_parameters);
                    let id = NominalId::new(self.module_path.clone(), e.name.clone());
                    let members: Vec<&prism_ast::types::Type> =
                        e.variants.iter().flat_map(|v| &v.fields).collect();
                    let marker_parameters =
                        prism_abi::marker_parameters(&e.type_parameters, &members);
                    self.types
                        .insert_marker(id.clone(), e.marker, marker_parameters);
                    self.types.insert_definition(
                        id.clone(),
                        TypeDef::Enum {
                            parameters: type_parameters_of(&e.type_parameters),
                            variants: Vec::new(),
                        },
                    );
                    Symbol::enum_decl(id, e.visibility, self.current_package.clone(), e.span)
                }
                _ => continue,
            };
            self.define_top_level(symbol);
        }

        for decl in &source_file.declarations {
            match decl {
                Declaration::Struct(s) => {
                    let parameters = type_parameters_of(&s.type_parameters);
                    let fields = self.with_type_parameters(parameters.clone(), |c| {
                        for f in &s.fields {
                            c.check_annotation_names(&f.ty);
                        }
                        s.fields
                            .iter()
                            .map(|f| (f.name.clone(), f.is_var, c.lower_type(&f.ty)))
                            .collect()
                    });
                    let id = NominalId::new(self.module_path.clone(), s.name.clone());
                    self.types
                        .replace_definition(id, TypeDef::Struct { parameters, fields });
                }
                Declaration::Enum(e) => {
                    let parameters = type_parameters_of(&e.type_parameters);
                    let variants: Vec<(String, Vec<Type>)> =
                        self.with_type_parameters(parameters.clone(), |c| {
                            for payload in e.variants.iter().flat_map(|v| &v.fields) {
                                c.check_annotation_names(payload);
                            }
                            e.variants
                                .iter()
                                .map(|v| {
                                    (
                                        v.name.clone(),
                                        v.fields.iter().map(|f| c.lower_type(f)).collect(),
                                    )
                                })
                                .collect()
                        });
                    let id = NominalId::new(self.module_path.clone(), e.name.clone());
                    // Inside the declaration, the enum is applied to its own parameters.
                    let enum_ty = Type::Enum {
                        id: id.clone(),
                        arguments: parameter_types(&parameters),
                    };
                    self.types.replace_definition(
                        id,
                        TypeDef::Enum {
                            parameters: parameters.clone(),
                            variants: variants.clone(),
                        },
                    );

                    for (v_name, f_types) in &variants {
                        let v_sym = if f_types.is_empty() {
                            Symbol::top_level_variable(
                                v_name.clone(),
                                enum_ty.clone(),
                                Mutability::Val,
                                e.visibility,
                                self.current_package.clone(),
                                e.span,
                            )
                            .with_module(self.module_path.clone())
                        } else {
                            Symbol::function_with_defaults(
                                v_name.clone(),
                                Type::function(false, f_types.clone(), enum_ty.clone()),
                                vec![String::new(); f_types.len()],
                                vec![false; f_types.len()],
                                e.visibility,
                                self.current_package.clone(),
                                e.span,
                            )
                            .with_module(self.module_path.clone())
                            .with_type_parameters(parameters.clone())
                        };
                        if let Err(err) = self.define_variant(&e.name, v_sym) {
                            self.errors.push(err);
                        }
                    }
                }
                // Extensions are registered once every struct's fields are known (below).
                Declaration::Function(f) if f.receiver.is_some() => {}
                Declaration::ExtensionProperty(_) => {}
                Declaration::Function(f) => {
                    self.register_function_decl(f);
                }
                Declaration::Var(v) => {
                    let ty = if let Some(annot) = &v.ty {
                        self.check_annotation_names(annot);
                        self.lower_type(annot)
                    } else if v.visibility != Visibility::Private {
                        self.errors.push(TypeError::TypeMismatch {
                            expected: Type::Error,
                            found: Type::Error,
                            span: v.span,
                            help: None,
                        });
                        Type::Error
                    } else {
                        Type::Error
                    };

                    let sym = Symbol::top_level_variable(
                        v.name.clone(),
                        ty,
                        Mutability::Val,
                        v.visibility,
                        self.current_package.clone(),
                        v.span,
                    )
                    .with_module(self.module_path.clone());
                    self.define_top_level(sym);
                }
            }
        }

        // The library's intrinsic members first, so that an extension named like one is
        // reported (D7 of `standard-library`).
        for decl in &source_file.declarations {
            match decl {
                Declaration::Function(f) if f.receiver.is_some() && f.is_intrinsic() => {
                    self.register_intrinsic_extension(f);
                }
                Declaration::ExtensionProperty(p) => self.register_intrinsic_property(p),
                _ => {}
            }
        }
        for decl in &source_file.declarations {
            if let Declaration::Function(f) = decl {
                if f.receiver.is_some() && !f.is_intrinsic() {
                    self.register_extension_decl(f);
                }
            }
        }
    }

    /// Checks the `intrinsic struct` `s` of a library file against the catalog of intrinsic
    /// structs (D16 of `compose-transform`): `E4029` for a name the compiler provides no
    /// intrinsic struct of in this module, or another declaration than its canonical one
    /// (visibility, type parameters and marker).
    fn check_intrinsic_struct(&mut self, s: &prism_ast::StructDecl) {
        let id = NominalId::new(self.module_path.clone(), s.name.clone());
        let Some(intrinsic) = id.intrinsic() else {
            self.errors.push(TypeError::InvalidLibraryDeclaration {
                message: format!(
                    "the compiler provides no intrinsic struct '{}' in '/{}'",
                    s.name, self.module_path
                ),
                help: Some(
                    "an intrinsic struct declares a type the compiler lays out itself; declare a struct with fields instead".to_string(),
                ),
                span: s.name_span,
            });
            return;
        };
        let mut text = format!("{} intrinsic struct {}", s.visibility, s.name);
        if !s.type_parameters.is_empty() {
            let names: Vec<&str> = s.type_parameters.iter().map(|p| p.name.as_str()).collect();
            text.push_str(&format!("<{}>", names.join(", ")));
        }
        if let Some(marker) = s.marker {
            text.push_str(&format!(" : {}", marker.as_str()));
        }
        let expected = intrinsic.declaration_text();
        if text != expected {
            self.errors.push(TypeError::InvalidLibraryDeclaration {
                message: format!(
                    "the intrinsic struct '{}' is declared as '{}', but the compiler provides '{}'",
                    s.name, text, expected
                ),
                help: Some(format!("declare it as '{}'", expected)),
                span: s.name_span,
            });
        }
    }

    /// Checks the intrinsic declaration whose path is `module::receiver.name` and whose text
    /// is `text` against the catalog (D6 of `standard-library`), returning its entry, or
    /// reporting `E4029` for a path the catalog does not have or a different signature.
    fn check_intrinsic(
        &mut self,
        receiver: Option<&str>,
        name: &str,
        text: &str,
        span: Span,
    ) -> Option<&'static crate::intrinsics::Intrinsic> {
        let Some(entry) = crate::intrinsics::lookup(&self.module_path, receiver, name) else {
            let on = receiver.map_or(String::new(), |receiver| format!(" on '{}'", receiver));
            self.errors.push(TypeError::InvalidLibraryDeclaration {
                message: format!(
                    "the compiler provides no intrinsic '{}'{} in '/{}'",
                    name, on, self.module_path
                ),
                help: Some(
                    "an intrinsic declares a function the compiler lowers itself; write a function with a body instead".to_string(),
                ),
                span,
            });
            return None;
        };
        if entry.signature != text {
            // A property declared `var` that the catalog lists as read-only, or the reverse
            // (D4 of `state-handles`).
            let swapped = |text: &str| {
                text.replacen(" intrinsic val ", " intrinsic \u{0}", 1)
                    .replacen(" intrinsic var ", " intrinsic val ", 1)
                    .replacen(" intrinsic \u{0}", " intrinsic var ", 1)
            };
            let message = if swapped(text) == entry.signature {
                let provided = if entry.signature.contains(" intrinsic var ") {
                    "a writable"
                } else {
                    "a read-only"
                };
                format!(
                    "the compiler provides '{}' as {} property: declare it '{}'",
                    name, provided, entry.signature
                )
            } else {
                format!(
                    "the intrinsic '{}' is declared as '{}', but the compiler provides '{}'",
                    name, text, entry.signature
                )
            };
            self.errors.push(TypeError::InvalidLibraryDeclaration {
                message,
                help: Some(format!("declare it as '{}'", entry.signature)),
                span,
            });
        }
        Some(entry)
    }

    /// Reports `E4029` for each rule of external functions `f` breaks (D5 of
    /// `standard-library`): an external is not `public`, generic, an extension or `compose`, has
    /// no default values, and returns no type holding a reference.
    fn check_external(&mut self, f: &FunctionDecl) {
        let mut broken = Vec::new();
        if f.visibility == Visibility::Public {
            broken.push((
                "is 'public'",
                "make it private and call it from a public function of the library",
            ));
        }
        if !f.type_parameters.is_empty() {
            broken.push((
                "is generic",
                "one runtime symbol has one machine signature: declare one external per type",
            ));
        }
        if f.receiver.is_some() {
            broken.push((
                "is an extension",
                "declare a plain external and an extension calling it",
            ));
        }
        if f.is_compose {
            broken.push((
                "is 'compose'",
                "declare a plain external and a composable calling it",
            ));
        }
        if f.params.iter().any(|p| p.default_value.is_some()) {
            broken.push((
                "has default values",
                "give the defaults to a function calling it",
            ));
        }
        let result = f.return_type.as_ref().map(|ty| self.lower_type(ty));
        if result
            .as_ref()
            .is_some_and(|ty| ty.reference_outside_functions().is_some())
        {
            broken.push((
                "returns a type holding a reference",
                "no elision rule can make the runtime's answer safe: return an owned value",
            ));
        }
        for (rule, help) in broken {
            self.errors.push(TypeError::InvalidLibraryDeclaration {
                message: format!("the external function '{}' {}", f.name, rule),
                help: Some(help.to_string()),
                span: f.span,
            });
        }
    }

    /// Registers the intrinsic extension `f` of a library file as a member of its receiver
    /// type, once the catalog accepts it (D6 and D7 of `standard-library`).
    fn register_intrinsic_extension(&mut self, f: &FunctionDecl) {
        let receiver = f.receiver.as_ref().expect("an extension has a receiver");
        let text = crate::intrinsics::declaration_text(f);
        let constructor = crate::intrinsics::constructor_name(&receiver.ty);
        let Some(entry) = self.check_intrinsic(constructor, &f.name, &text, f.span) else {
            return;
        };
        let receiver_ty = self.with_type_parameters(type_parameters_of(&f.type_parameters), |c| {
            c.check_annotation_names(&receiver.ty);
            c.lower_type(&receiver.ty)
        });
        let symbol = self.function_symbol(f);
        let key = (ReceiverKey::of(&receiver_ty), f.name.clone());
        self.members.insert(
            key,
            Extension {
                mode: receiver.mode,
                receiver: receiver_ty,
                module: self.module_path.clone(),
                symbol,
                builtin: Some(entry.builtin),
                property: false,
                writable: false,
                context: None,
            },
        );
    }

    /// Registers the intrinsic property `p` of a library file as a member of its receiver type
    /// (D6 and D7 of `standard-library`).
    fn register_intrinsic_property(&mut self, p: &prism_ast::decl::ExtensionPropertyDecl) {
        let text = crate::intrinsics::property_text(p);
        let constructor = crate::intrinsics::constructor_name(&p.receiver);
        let Some(entry) = self.check_intrinsic(constructor, &p.name, &text, p.span) else {
            return;
        };
        let parameters = type_parameters_of(&p.type_parameters);
        let (receiver_ty, property_ty) = self.with_type_parameters(parameters.clone(), |c| {
            c.check_annotation_names(&p.receiver);
            c.check_annotation_names(&p.ty);
            (c.lower_type(&p.receiver), c.lower_type(&p.ty))
        });
        let symbol = Symbol::function_with_defaults(
            p.name.clone(),
            Type::function(false, Vec::new(), property_ty),
            Vec::new(),
            Vec::new(),
            p.visibility,
            self.current_package.clone(),
            p.span,
        )
        .with_module(self.module_path.clone())
        .with_type_parameters(parameters);
        let key = (ReceiverKey::of(&receiver_ty), p.name.clone());
        self.members.insert(
            key,
            Extension {
                mode: ReceiverMode::Shared,
                receiver: receiver_ty,
                module: self.module_path.clone(),
                symbol,
                builtin: Some(entry.builtin),
                property: true,
                writable: p.writable,
                context: None,
            },
        );
    }

    /// Defines a top-level declaration in module scope. A declaration with the same name as an
    /// imported symbol shadows the import: lookups check module scope before the import layer.
    fn define_top_level(&mut self, symbol: Symbol) {
        if let Err(err) = self.env.define(symbol) {
            self.errors.push(err);
        }
    }

    fn register_function_decl(&mut self, f: &FunctionDecl) {
        if f.is_intrinsic() {
            let text = crate::intrinsics::declaration_text(f);
            if let Some(entry) = self.check_intrinsic(None, &f.name, &text, f.span) {
                self.intrinsic_functions
                    .insert((self.module_path.clone(), f.name.clone()), entry.builtin);
            }
        } else if f.is_external() {
            self.check_external(f);
            let path = DeclarationPath::new(self.module_path.clone(), f.name.clone());
            if !self.externals.contains(&path) {
                self.externals.push(path);
            }
        }
        let sym = self.function_symbol(f);
        self.define_top_level(sym);
    }

    /// The symbol of function `f`: its signature, parameter names, defaults and visibility. An
    /// unannotated private expression body gets the error type as a provisional return type.
    fn function_symbol(&mut self, f: &FunctionDecl) -> Symbol {
        self.check_type_parameter_names(&f.type_parameters);
        let type_parameters = type_parameters_of(&f.type_parameters);
        let symbol =
            self.with_type_parameters(type_parameters.clone(), |c| c.function_signature_symbol(f));
        symbol.with_type_parameters(type_parameters)
    }

    /// [`Self::function_symbol`] with the function's type parameters in scope.
    fn function_signature_symbol(&mut self, f: &FunctionDecl) -> Symbol {
        for annot in f
            .params
            .iter()
            .map(|p| &p.ty)
            .chain(&f.return_type)
            .chain(f.context.as_ref().map(|c| &c.ty))
        {
            self.check_annotation_names(annot);
        }
        let context = f
            .context
            .as_ref()
            .map(|c| self.lower_type(&c.ty))
            .filter(|ty| ty != &Type::Error);
        let params: Vec<Type> = f.params.iter().map(|p| self.lower_type(&p.ty)).collect();
        let param_names: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
        let has_default: Vec<bool> = f.params.iter().map(|p| p.default_value.is_some()).collect();

        let ret = if let Some(annot) = &f.return_type {
            self.lower_type(annot)
        } else if f.visibility != Visibility::Private {
            self.errors.push(TypeError::MissingReturnType {
                function_name: f.name.clone(),
                span: f.span,
            });
            Type::Error
        } else if matches!(f.body, Some(FunctionBody::Block(_)) | None) {
            Type::Unit
        } else {
            self.unannotated_private_fns.push(f.name.clone());
            Type::Error
        };

        let fn_ty = Type::Fn {
            is_compose: f.is_compose,
            receiver: false,
            params,
            escaping: f.params.iter().map(|p| p.escaping).collect(),
            ret: Box::new(ret),
        };

        Symbol::function_with_defaults(
            f.name.clone(),
            fn_ty,
            param_names,
            has_default,
            f.visibility,
            self.current_package.clone(),
            f.span,
        )
        .with_module(self.module_path.clone())
        .with_context(context)
    }

    /// Registers the extension function `f` in the file's extension table (D7 of
    /// `extension-methods`). Its receiver type follows the rules of annotations. Two extensions
    /// with the same receiver, name and context (D8 of `receiver-lambdas`), an extension named
    /// like a field of its receiver struct,
    /// an extension named `clone` on a `Clone` type, and one named like a built-in member
    /// (`toString` on `Int`, `Double`, `Bool` or `Char`, `length` on `String`) are rejected with
    /// `E4010`: all but the first could never be called, because members take precedence.
    fn register_extension_decl(&mut self, f: &FunctionDecl) {
        let receiver = f.receiver.as_ref().expect("an extension has a receiver");
        if f.is_external() {
            self.check_external(f);
        }
        let receiver_ty = self.with_type_parameters(type_parameters_of(&f.type_parameters), |c| {
            c.check_annotation_names(&receiver.ty);
            c.lower_type(&receiver.ty)
        });
        let symbol = self.function_symbol(f);
        if receiver_ty == Type::Error {
            return;
        }
        let key = (ReceiverKey::of(&receiver_ty), f.name.clone());
        let field_clash = match &receiver_ty {
            Type::Struct { id, .. } => self
                .struct_fields(id)
                .is_some_and(|fields| fields.iter().any(|(name, _, _)| name == &f.name)),
            _ => false,
        };
        // An intrinsic member of the library always takes precedence (D7 of
        // `standard-library`).
        let member_clash = self.member(&receiver_ty, &f.name).map(|m| m.module.clone());
        let context = symbol.context().cloned();
        let duplicate = self
            .own_extensions
            .get(&key)
            .is_some_and(|extensions| extensions.declares(&context));
        if duplicate {
            // Extensions are keyed by the receiver's type constructor (D10 of `generics`), so
            // `Box<Int>.size` and `Box<T>.size` are one name.
            let constructor = match &key.0 {
                ReceiverKey::Nominal(id) => id.name.clone(),
                ReceiverKey::List => LIST.to_string(),
                ReceiverKey::Other(ty) => ty.to_string(),
            };
            let context = context.map_or(String::new(), |c| format!(" with the context '{}'", c));
            self.errors.push(TypeError::DuplicateSymbol {
                name: format!("{}.{}{}", constructor, f.name, context),
                span: f.span,
            });
            return;
        }
        if field_clash {
            self.errors.push(TypeError::UncallableExtension {
                message: format!(
                    "extension '{}.{}' can never be called: the field '{}' of '{}' always takes precedence",
                    receiver_ty, f.name, f.name, receiver_ty
                ),
                span: f.span,
            });
        } else if f.name == "clone" && self.is_clone(&receiver_ty) {
            self.errors.push(TypeError::UncallableExtension {
                message: format!(
                    "extension '{}.clone' can never be called: '{}' is 'Clone', and the built-in 'clone()' always takes precedence",
                    receiver_ty, receiver_ty
                ),
                span: f.span,
            });
        } else if let Some(module) = member_clash {
            self.errors.push(TypeError::UncallableExtension {
                message: format!(
                    "extension '{}.{}' can never be called: the member '{}' of '{}' that '/{}' declares always takes precedence",
                    receiver_ty, f.name, f.name, receiver_ty, module
                ),
                span: f.span,
            });
        }
        let extension = Extension {
            mode: receiver.mode,
            receiver: receiver_ty,
            module: self.module_path.clone(),
            symbol,
            builtin: None,
            property: false,
            writable: false,
            context,
        };
        let module = self.module_path.clone();
        self.own_extensions
            .entry(key)
            .or_default()
            .insert(&module, extension);
    }

    /// Pass 2: Typechecks the bodies of all top-level declarations in the module, returning the
    /// file's typed tree. The tree is produced even when the file has type errors, which are
    /// left in [`Self::errors`].
    pub fn check_module(&mut self, source_file: &SourceFile) -> TypedFile {
        self.register_module_declarations(source_file);
        self.check_marker_declarations(source_file);
        self.check_finite_sizes(source_file);

        // Unannotated private declarations get their type from their body, inferred on demand.
        let mut inferred = HashMap::new();
        for decl in &source_file.declarations {
            let (key, pending) = match decl {
                Declaration::Function(f)
                    if f.visibility == Visibility::Private
                        && f.return_type.is_none()
                        && matches!(f.body, Some(FunctionBody::Expr(_))) =>
                {
                    let key = match &f.receiver {
                        Some(receiver) => InferenceKey::Extension {
                            receiver: ReceiverKey::of(&self.with_type_parameters(
                                type_parameters_of(&f.type_parameters),
                                |c| c.lower_type(&receiver.ty),
                            )),
                            name: f.name.clone(),
                            context: f.context.as_ref().map(|c| self.lower_type(&c.ty)),
                        },
                        None => InferenceKey::Declaration(f.name.clone()),
                    };
                    (key, Box::new(PendingDecl::Function(f.clone())))
                }
                Declaration::Var(v) if v.visibility == Visibility::Private && v.ty.is_none() => (
                    InferenceKey::Declaration(v.name.clone()),
                    Box::new(PendingDecl::Var(v.clone())),
                ),
                _ => continue,
            };
            // A duplicate declaration (`E4010`) keeps the first one's inference.
            if !self.inference.contains_key(&key) {
                self.inference
                    .insert(key.clone(), Inference::Pending(pending));
                inferred.insert(decl.span(), key);
            }
        }

        for decl in &source_file.declarations {
            if let Some(key) = inferred.get(&decl.span()) {
                // Inferred on first use, or here if never used before.
                self.infer_declaration(key);
                continue;
            }
            match decl {
                Declaration::Function(f) => {
                    self.check_function_body(f);
                }
                Declaration::Var(v) => self.check_top_level_var(v),
                _ => {}
            }
        }
        let file = self.finish_file(source_file);
        let recursion =
            crate::recursion::check_polymorphic_recursion(source_file, &file, &self.errors);
        self.errors.extend(recursion);
        file
    }

    /// The typed tree of the file: its items in source order, structs and enums from the type
    /// table, and the functions and values whose bodies were checked.
    fn finish_file(&mut self, source_file: &SourceFile) -> TypedFile {
        let mut file = std::mem::take(&mut self.file);
        let mut items = Vec::new();
        for decl in &source_file.declarations {
            match decl {
                Declaration::Struct(s) => {
                    let id = NominalId::new(self.module_path.clone(), s.name.clone());
                    let fields = self.struct_fields(&id).unwrap_or_default();
                    items.push(Item::Struct(StructItem {
                        type_parameters: self.types.parameters(&id).to_vec(),
                        marker_parameters: self.types.marker_parameters(&id).to_vec(),
                        id,
                        visibility: s.visibility,
                        marker: s.marker,
                        fields,
                        span: s.span,
                    }));
                }
                Declaration::Enum(e) => {
                    let id = NominalId::new(self.module_path.clone(), e.name.clone());
                    let variants = self.enum_variants(&id).unwrap_or_default();
                    items.push(Item::Enum(EnumItem {
                        type_parameters: self.types.parameters(&id).to_vec(),
                        marker_parameters: self.types.marker_parameters(&id).to_vec(),
                        id,
                        visibility: e.visibility,
                        marker: e.marker,
                        variants,
                        span: e.span,
                    }));
                }
                Declaration::Function(_) | Declaration::Var(_) => {
                    if let Some(item) = file.items.remove(&decl.span()) {
                        items.push(item);
                    }
                }
                // An intrinsic property has no code: its reads are lowered where they are.
                Declaration::ExtensionProperty(_) => {}
            }
        }
        let mut externals = self.externals.clone();
        if let Some(prelude) = &self.prelude {
            externals.extend(prelude.externals().iter().cloned());
        }
        TypedFile {
            module: self.module_path.clone(),
            package: self.current_package.clone(),
            items,
            bodies: file.bodies,
            types: self.types.clone(),
            externals,
        }
    }

    /// Starts building the body of the declaration `owner`, returning the builder of the body
    /// being built when it started, which [`Self::finish_body`] restores.
    fn start_body(&mut self, owner: DeclarationPath) -> BodyBuilder {
        std::mem::replace(&mut self.builder, BodyBuilder::new(owner))
    }

    /// Finishes the body being built, restoring `previous`, and records it with `item`, which
    /// gets the body's id.
    /// Before that, it decides which captures each of its closures owns (D2 of
    /// `escaping-closures`).
    fn finish_body(
        &mut self,
        previous: BodyBuilder,
        kind: BodyKind,
        span: Span,
        item: impl FnOnce(prism_typed_tree::BodyId) -> Item,
    ) {
        let built = std::mem::replace(&mut self.builder, previous);
        let has_closures = built.has_closures();
        let mut built = built.finish();
        if has_closures {
            let errors = decide_captures(&mut built, &self.types, kind);
            self.errors.extend(errors);
        }
        let body = self.file.bodies.alloc(built);
        self.file.items.entry(span).or_insert_with(|| item(body));
    }

    /// Checks a top-level variable's initializer: against its annotation, which is the
    /// initializer's expected type (so a closure's parameters take their types from it), as for
    /// a local `val`. An unannotated private variable takes the initializer's type.
    fn check_top_level_var(&mut self, v: &TopLevelVarDecl) {
        let previous = self.start_body(DeclarationPath::new(self.module_path.clone(), &v.name));
        let (ty, value) = if let Some(annot) = &v.ty {
            let expected = self.lower_type(annot);
            let value = self.with_consuming(true, |c| c.check_expr(&v.initializer, &expected));
            (expected, value)
        } else {
            let initializer = self.with_consuming(true, |c| c.infer_expr(&v.initializer));
            let inferred = self.ty(initializer);
            if v.visibility == Visibility::Private {
                self.env.update_type(&v.name, inferred.clone());
            }
            (inferred, initializer)
        };
        self.builder.body_mut().value = Some(value);
        let path = DeclarationPath::new(self.module_path.clone(), &v.name);
        self.finish_body(previous, BodyKind::TopLevelValue, v.span, |body| {
            Item::Value(ValueItem {
                path,
                visibility: v.visibility,
                ty,
                body,
                span: v.span,
            })
        });
    }

    /// Infers the unannotated private declaration `name` if it is still pending. Its body is
    /// checked in a fresh body context (module scope only, and a body of its own in the typed
    /// tree), so the body being checked when the declaration is first used does not leak into
    /// it.
    fn infer_declaration(&mut self, key: &InferenceKey) {
        let pending = match self.inference.get(key) {
            Some(Inference::Pending(pending)) => pending.clone(),
            _ => return,
        };
        self.inference.insert(key.clone(), Inference::InProgress);

        let saved = BodyContext {
            locals: self.env.take_locals(),
            in_compose_stack: std::mem::replace(&mut self.in_compose_stack, vec![false]),
            current_expected_return: self.current_expected_return.take(),
            missing_implicit_parameter: self
                .missing_implicit_parameter
                .replace(MissingImplicitParameter::OutsideClosures),
            in_function: std::mem::replace(&mut self.in_function, false),
            origin: std::mem::replace(&mut self.origin, Origin::Value),
            consuming: std::mem::replace(&mut self.parent_consuming, false),
            builder: std::mem::replace(
                &mut self.builder,
                BodyBuilder::new(DeclarationPath::new("", "<none>")),
            ),
        };
        match pending.as_ref() {
            PendingDecl::Function(f) => self.check_function_body(f),
            PendingDecl::Var(v) => self.check_top_level_var(v),
        }
        self.env.restore_locals(saved.locals);
        self.in_compose_stack = saved.in_compose_stack;
        self.current_expected_return = saved.current_expected_return;
        self.missing_implicit_parameter = saved.missing_implicit_parameter;
        self.in_function = saved.in_function;
        self.origin = saved.origin;
        self.parent_consuming = saved.consuming;
        self.builder = saved.builder;

        self.inference.insert(key.clone(), Inference::Done);
    }

    /// Before `name` is read at `span`, infers it if it is a pending unannotated declaration.
    /// Reaching a declaration whose inference is in progress means its type depends on itself,
    /// which reports `E4013` once per declaration. The use then sees the declaration's provisional
    /// type, whose inferred part (a function's return type, a variable's type) is the error type.
    fn ensure_inferred(&mut self, name: &str, span: Span) {
        if !self.env.resolves_at_module_level(name) {
            return;
        }
        self.ensure_key_inferred(&InferenceKey::Declaration(name.to_string()), span);
    }

    /// Like [`Self::ensure_inferred`], for the declaration or own extension `key`.
    fn ensure_key_inferred(&mut self, key: &InferenceKey, span: Span) {
        match self.inference.get(key) {
            Some(Inference::Pending(_)) => self.infer_declaration(key),
            Some(Inference::InProgress) if self.recursive_inference.insert(key.clone()) => {
                let name = match key {
                    InferenceKey::Declaration(name) => name.clone(),
                    InferenceKey::Extension { receiver, name, .. } => match receiver {
                        ReceiverKey::Nominal(id) => format!("{}.{}", id.name, name),
                        ReceiverKey::List => format!("{}.{}", LIST, name),
                        ReceiverKey::Other(ty) => format!("{}.{}", ty, name),
                    },
                };
                self.errors.push(TypeError::CannotInferType {
                    help: None,
                    message: format!(
                        "cannot infer the type of '{}' because it depends on itself; add an explicit type",
                        name
                    ),
                    span,
                });
            }
            _ => {}
        }
    }

    /// Adds a binding to the body being built, returning the symbol that resolves to it.
    fn bind_local(
        &mut self,
        name: &str,
        ty: Type,
        mutability: Mutability,
        kind: LocalKind,
        span: Span,
    ) -> Symbol {
        let local = self.builder.add_local(Local {
            name: name.to_string(),
            ty: ty.clone(),
            mutable: mutability == Mutability::Var,
            kind,
            escaping: false,
            span,
        });
        Symbol::variable(name, ty, mutability, span).with_local(local)
    }

    /// Defines `symbol` in the current scope, reporting a duplicate (`E4010`).
    fn define_local(&mut self, symbol: Symbol) {
        if let Err(err) = self.env.define(symbol) {
            self.errors.push(err);
        }
    }

    /// The binding a local symbol resolves to.
    fn local_of(symbol: &Symbol) -> Option<prism_typed_tree::LocalId> {
        match &symbol.kind {
            SymbolKind::Variable { local, .. } => *local,
            _ => None,
        }
    }

    /// Typechecks an individual function declaration body, with its type parameters in scope:
    /// a generic body is checked once, against its type parameters and their bounds alone (D5
    /// of `generics`).
    pub fn check_function_body(&mut self, f: &FunctionDecl) {
        self.with_type_parameters(type_parameters_of(&f.type_parameters), |c| {
            c.check_function_body_in_scope(f)
        });
    }

    fn check_function_body_in_scope(&mut self, f: &FunctionDecl) {
        let receiver_ty = f.receiver.as_ref().map(|r| self.lower_type(&r.ty));
        let context_ty = f
            .context
            .as_ref()
            .map(|c| self.lower_type(&c.ty))
            .filter(|ty| ty != &Type::Error);
        let path = match &receiver_ty {
            Some(receiver_ty) => {
                DeclarationPath::extension(self.module_path.clone(), receiver_ty.clone(), &f.name)
                    .with_context(context_ty.clone())
            }
            None => DeclarationPath::new(self.module_path.clone(), &f.name),
        };
        let previous = self.start_body(path.clone());
        let prev_in_func = self.in_function;
        self.in_function = true;
        self.env.enter_scope();
        self.push_compose_context(f.is_compose);
        // The function's own context: its receiver, then its context parameter (D7 of
        // `receiver-lambdas`), bound below.
        let outer_candidates = std::mem::take(&mut self.context_candidates);

        // A context parameter is the first parameter, a shared borrow of its type, before the
        // receiver (D6 of `receiver-lambdas`). It is no implicit receiver.
        let mut context_candidate = None;
        if let (Some(context), Some(context_ty)) = (&f.context, &context_ty) {
            let sym = self.bind_local(
                &context.name,
                Type::Reference {
                    mutable: false,
                    inner: Box::new(context_ty.clone()),
                },
                Mutability::Val,
                LocalKind::Parameter,
                context.span,
            );
            let local = Self::local_of(&sym).expect("a bound local");
            self.builder.body_mut().parameters.push(local);
            context_candidate = Self::context_candidate(
                &sym,
                ContextSource::ContextParameter(context.name.clone()),
            );
            self.define_local(sym);
        }

        // An extension binds its receiver as the `val` named `this`, before the parameters, so
        // defaults may read it (D4 of `extension-methods`). No user name can collide with it,
        // because `this` is a keyword.
        if let (Some(receiver), Some(receiver_ty)) = (&f.receiver, &receiver_ty) {
            let this_ty = match receiver.mode {
                ReceiverMode::Shared => Type::Reference {
                    mutable: false,
                    inner: Box::new(receiver_ty.clone()),
                },
                ReceiverMode::Mutable => Type::Reference {
                    mutable: true,
                    inner: Box::new(receiver_ty.clone()),
                },
                ReceiverMode::Consuming => receiver_ty.clone(),
            };
            let sym = self.bind_local(
                THIS,
                this_ty,
                Mutability::Val,
                LocalKind::Receiver,
                receiver.span,
            );
            let local = Self::local_of(&sym).expect("a bound local");
            self.builder.body_mut().parameters.push(local);
            self.context_candidates.extend(Self::context_candidate(
                &sym,
                ContextSource::ExtensionReceiver,
            ));
            self.define_local(sym);
        }
        self.context_candidates.extend(context_candidate);

        // Bind parameters in local scope. A default value is checked against its parameter type
        // before the parameter is bound, so it may refer to earlier parameters only.
        let mut parameters = Vec::new();
        for p in &f.params {
            let p_ty = self.lower_type(&p.ty);
            // A default value is evaluated as the call's argument, so it is borrowed like one.
            // Its generator only borrows `this` and the earlier parameters (D11 of
            // `borrowck-moves`).
            let inputs = (
                self.builder.body_mut().parameters.clone(),
                self.builder.closure_depth(),
            );
            let saved_inputs = self.default_inputs.replace(inputs);
            // A default's generator runs in the caller before the call, outside the callee's
            // groups, so it is never a composable context (D17 of `compose-transform`).
            self.push_compose_context(false);
            let saved_in_default = std::mem::replace(&mut self.in_default, true);
            let default = p.default_value.as_ref().map(|default| {
                self.with_consuming(true, |c| c.check_argument(default, &p_ty, Origin::Value))
            });
            self.in_default = saved_in_default;
            self.pop_compose_context();
            self.default_inputs = saved_inputs;
            self.builder.body_mut().defaults.push(default);
            let sym = self.bind_local(
                &p.name,
                p_ty.clone(),
                Mutability::Val,
                LocalKind::Parameter,
                p.span,
            );
            let local = Self::local_of(&sym).expect("a bound local");
            self.builder.body_mut().locals[local].escaping = p.escaping;
            self.builder.body_mut().parameters.push(local);
            self.define_local(sym);
            parameters.push((p.name.clone(), p_ty));
        }

        let declared_ret = f.return_type.as_ref().map(|t| self.lower_type(t)).or({
            if matches!(f.body, Some(FunctionBody::Block(_)) | None) {
                Some(Type::Unit)
            } else {
                None
            }
        });
        let prev_ret = self.current_expected_return.take();
        self.current_expected_return = declared_ret.clone();

        let mut return_type = declared_ret.clone().unwrap_or(Type::Error);
        if let Some(body) = &f.body {
            let value = match body {
                FunctionBody::Block(block) => {
                    // The final expression of a non-`Unit` body is the returned value.
                    let origin = match &declared_ret {
                        Some(Type::Unit) | None => Origin::Value,
                        Some(_) => Origin::Return,
                    };
                    let block_id = self.with_origin(origin, |c| {
                        c.with_consuming(true, |c| c.check_block(block, declared_ret.as_ref()))
                    });
                    let block_ty = self.ty(block_id);
                    if let Some(expected) = &declared_ret {
                        // A `Unit` block body discards its final value (see `check_block`).
                        if expected != &Type::Unit && !block_ty.is_assignable_to(expected) {
                            self.errors.push(TypeError::ReturnTypeMismatch {
                                expected: expected.clone(),
                                found: block_ty,
                                span: block.span,
                                help: None,
                            });
                        }
                    }
                    block_id
                }
                FunctionBody::Expr(expr) => {
                    let value = if let Some(expected) = &declared_ret {
                        self.check_expr_at(expr, expected, Origin::Return)
                    } else {
                        self.with_consuming(true, |c| c.infer_expr(expr))
                    };

                    if declared_ret.is_none() {
                        let expr_ty = self.ty(value);
                        return_type = expr_ty.clone();
                        let inferred = Type::Fn {
                            is_compose: f.is_compose,
                            receiver: false,
                            params: f.params.iter().map(|p| self.lower_type(&p.ty)).collect(),
                            escaping: f.params.iter().map(|p| p.escaping).collect(),
                            ret: Box::new(expr_ty),
                        };
                        match &receiver_ty {
                            Some(receiver_ty) => {
                                let key = (ReceiverKey::of(receiver_ty), f.name.clone());
                                let context = f.context.as_ref().map(|c| self.lower_type(&c.ty));
                                if let Some(extension) = self
                                    .own_extensions
                                    .get_mut(&key)
                                    .and_then(|all| all.of_context_mut(&context))
                                {
                                    if let SymbolKind::Function { fn_type, .. } =
                                        &mut extension.symbol.kind
                                    {
                                        *fn_type = inferred;
                                    }
                                }
                            }
                            None => {
                                self.env.update_type(&f.name, inferred);
                            }
                        }
                    }
                    value
                }
            };
            self.builder.body_mut().value = Some(value);
        }

        self.current_expected_return = prev_ret;
        self.pop_compose_context();
        self.env.exit_scope();
        self.in_function = prev_in_func;
        self.context_candidates = outer_candidates;
        let receiver = f
            .receiver
            .as_ref()
            .zip(receiver_ty)
            .map(|(receiver, ty)| (receiver.mode, ty));
        let type_parameters = self.type_parameter_scope.clone();
        self.finish_body(previous, BodyKind::Function, f.span, |body| {
            Item::Function(FunctionItem {
                path,
                visibility: f.visibility,
                is_compose: f.is_compose,
                type_parameters,
                receiver,
                context: f
                    .context
                    .as_ref()
                    .zip(context_ty)
                    .map(|(c, ty)| Box::new((c.name.clone(), ty))),
                parameters,
                return_type,
                body,
                span: f.span,
            })
        });
    }

    /// Runs `f` with the expectation origin set to `origin`, restoring it afterwards.
    fn with_origin<T>(&mut self, origin: Origin, f: impl FnOnce(&mut Self) -> T) -> T {
        let prev = std::mem::replace(&mut self.origin, origin);
        let result = f(self);
        self.origin = prev;
        result
    }

    /// Checks `expr` against `expected` with the expectation origin `origin`.
    fn check_expr_at(&mut self, expr: &Expr, expected: &Type, origin: Origin) -> ExpressionId {
        // A returned value and an argument are consumed.
        let consuming = origin != Origin::Value || self.consuming;
        self.with_origin(origin, |checker| {
            checker.consuming = consuming;
            checker.check_expr(expr, expected)
        })
    }

    /// Checks a call argument (or a default value, which becomes one) against its parameter type.
    /// A by-value parameter consumes the argument. A reference parameter borrows it implicitly
    /// when it is not a reference already (D1 and D2 of `reference-access`): a place is borrowed
    /// where it lives and is not consumed, and anything else is evaluated into a consumed
    /// temporary. The argument is inferred rather than checked, because it may already be a
    /// reference (`greet(longest(&a, &b))`) or be a value to borrow.
    fn check_argument(&mut self, expr: &Expr, param_ty: &Type, origin: Origin) -> ExpressionId {
        if !is_reference_parameter(param_ty) {
            return self.check_expr_at(expr, param_ty, origin);
        }
        let is_place = self.is_place(expr);
        // A generic call passed to a borrow infers its type arguments from the referent.
        let referent = referent_of_reference_parameter(param_ty);
        let found = self.with_origin(Origin::Value, |c| {
            // A closure lent to a borrowed function type takes its parameter types from it.
            if matches!(expr.kind, ExprKind::Closure { .. }) && matches!(referent, Type::Fn { .. })
            {
                return c.with_consuming(true, |c| c.check_expr(expr, &referent));
            }
            c.with_consuming(!is_place, |c| c.infer_expr_expecting(expr, Some(&referent)))
        });
        self.with_origin(origin, |c| {
            c.fit(expr, found, param_ty, Site::Argument { is_place }, false)
        })
    }

    /// Whether `expr` is a place, which a borrow refers to where it lives: a variable (local,
    /// parameter or top-level `val`) or a `.` field access from any root. Anything else (a
    /// literal, call, safe navigation, branch, block, `?:` or a string's character `s[i]`, which
    /// is computed) is a temporary (D2 of `reference-access`, D5 of
    /// `runtime-strings-and-drops`).
    fn is_place(&self, expr: &Expr) -> bool {
        if let Some(field) = self.implicit_this_field(expr) {
            return self.is_place(&field);
        }
        match &expr.kind {
            ExprKind::This => self.env.lookup(THIS).is_some(),
            ExprKind::Ident(name) => {
                matches!(
                    self.env.lookup(name).map(|sym| &sym.kind),
                    Some(SymbolKind::Variable { .. })
                ) && !self.variant_owners.contains_key(name)
            }
            ExprKind::MemberAccess {
                target,
                is_safe_nav: false,
                ..
            } => self.type_name_symbol(target).is_none(),
            // An element of a list, from any root, as a field path is (D7 of `collections`); a
            // string's character is a value, which reading in place makes a temporary.
            ExprKind::Index { .. } => true,
            _ => false,
        }
    }

    /// Runs `f` on an expression in a consuming position (`consuming` is true) or a reading one.
    fn with_consuming<T>(&mut self, consuming: bool, f: impl FnOnce(&mut Self) -> T) -> T {
        self.consuming = consuming;
        let result = f(self);
        self.consuming = false;
        result
    }

    /// Infers a value that is read where it is rather than consumed: an operand, a `when`
    /// subject or the target of a member access. A place or a field access, `?.` included, is
    /// read in place (a field's own target is read the same way); anything else is a temporary,
    /// so its branches, block tail and `?:` operands are consuming (the `ownership` spec,
    /// "Consuming Positions"), as for a borrowed argument that is not a place.
    fn infer_read(&mut self, expr: &Expr) -> ExpressionId {
        let read_in_place = self.is_place(expr)
            || self.implicit_this_field(expr).is_some()
            || matches!(&expr.kind, ExprKind::MemberAccess { target, .. }
                if self.type_name_symbol(target).is_none());
        self.with_consuming(!read_in_place, |c| c.infer_expr(expr))
    }

    /// Infers an operator operand or a condition. Operators read through references (D1 of
    /// `typechecker-soundness`) and boxes (D4 of `heap-boxes`), so every `&`, `&mut` and
    /// `Heap<T>` layer is peeled: with `a: &Int` or `a: Heap<Int>`, `a + 1` is an `Int`. Each
    /// peeled layer is an explicit dereference, and the operand is read in place (D5 of
    /// `typed-tree`).
    fn infer_operand(&mut self, expr: &Expr) -> ExpressionId {
        let id = self.with_origin(Origin::Value, |c| c.infer_read(expr));
        self.peel_indirection_nodes(id)
    }

    /// Checks an operator operand or a condition against `expected`, reading through references
    /// like [`Self::infer_operand`]. A mismatch is a plain `E4002`.
    fn check_operand(&mut self, expr: &Expr, expected: &Type) -> ExpressionId {
        let id = self.infer_operand(expr);
        let ty = self.ty(id);
        if ty.is_assignable_to(expected) {
            return self.coerce_to(id, expected);
        }
        if ty != Type::Error {
            self.with_origin(Origin::Value, |c| {
                c.report_mismatch(expected, ty, expr.span)
            });
        }
        self.wrap_error(id)
    }

    /// Reports that `found` is not assignable to `expected`, with the code of the current
    /// expectation origin.
    fn report_mismatch(&mut self, expected: &Type, found: Type, span: Span) {
        self.report_mismatch_with_help(expected, found, span, None);
    }

    /// Like [`Self::report_mismatch`], with a fix to suggest.
    fn report_mismatch_with_help(
        &mut self,
        expected: &Type,
        found: Type,
        span: Span,
        help: Option<String>,
    ) {
        let expected = expected.clone();
        self.errors.push(match self.origin {
            Origin::Value => TypeError::TypeMismatch {
                expected,
                found,
                span,
                help: help.clone(),
            },
            Origin::Return => TypeError::ReturnTypeMismatch {
                expected,
                found,
                span,
                help: help.clone(),
            },
            Origin::Argument => TypeError::ArgumentTypeMismatch {
                expected,
                found,
                span,
                help: help.clone(),
            },
        });
    }

    /// Checks an expression against an expected type in bidirectional checking mode, returning
    /// its node.
    pub fn check_expr(&mut self, expr: &Expr, expected: &Type) -> ExpressionId {
        if let Some(field) = self.implicit_this_field(expr) {
            return self.check_expr(&field, expected);
        }
        let consuming = std::mem::replace(&mut self.consuming, false);
        let saved = std::mem::replace(&mut self.parent_consuming, consuming);
        let id = self.check_expr_kind(expr, expected);
        self.parent_consuming = saved;
        id
    }

    fn check_expr_kind(&mut self, expr: &Expr, expected: &Type) -> ExpressionId {
        if expected == &Type::Error {
            // Nothing is checked against the error type; the node keeps no child.
            return self.error_node(Vec::new(), expr.span);
        }

        // Special handling for null literal
        if let ExprKind::Literal(Literal::Null) = &expr.kind {
            let null = self.add(
                ExpressionKind::Literal(Literal::Null),
                Type::Nullable(Box::new(Type::Nothing)),
                expr.span,
            );
            if let Type::Nullable(_) = expected {
                return self.coerce_to(null, expected);
            } else {
                self.report_mismatch(expected, Type::Nullable(Box::new(Type::Nothing)), expr.span);
                return self.wrap_error(null);
            }
        }

        // Special handling for closures checked against expected function type, also inside a
        // nullable one (`val f: ((Int) -> Int)? = { it + 1 }`), where the closure is checked
        // against the function type and then made nullable.
        if let ExprKind::Closure {
            params,
            arrow,
            body,
        } = &expr.kind
        {
            let (function_type, nullable) = match expected {
                Type::Nullable(inner) => (inner.as_ref(), true),
                other => (other, false),
            };
            if let Type::Fn {
                is_compose,
                receiver: expected_receiver,
                params: exp_params,
                escaping: exp_escaping,
                ret: exp_ret,
            } = function_type
            {
                let closure = self.check_closure(
                    params,
                    arrow.is_some(),
                    body,
                    *is_compose,
                    *expected_receiver,
                    exp_params,
                    exp_escaping,
                    ClosureReturn::Expected(exp_ret),
                    expr.span,
                );
                return if nullable {
                    self.coerce_to(closure, expected)
                } else {
                    closure
                };
            }
        }

        // Special handling for if/else in check mode: each branch produces the checked value.
        if let ExprKind::If {
            cond,
            then_branch,
            else_branch,
        } = &expr.kind
        {
            let condition = self.check_operand(cond, &Type::Bool);
            let consuming = self.parent_consuming;
            let mut then_id =
                self.with_consuming(consuming, |c| c.check_expr(then_branch, expected));
            if else_branch.is_none() {
                then_id = self.discard_branch(then_id);
            }
            let else_id = else_branch
                .as_ref()
                .map(|else_b| self.with_consuming(consuming, |c| c.check_expr(else_b, expected)));
            let kind = ExpressionKind::If {
                condition,
                then_branch: then_id,
                else_branch: else_id,
            };
            if else_id.is_some() || expected == &Type::Unit {
                // Each branch fits the expected type, or reported why not.
                let ty = if else_id.is_some() {
                    expected.clone()
                } else {
                    Type::Unit
                };
                return self.add(kind, ty, expr.span);
            }
            // Without `else`, the `if` is a `Unit`, which must fit the expected type. Its
            // branch is checked once, above (D1 of `typed-tree`).
            let if_id = self.add(kind, Type::Unit, expr.span);
            return self.fit(expr, if_id, expected, Site::Value, false);
        }

        // Special handling for when in check mode: each branch produces the checked value
        if let ExprKind::When { target, branches } = &expr.kind {
            return self.infer_when_expr(
                target.as_deref(),
                branches,
                WhenUse::Check(expected),
                expr.span,
            );
        }

        // A payload-free variant of a generic enum takes its type arguments from the expected
        // type (D8 of `generics`).
        if let Some((id, member)) = self.generic_unit_variant(expr) {
            return self.check_generic_unit_variant(expr, &id, &member, expected);
        }

        // A call is inferred knowing its expected type, from which a generic callee infers
        // type arguments (D8 of `generics`), then fitted like any other value. So is `?:`,
        // whose fallback may be such a call.
        if let ExprKind::Call { .. }
        | ExprKind::Binary {
            op: BinaryOp::Elvis,
            ..
        } = &expr.kind
        {
            let consuming = self.parent_consuming;
            let inferred = self.with_origin(Origin::Value, |c| {
                c.with_consuming(consuming, |c| c.infer_expr_expecting(expr, Some(expected)))
            });
            return self.fit(expr, inferred, expected, Site::Value, false);
        }

        // Default: infer, then fit the value to the expected type (copying out of a borrow if
        // needed). Expectations nested inside the inferred expression have their own origin. A
        // leaf's move check waits for the fit, which decides whether anything moves.
        let consuming = self.parent_consuming;
        let deferred_move = consuming && is_move_leaf(expr);
        let inferred = self.with_origin(Origin::Value, |c| {
            c.with_consuming(consuming && !deferred_move, |c| c.infer_expr(expr))
        });
        self.fit(expr, inferred, expected, Site::Value, deferred_move)
    }

    /// Checks the closure `{ params -> body }` against a function type with parameter types
    /// `exp_params`, whose `escaping` flags `exp_escaping` its parameters take, and, when known,
    /// return type `exp_ret`; without one the body's type is the closure's return type, or its
    /// referent for [`ClosureReturn::InferredReferent`]. A closure
    /// without a header (`has_header` false) expecting one parameter gets the implicit parameter
    /// `it` (D1 of `implicit-it`), decided here from the expected type alone.
    #[allow(clippy::too_many_arguments)]
    fn check_closure(
        &mut self,
        params: &[prism_ast::ClosureParameter],
        has_header: bool,
        body: &Block,
        is_compose: bool,
        receiver: bool,
        exp_params: &[Type],
        exp_escaping: &[bool],
        exp_ret: ClosureReturn,
        span: Span,
    ) -> ExpressionId {
        // A lambda with a receiver binds `this` before its other parameters, which alone its
        // header declares and `it` stands for (D3 of `receiver-lambdas`).
        let others = if receiver && !exp_params.is_empty() {
            &exp_params[1..]
        } else {
            exp_params
        };
        let skipped = exp_params.len() - others.len();
        let implicit = !has_header && others.len() == 1;
        if !implicit && params.len() != others.len() {
            self.errors.push(TypeError::ClosureParameterCount {
                expected: others.to_vec(),
                declared: params.len(),
                has_header,
                span,
            });
            return self.error_node(Vec::new(), span);
        }

        // The implicit `it` and the receiver are declared at the closure's `{` (D5 of
        // `implicit-it`).
        let brace = Span::new(span.start, span.start + 1, span.line, span.column);
        let mut declared: Vec<(&str, Span, LocalKind)> = Vec::new();
        if skipped == 1 {
            declared.push((THIS, brace, LocalKind::ClosureReceiver));
        }
        declared.extend(if implicit {
            vec![(
                IMPLICIT_PARAMETER,
                brace,
                LocalKind::ImplicitClosureParameter,
            )]
        } else {
            params
                .iter()
                .map(|p| (p.name.as_str(), p.span, LocalKind::ClosureParameter))
                .collect()
        });
        let missing = if implicit {
            None
        } else if has_header {
            Some(MissingImplicitParameter::DeclaredParameters(
                params.iter().map(|p| p.name.clone()).collect(),
            ))
        } else {
            Some(MissingImplicitParameter::NoExpectedParameters {
                receiver: exp_params[..skipped].first().cloned(),
                function_type: None,
            })
        };
        let expected_return = match exp_ret {
            ClosureReturn::Expected(ret) => Some(ret),
            ClosureReturn::Inferred | ClosureReturn::InferredReferent => None,
        };
        let prev = self.enter_closure(is_compose, expected_return.cloned(), missing);
        let mut parameters = Vec::new();
        let mut receiver_context = None;
        for (index, ((name, parameter_span, kind), param_ty)) in
            declared.into_iter().zip(exp_params.iter()).enumerate()
        {
            let sym = self.bind_local(
                name,
                param_ty.clone(),
                Mutability::Val,
                kind,
                parameter_span,
            );
            let local = Self::local_of(&sym).expect("a bound local");
            self.builder.body_mut().locals[local].escaping =
                exp_escaping.get(index).copied().unwrap_or(false);
            parameters.push(local);
            // A lambda with a receiver is its body's context (D7 of `receiver-lambdas`).
            if kind == LocalKind::ClosureReceiver {
                receiver_context = Self::context_candidate(&sym, ContextSource::LambdaReceiver);
            }
            self.define_local(sym);
        }
        let outer_candidates = match receiver_context {
            Some(candidate) => Some(std::mem::replace(
                &mut self.context_candidates,
                vec![candidate],
            )),
            None => None,
        };

        // The body's value is what the closure returns.
        let body_id = self.with_origin(Origin::Return, |c| {
            c.with_consuming(true, |c| c.check_block(body, expected_return))
        });
        let body_id = match (exp_ret, &body.tail) {
            (ClosureReturn::InferredReferent, Some(tail))
                if matches!(self.ty(body_id), Type::Reference { .. }) =>
            {
                self.copy_result_out_of_borrow(body_id, tail)
            }
            _ => body_id,
        };
        let captures = self.exit_closure(prev);
        if let Some(outer) = outer_candidates {
            self.context_candidates = outer;
        }
        let ty = Type::Fn {
            is_compose,
            receiver,
            params: exp_params.to_vec(),
            escaping: (0..exp_params.len())
                .map(|index| exp_escaping.get(index).copied().unwrap_or(false))
                .collect(),
            ret: Box::new(self.ty(body_id)),
        };
        let kind = ExpressionKind::Closure {
            parameters,
            body: body_id,
            is_compose,
            captures,
            ownership_reason: None,
        };
        self.add(kind, ty, span)
    }

    /// Fits the borrowed tail `tail` of the closure body `block_id` to its referent, copying it
    /// out of its borrows when the referent is `Copy` and reporting a move out of a borrow
    /// (`E4017`) otherwise, as a borrowed argument of a by-value type parameter is (`generics`,
    /// "Type Arguments Are Borrow-Free"). The body is not checked again: only its tail's node is
    /// wrapped. The block takes the referent's type either way, so the type parameter it solves
    /// is the referent and an error is reported once.
    fn copy_result_out_of_borrow(&mut self, block_id: ExpressionId, tail: &Expr) -> ExpressionId {
        let ExpressionKind::Block {
            statements,
            tail: Some(tail_id),
            close_brace,
        } = self.builder.kind(block_id).clone()
        else {
            return block_id;
        };
        let mut referent = self.ty(tail_id);
        while let Type::Reference { inner, .. } = referent {
            referent = *inner;
        }
        let fitted = self.with_origin(Origin::Return, |c| {
            c.fit(tail, tail_id, &referent, Site::Value, false)
        });
        let kind = ExpressionKind::Block {
            statements,
            tail: Some(fitted),
            close_brace,
        };
        let span = self.builder.span_of(block_id);
        self.builder.replace(block_id, kind, referent, span);
        block_id
    }

    /// Enters the body of a closure: a scope, a compose context, a fresh set of open loops (a
    /// jump cannot leave the closure for a loop around it), the expected return type, and why
    /// the closure has no implicit `it` (`None` when it has one). The builder records the
    /// closure's captures (D11 of `typed-tree`). Returns what [`Self::exit_closure`] restores.
    fn enter_closure(
        &mut self,
        is_compose: bool,
        expected_return: Option<Type>,
        missing_implicit_parameter: Option<MissingImplicitParameter>,
    ) -> EnclosingClosureContext {
        self.env.enter_scope();
        self.push_compose_context(is_compose);
        let saved = EnclosingClosureContext {
            in_function: std::mem::replace(&mut self.in_function, true),
            loops: std::mem::take(&mut self.builder.loops),
            expected_return: std::mem::replace(&mut self.current_expected_return, expected_return),
            missing_implicit_parameter: std::mem::replace(
                &mut self.missing_implicit_parameter,
                missing_implicit_parameter,
            ),
        };
        self.builder.open_closure();
        saved
    }

    /// Leaves a closure body entered by [`Self::enter_closure`], returning its captures.
    fn exit_closure(&mut self, saved: EnclosingClosureContext) -> Vec<prism_typed_tree::Capture> {
        let captures = self.builder.close_closure();
        self.builder.loops = saved.loops;
        self.current_expected_return = saved.expected_return;
        self.missing_implicit_parameter = saved.missing_implicit_parameter;
        self.in_function = saved.in_function;
        self.pop_compose_context();
        self.env.exit_scope();
        captures
    }

    /// Types an identifier read as a value.
    fn infer_ident(&mut self, name: &str, span: Span) -> ExpressionId {
        match self.resolve_ident(name, span) {
            Some(sym) => self.symbol_node(&sym, name, span),
            None => self.error_node(Vec::new(), span),
        }
    }

    /// The node of a name that resolved to `sym` (D4 of `typed-tree`): a binding of the body, a
    /// top-level value or function by its path, or a variant of an enum.
    fn symbol_node(&mut self, sym: &Symbol, name: &str, span: Span) -> ExpressionId {
        let generic_callee = std::mem::take(&mut self.generic_callee);
        let intrinsic_call = std::mem::take(&mut self.intrinsic_call);
        // An intrinsic has no code to name: its calls are lowered where they are written (D6
        // of `standard-library`).
        if let SymbolKind::Function { module, .. } = &sym.kind {
            if self.intrinsic_function(module, name).is_some() && !intrinsic_call {
                self.errors.push(TypeError::CannotInferType {
                    help: Some(format!(
                        "call it, or wrap the call in a closure, as in '{{ x -> {}(x) }}'",
                        name
                    )),
                    message: format!(
                        "the intrinsic '{}' of '/{}' cannot be used as a value: it can only be called",
                        name, module
                    ),
                    span,
                });
                return self.error_node(Vec::new(), span);
            }
        }
        if !generic_callee && !sym.type_parameters().is_empty() {
            // A generic function or variant constructor named outside a call: nothing gives its
            // type arguments (D8 of `generics`).
            self.errors.push(TypeError::CannotInferType {
                help: Some(format!(
                    "call it, or wrap the call in a closure, as in '{{ x -> {}(x) }}'",
                    name
                )),
                message: format!(
                    "cannot infer the type arguments of the generic '{}' used as a value",
                    name
                ),
                span,
            });
            return self.error_node(Vec::new(), span);
        }
        match &sym.kind {
            SymbolKind::Struct { .. } | SymbolKind::Enum { .. } => {
                self.errors.push(TypeError::TypeNameAsValue {
                    name: name.to_string(),
                    span,
                });
                self.error_node(Vec::new(), span)
            }
            SymbolKind::Variable {
                ty,
                local: Some(local),
                ..
            } => {
                self.builder.use_local(*local, span);
                self.add(ExpressionKind::Local(*local), ty.clone(), span)
            }
            SymbolKind::Variable { ty, module, .. } => {
                if self.variant_owners.contains_key(name) {
                    if ty.mentions_parameter() {
                        return self.uninferred_unit_variant(name, span);
                    }
                    return self.variant_node(ty, name, ty.clone(), span);
                }
                let module = module.clone().unwrap_or_else(|| self.module_path.clone());
                let path = DeclarationPath::new(module, name);
                self.add(ExpressionKind::TopLevelValue(path), ty.clone(), span)
            }
            SymbolKind::Function {
                context: Some(context),
                ..
            } if !generic_callee => {
                // Its value would need its context (D10 of `receiver-lambdas`).
                self.errors.push(TypeError::MissingContext {
                    function: name.to_string(),
                    needed: vec![context.clone()],
                    found: "a function with a context parameter can only be called, not used as a value".to_string(),
                    span,
                });
                self.error_node(Vec::new(), span)
            }
            SymbolKind::Function {
                fn_type, module, ..
            } => {
                if self.variant_owners.contains_key(name) {
                    if let Type::Fn { ret, .. } = fn_type {
                        return self.variant_node(ret, name, fn_type.clone(), span);
                    }
                }
                let path = DeclarationPath::new(module.clone(), name);
                self.add(
                    ExpressionKind::FunctionReference(path),
                    fn_type.clone(),
                    span,
                )
            }
        }
    }

    /// Infers the type of an expression in inference mode, returning its node. A consumed place
    /// is wrapped in a move or a copy (D5 of `typed-tree`).
    pub fn infer_expr(&mut self, expr: &Expr) -> ExpressionId {
        self.infer_expr_expecting(expr, None)
    }

    /// Infers `expr` as [`Self::infer_expr`] does. A call uses `expected`, the type it is checked
    /// against, to infer the type arguments of a generic callee (D8 of `generics`); any other
    /// expression ignores it.
    fn infer_expr_expecting(&mut self, expr: &Expr, expected: Option<&Type>) -> ExpressionId {
        if let Some(field) = self.implicit_this_field(expr) {
            return self.infer_expr(&field);
        }
        let consuming = std::mem::replace(&mut self.consuming, false);
        let saved = std::mem::replace(&mut self.parent_consuming, consuming);
        let id = self.infer_expr_kind(expr, expected);
        self.parent_consuming = saved;
        if consuming {
            let ty = self.ty(id);
            self.check_move_source(expr, &ty);
            return self.consume(id);
        }
        id
    }

    fn infer_expr_kind(&mut self, expr: &Expr, expected: Option<&Type>) -> ExpressionId {
        match &expr.kind {
            ExprKind::Literal(lit) => {
                let ty = match lit {
                    Literal::Int(_) => Type::Int,
                    Literal::Float(_) => Type::Float,
                    Literal::String(_) => Type::String,
                    Literal::Char(_) => Type::Char,
                    Literal::Bool(_) => Type::Bool,
                    Literal::Null => Type::Nullable(Box::new(Type::Nothing)),
                };
                self.add(ExpressionKind::Literal(lit.clone()), ty, expr.span)
            }

            ExprKind::Ident(name) => {
                self.ensure_inferred(name, expr.span);
                self.infer_ident(name, expr.span)
            }

            ExprKind::This => match self.env.lookup(THIS).cloned() {
                Some(sym) => self.symbol_node(&sym, THIS, expr.span),
                None => {
                    self.errors
                        .push(TypeError::ThisOutsideExtension { span: expr.span });
                    self.error_node(Vec::new(), expr.span)
                }
            },

            ExprKind::Binary { op, left, right } => {
                self.infer_binary_op(*op, left, right, expr.span, expected)
            }

            ExprKind::Unary { op, expr: inner } => self.infer_unary_op(*op, inner, expr.span),

            ExprKind::Block(block) => {
                let consuming = self.parent_consuming;
                self.with_consuming(consuming, |c| c.check_block(block, None))
            }

            ExprKind::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let condition = self.check_operand(cond, &Type::Bool);
                let consuming = self.parent_consuming;
                let then_id = self.with_consuming(consuming, |c| c.infer_expr(then_branch));
                let Some(else_b) = else_branch else {
                    let then_id = self.discard_branch(then_id);
                    let kind = ExpressionKind::If {
                        condition,
                        then_branch: then_id,
                        else_branch: None,
                    };
                    return self.add(kind, Type::Unit, expr.span);
                };
                let else_id = self.with_consuming(consuming, |c| c.infer_expr(else_b));
                let (then_ty, else_ty) = (self.ty(then_id), self.ty(else_id));
                let (then_id, else_id, ty) = if let Some(joined) = then_ty.join(&else_ty) {
                    // Each branch is coerced to the join (D6 of `typed-tree`).
                    let then_id = self.coerce_to(then_id, &joined);
                    let else_id = self.coerce_to(else_id, &joined);
                    (then_id, else_id, joined)
                } else {
                    self.errors.push(TypeError::TypeMismatch {
                        expected: then_ty,
                        found: else_ty,
                        span: expr.span,
                        help: None,
                    });
                    (then_id, else_id, Type::Error)
                };
                let kind = ExpressionKind::If {
                    condition,
                    then_branch: then_id,
                    else_branch: Some(else_id),
                };
                self.add(kind, ty, expr.span)
            }

            ExprKind::When { target, branches } => {
                self.infer_when_expr(target.as_deref(), branches, WhenUse::Infer, expr.span)
            }

            ExprKind::Call {
                callee,
                type_arguments,
                args,
            } => self.infer_call_expr(callee, type_arguments, args, expr.span, expected.cloned()),

            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav,
            } => self.infer_member_access(target, member, *is_safe_nav, expr.span),

            ExprKind::Index { target, index } => self.infer_index(target, index, expr.span).0,

            ExprKind::Return(opt_expr) => {
                if !self.in_function {
                    self.errors.push(TypeError::ReturnTypeMismatch {
                        expected: Type::Unit,
                        found: Type::Error,
                        span: expr.span,
                        help: None,
                    });
                    return self.error_node(Vec::new(), expr.span);
                }
                let expected = self.current_expected_return.clone();
                let value = if let Some(e) = opt_expr {
                    Some(if let Some(exp) = &expected {
                        self.check_expr_at(e, exp, Origin::Return)
                    } else {
                        self.with_consuming(true, |c| c.infer_expr(e))
                    })
                } else {
                    if let Some(exp) = &expected {
                        if exp != &Type::Unit && exp != &Type::Error {
                            self.errors.push(TypeError::ReturnTypeMismatch {
                                expected: exp.clone(),
                                found: Type::Unit,
                                span: expr.span,
                                help: None,
                            });
                        }
                    }
                    None
                };
                self.add(ExpressionKind::Return { value }, Type::Nothing, expr.span)
            }

            ExprKind::Break | ExprKind::Continue => {
                // A jump names the innermost loop around it (D9 of `typed-tree`).
                let target = self.builder.loops.last().copied();
                if target.is_none() {
                    let keyword = match expr.kind {
                        ExprKind::Break => "break",
                        _ => "continue",
                    };
                    self.errors.push(TypeError::BreakOutsideLoop {
                        keyword,
                        span: expr.span,
                    });
                }
                let kind = match expr.kind {
                    ExprKind::Break => ExpressionKind::Break { target },
                    _ => ExpressionKind::Continue { target },
                };
                self.add(kind, Type::Nothing, expr.span)
            }

            ExprKind::Template(parts) => self.infer_template(parts, expr.span),

            ExprKind::Closure {
                params,
                arrow,
                body,
            } => {
                let missing = if arrow.is_some() {
                    MissingImplicitParameter::DeclaredParameters(
                        params.iter().map(|p| p.name.clone()).collect(),
                    )
                } else {
                    MissingImplicitParameter::NoExpectedType
                };
                let prev = self.enter_closure(false, None, Some(missing));

                // Without an expected function type, nothing gives the parameters a type (D6 of
                // `typechecker-soundness`). They are bound to the error type so the body is still
                // checked for other errors.
                let mut param_types = Vec::new();
                let mut parameters = Vec::new();
                for parameter in params {
                    let name = &parameter.name;
                    let sym = self.bind_local(
                        name,
                        Type::Error,
                        Mutability::Val,
                        LocalKind::ClosureParameter,
                        parameter.span,
                    );
                    parameters.push(Self::local_of(&sym).expect("a bound local"));
                    match self.env.define(sym) {
                        Ok(()) => self.errors.push(TypeError::CannotInferType {
                            help: None,
                            message: format!(
                                "cannot infer the type of closure parameter '{}'; pass the closure where a function type is expected",
                                name
                            ),
                            span: expr.span,
                        }),
                        Err(err) => self.errors.push(err),
                    }
                    param_types.push(Type::Error);
                }
                // The body's value is what the closure returns.
                let body_id = self.with_consuming(true, |c| c.check_block(body, None));
                let captures = self.exit_closure(prev);

                let ty = Type::function(false, param_types, self.ty(body_id));
                let kind = ExpressionKind::Closure {
                    parameters,
                    body: body_id,
                    is_compose: false,
                    captures,
                    ownership_reason: None,
                };
                self.add(kind, ty, expr.span)
            }
        }
    }

    /// Types a string template (D6 to D9 of `string-templates`): a `String` holding its text runs
    /// and the text form of each part, in source order.
    fn infer_template(&mut self, parts: &[TemplatePart], span: Span) -> ExpressionId {
        let mut typed = Vec::with_capacity(parts.len());
        for (index, part) in parts.iter().enumerate() {
            match part {
                TemplatePart::Text(text) => typed.push(TreeTemplatePart::Text(text.clone())),
                TemplatePart::Expression(expr) => {
                    let following = match parts.get(index + 1) {
                        Some(TemplatePart::Text(text)) => text.as_str(),
                        _ => "",
                    };
                    let value = self.check_template_part(expr, following);
                    typed.push(TreeTemplatePart::Value(value));
                }
            }
        }
        self.add(
            ExpressionKind::Template { parts: typed },
            Type::String,
            span,
        )
    }

    /// Checks one part of a string template, with no expected type, and gives the node of its
    /// text form, read in place (D6 and D9 of `string-templates`). The part is read like an
    /// operator's operand, through its `&`, `&mut` and `Heap<T>` layers: a `String` is a place
    /// (a computed one a temporary), a scalar is used as it is, and any other type is the result
    /// of its `toString()`, resolved as the method call `part.toString()` is, in a temporary.
    /// `following` is the text after the part, for the help of `$u.name`.
    fn check_template_part(&mut self, part: &Expr, following: &str) -> ExpressionId {
        let is_place = self.is_place(part) || self.is_optional_place(part);
        let id = self.with_origin(Origin::Value, |c| c.infer_read(part));
        self.check_text_form(part, id, is_place, TextFormUse::Template { following })
    }

    /// The node of the text form of `part`, whose value read in place is the node `id`: the
    /// rule of template parts (D6 and D9 of `string-templates`), which the arguments of `print`
    /// and `println` follow too (D9 of `standard-library`). A `String` is a place, a scalar is
    /// used as it is, and any other type is the result of its `toString()`, resolved as the
    /// method call `part.toString()` is, in a temporary. A type without a text form is
    /// `E4026`, with the help of `context`.
    fn check_text_form(
        &mut self,
        part: &Expr,
        id: ExpressionId,
        is_place: bool,
        context: TextFormUse,
    ) -> ExpressionId {
        let ty = self.ty(id);
        let reason = match peel_indirection(&ty) {
            Type::Error => return id,
            Type::String => {
                let read = self.peel_indirection_nodes(id);
                return self.as_place(read);
            }
            Type::Int | Type::Float | Type::Bool | Type::Char => {
                return self.peel_indirection_nodes(id);
            }
            Type::Nothing => {
                let value = self.coerce_to(id, &Type::String);
                return self.as_place(value);
            }
            Type::Nullable(_) => Some(NoTextFormReason::Nullable),
            Type::Unit | Type::Fn { .. } | Type::Parameter(_) | Type::Opaque(_) => {
                Some(NoTextFormReason::NoToString)
            }
            _ => self.template_to_string_problem(&ty, part.span),
        };
        if let Some(reason) = reason {
            let referent = peel_indirection(&ty).clone();
            let help = template_part_help(part, &referent, &reason, context);
            self.errors.push(TypeError::NoTextForm {
                ty: referent,
                reason,
                help,
                printed: matches!(context, TextFormUse::Printed),
                span: part.span,
            });
            return self.wrap_error(id);
        }
        // The conversion is exactly the explicit call `part.toString()`.
        let callee = Expr::new(
            ExprKind::MemberAccess {
                target: Box::new(part.clone()),
                member: TO_STRING.to_string(),
                is_safe_nav: false,
            },
            part.span,
        );
        let call = MethodCall {
            callee: &callee,
            target: part,
            member: TO_STRING,
            is_safe_nav: false,
            type_arguments: &[],
            args: &[],
            expected: None,
        };
        let text = self.method_call_on(call, id, is_place, false, part.span);
        self.as_place(text)
    }

    /// Why the `toString()` that `part.toString()` resolves to cannot give a part of type `ty`
    /// its text form, or `None` when it can: it must exist, be callable without arguments,
    /// return `String` and, for an extension, take a shared receiver (D6 of `string-templates`).
    /// An ambiguous import is left to the call, which reports it (`E4012`).
    fn template_to_string_problem(&mut self, ty: &Type, span: Span) -> Option<NoTextFormReason> {
        let signature = if self.has_field(ty, TO_STRING) {
            let field = match peel_indirection(ty) {
                Type::Struct { id, arguments } => self.struct_fields(id).and_then(|fields| {
                    fields
                        .into_iter()
                        .find(|(name, _, _)| name == TO_STRING)
                        .map(|(_, _, field)| field.substitute(arguments))
                }),
                _ => None,
            };
            match field {
                Some(Type::Fn { params, ret, .. }) => (params.len(), Vec::new(), *ret),
                // A field of another type cannot be called: the call reports it.
                _ => return None,
            }
        } else {
            let mut layers = Vec::new();
            let (lookup, reached) =
                self.find_extension_through_indirection(ty.clone(), TO_STRING, &mut layers);
            let extension = match lookup {
                ExtensionLookup::None => return Some(NoTextFormReason::NoToString),
                // The call of the text form reports these.
                ExtensionLookup::Ambiguous(_) | ExtensionLookup::MissingContext(_) => return None,
                ExtensionLookup::Found { extension, own } => {
                    if own {
                        let key = InferenceKey::Extension {
                            receiver: ReceiverKey::of(&reached),
                            name: TO_STRING.to_string(),
                            context: extension.context.clone(),
                        };
                        self.ensure_key_inferred(&key, span);
                    }
                    match self.find_extension(&reached, TO_STRING) {
                        ExtensionLookup::Found { extension, .. } => extension,
                        _ => extension,
                    }
                }
            };
            match extension.mode {
                ReceiverMode::Shared => {}
                ReceiverMode::Mutable => {
                    return Some(NoTextFormReason::ToStringReceiver { mutable: true })
                }
                ReceiverMode::Consuming => {
                    return Some(NoTextFormReason::ToStringReceiver { mutable: false })
                }
            }
            match &extension.symbol.kind {
                SymbolKind::Function {
                    fn_type: Type::Fn { params, ret, .. },
                    has_default,
                    ..
                } => (params.len(), has_default.clone(), (**ret).clone()),
                _ => return None,
            }
        };
        let (parameters, has_default, returned) = signature;
        let required =
            (0..parameters).any(|index| !has_default.get(index).copied().unwrap_or(false));
        if required {
            Some(NoTextFormReason::ToStringNeedsArguments)
        } else if returned != Type::String && returned != Type::Error {
            Some(NoTextFormReason::ToStringReturns(returned))
        } else {
            None
        }
    }

    /// Infers the binary operation `left op right`. `expected`, the type the operation is checked
    /// against, if any, is the expected type of the fallback of `?:`, which a generic call there
    /// infers its type arguments from (D8 of `generics`).
    fn infer_binary_op(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        span: Span,
        expected: Option<&Type>,
    ) -> ExpressionId {
        let binary = |checker: &mut Self, l: ExpressionId, r: ExpressionId, ty: Type| {
            let kind = ExpressionKind::Binary {
                operator: binary_operator(op),
                left: l,
                right: r,
            };
            checker.add(kind, ty, span)
        };
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                let left_id = self.infer_operand(left);
                let right_id = self.infer_operand(right);
                let (left_ty, right_ty) = (self.ty(left_id), self.ty(right_id));

                let ty = if left_ty == Type::Error || right_ty == Type::Error {
                    Type::Error
                } else if let Some(ty) = arithmetic_result(op, &left_ty, &right_ty) {
                    ty
                } else if let Some(help) = type_parameter_operator_help(op, &left_ty, &right_ty) {
                    // The operator, not an operand, does not apply (D5 of `generics`).
                    self.errors.push(TypeError::OperatorMismatch {
                        operator: operator_text(op).to_string(),
                        left: left_ty,
                        right: right_ty,
                        span,
                        help: Some(help),
                    });
                    Type::Error
                } else {
                    // Strict typing: no implicit widening (e.g. Int + Float rejected)
                    self.errors.push(TypeError::TypeMismatch {
                        expected: left_ty,
                        found: right_ty,
                        span,
                        help: None,
                    });
                    Type::Error
                };
                binary(self, left_id, right_id, ty)
            }

            BinaryOp::Eq | BinaryOp::NotEq => {
                let left_id = self.infer_operand(left);
                let right_id = self.infer_operand(right);
                let (left_ty, right_ty) = (self.ty(left_id), self.ty(right_id));

                if left_ty == Type::Error || right_ty == Type::Error {
                    return binary(self, left_id, right_id, Type::Bool);
                }

                // A type parameter has no equality until traits exist (D5 of `generics`); only
                // a comparison with `null` reads a `T?`.
                let null = Type::Nothing.nullable();
                let compares_parameters = left_ty.mentions_parameter_outside_functions()
                    || right_ty.mentions_parameter_outside_functions();
                if compares_parameters && left_ty != null && right_ty != null {
                    let help = type_parameter_operator_help(op, &left_ty, &right_ty);
                    self.errors.push(TypeError::OperatorMismatch {
                        operator: operator_text(op).to_string(),
                        left: left_ty,
                        right: right_ty,
                        span,
                        help,
                    });
                    return binary(self, left_id, right_id, Type::Error);
                }

                if right_ty.is_assignable_to(&left_ty) {
                    // Both operands are compared at one type (D6 of `typed-tree`).
                    let right_id = self.coerce_to(right_id, &left_ty);
                    binary(self, left_id, right_id, Type::Bool)
                } else if left_ty.is_assignable_to(&right_ty) {
                    let left_id = self.coerce_to(left_id, &right_ty);
                    binary(self, left_id, right_id, Type::Bool)
                } else {
                    self.errors.push(TypeError::TypeMismatch {
                        expected: left_ty,
                        found: right_ty,
                        span,
                        help: None,
                    });
                    binary(self, left_id, right_id, Type::Error)
                }
            }

            BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => {
                let left_id = self.infer_operand(left);
                let right_id = self.infer_operand(right);
                let (left_ty, right_ty) = (self.ty(left_id), self.ty(right_id));

                let comparable = left_ty == Type::Error
                    || right_ty == Type::Error
                    || (left_ty == Type::Int && right_ty == Type::Int)
                    || (left_ty == Type::Float && right_ty == Type::Float)
                    || (left_ty == Type::Char && right_ty == Type::Char);
                let ty = if comparable {
                    Type::Bool
                } else {
                    // Two `Bool`s or two `String`s must not read "expected 'Bool', found
                    // 'Bool'": the operator, not an operand, does not apply.
                    let help = type_parameter_operator_help(op, &left_ty, &right_ty).or_else(|| {
                        Some(format!(
                            "'{}' needs two operands that are both 'Int', both 'Float' or both 'Char'",
                            operator_text(op)
                        ))
                    });
                    self.errors.push(TypeError::OperatorMismatch {
                        operator: operator_text(op).to_string(),
                        left: left_ty,
                        right: right_ty,
                        span,
                        help,
                    });
                    Type::Error
                };
                binary(self, left_id, right_id, ty)
            }

            BinaryOp::And | BinaryOp::Or => {
                let left_id = self.check_operand(left, &Type::Bool);
                let right_id = self.check_operand(right, &Type::Bool);
                let operator = if op == BinaryOp::And {
                    LogicalOperator::And
                } else {
                    LogicalOperator::Or
                };
                let kind = ExpressionKind::Logical {
                    operator,
                    left: left_id,
                    right: right_id,
                };
                self.add(kind, Type::Bool, span)
            }

            BinaryOp::Elvis => {
                // Both sides produce the result, so both are consumed when it is.
                let consuming = self.parent_consuming;
                let left_id = self.with_consuming(consuming, |c| c.infer_expr(left));
                // The right side is checked even when the left side cannot be null. A borrowed
                // nullable `&(T?)` is read as `(&T)?` (D8 of `reference-access`).
                let left_ty = self.ty(left_id);
                let (left_id, left_ty) = match nullable_view(&left_ty) {
                    Some(view) => (self.nullable_view_chain(left_id), view),
                    None => (left_id, left_ty),
                };
                let left_is_nullable = matches!(left_ty, Type::Nullable(_));
                let inner = match left_ty {
                    Type::Nullable(inner) => *inner,
                    other => other,
                };
                let right_id =
                    self.with_consuming(consuming, |c| c.infer_expr_expecting(right, expected));
                let right_ty = self.ty(right_id);
                let mut fits = true;
                let ty = if inner == Type::Error || right_ty == Type::Error {
                    Type::Error
                } else if right_ty == Type::Nothing {
                    inner
                } else if inner == Type::Nothing {
                    right_ty
                } else if right_ty.is_assignable_to(&inner) {
                    inner
                } else if inner.is_assignable_to(&right_ty) {
                    right_ty
                } else {
                    self.errors.push(TypeError::TypeMismatch {
                        expected: inner.clone(),
                        found: right_ty,
                        span,
                        help: None,
                    });
                    fits = false;
                    inner
                };
                // Both sides are coerced to the result (D6 of `typed-tree`).
                let (left_id, right_id) = if fits {
                    let left_slot = if left_is_nullable {
                        ty.clone().nullable()
                    } else {
                        ty.clone()
                    };
                    (
                        self.coerce_to(left_id, &left_slot),
                        self.coerce_to(right_id, &ty),
                    )
                } else {
                    (left_id, right_id)
                };
                let kind = ExpressionKind::Elvis {
                    nullable: left_id,
                    fallback: right_id,
                };
                self.add(kind, ty, span)
            }

            // A half-open range has the same type; whether it excludes its end is a property
            // of the runtime value.
            BinaryOp::Range | BinaryOp::HalfOpenRange => {
                let start = self.check_operand(left, &Type::Int);
                let end = self.check_operand(right, &Type::Int);
                let kind = ExpressionKind::Range {
                    start,
                    end,
                    end_inclusive: op == BinaryOp::Range,
                };
                self.add(kind, Type::plain_struct(NominalId::builtin_range()), span)
            }
        }
    }

    /// Reads the borrowed nullable `id`, a `&(T?)` or `&mut (T?)`, as the optional borrow `(&T)?`
    /// or `(&mut T)?`: a chain borrowing the payload of its referent (D7 of `typed-tree`).
    fn nullable_view_chain(&mut self, id: ExpressionId) -> ExpressionId {
        let mutable = matches!(
            self.builder.type_of(id),
            Type::Reference { mutable: true, .. }
        );
        let span = self.builder.span_of(id);
        let referent = self.deref(id);
        let payload = self.non_null(referent);
        let borrow = self.borrow(mutable, BorrowOrigin::Conversion, payload);
        self.close_chain(None, borrow, SafeNavigationOrigin::NullableView, span)
    }

    /// A borrow of `id`, held in a temporary when it is not a place (D5 of `typed-tree`).
    fn borrow(&mut self, mutable: bool, origin: BorrowOrigin, id: ExpressionId) -> ExpressionId {
        let place = self.as_place(id);
        let (ty, span) = (self.ty(place), self.builder.span_of(place));
        let kind = ExpressionKind::Borrow {
            mutable,
            origin,
            place,
        };
        let ty = Type::Reference {
            mutable,
            inner: Box::new(ty),
        };
        self.add(kind, ty, span)
    }

    fn infer_unary_op(&mut self, op: UnaryOp, inner: &Expr, span: Span) -> ExpressionId {
        if let Some(field) = self.implicit_this_field(inner) {
            return self.infer_unary_op(op, &field, span);
        }
        match op {
            UnaryOp::Neg => {
                let operand = self.infer_operand(inner);
                let ty = self.ty(operand);
                let ty = if ty == Type::Int || ty == Type::Float || ty == Type::Error {
                    ty
                } else {
                    self.errors.push(TypeError::TypeMismatch {
                        expected: Type::Int,
                        found: ty,
                        span,
                        help: None,
                    });
                    Type::Error
                };
                let kind = ExpressionKind::Unary {
                    operator: UnaryOperator::Negate,
                    operand,
                };
                self.add(kind, ty, span)
            }
            UnaryOp::Not => {
                let operand = self.check_operand(inner, &Type::Bool);
                let kind = ExpressionKind::Unary {
                    operator: UnaryOperator::Not,
                    operand,
                };
                self.add(kind, Type::Bool, span)
            }
            // The parser produces `*` only at the top of an assignment target, which
            // `check_assignment` handles; reads through references are implicit.
            UnaryOp::Deref => {
                debug_assert!(false, "'*' outside an assignment target: {}", inner);
                self.error_node(Vec::new(), span)
            }
            // A place is borrowed where it lives; anything else is a temporary, which is
            // consumed (D2 of `reference-access`).
            UnaryOp::Borrow => {
                let value = self.with_consuming(!self.is_place(inner), |c| c.infer_expr(inner));
                if self.ty(value) == Type::Error {
                    return self.error_node(vec![value], span);
                }
                let borrow = self.borrow(false, BorrowOrigin::Written, value);
                self.builder.expression_mut(borrow).span = span;
                borrow
            }
            UnaryOp::BorrowMut => {
                let (value, writable) = if self.is_place(inner) {
                    self.infer_place(inner)
                } else {
                    (self.with_consuming(true, |c| c.infer_expr(inner)), true)
                };
                let ty = self.ty(value);
                if ty == Type::Error {
                    return self.error_node(vec![value], span);
                }
                if self.reject_guard_mutation(inner, span) {
                    return self.error_node(vec![value], span);
                }
                match (binding_name(inner), &inner.kind) {
                    (Some(name), _) => {
                        let is_val = matches!(
                            self.env.lookup(name).map(|sym| &sym.kind),
                            Some(SymbolKind::Variable {
                                mutability: Mutability::Val,
                                ..
                            })
                        );
                        if is_val {
                            // `&mut r` borrows the binding itself, even when it holds a `&mut T`.
                            let help = matches!(ty, Type::Reference { mutable: true, .. }).then(|| {
                                format!(
                                    "'{}' is already a mutable borrow: pass '{}' itself, which reborrows it",
                                    name, name
                                )
                            });
                            self.errors.push(TypeError::ImmutabilityViolation {
                                message: format!(
                                    "cannot take mutable borrow '&mut' of immutable 'val' binding '{}'",
                                    name
                                ),
                                span,
                                help,
                            });
                        }
                    }
                    (None, ExprKind::MemberAccess { member, .. }) if !writable => {
                        self.errors.push(TypeError::ImmutabilityViolation {
                            message: format!(
                                "cannot take mutable borrow '&mut' of field '{}': it is a 'val' field or is reached through a 'val' binding, 'val' field or shared '&' borrow",
                                member
                            ),
                            span,
                            help: None,
                        });
                    }
                    _ => {}
                }
                let borrow = self.borrow(true, BorrowOrigin::Written, value);
                self.builder.expression_mut(borrow).span = span;
                borrow
            }
        }
    }

    /// Types the target of an assignment or a compound assignment, reporting a target that cannot
    /// be written (`E4003`) except a `val` binding, which the caller reports. Returns the target's
    /// place (an error node when it is not one), its type, and whether it is writable: a `var`
    /// binding, a writable field or a write through `&mut`. Implicit `this` fields are resolved
    /// by the caller.
    fn assignment_target(&mut self, target: &Expr, span: Span) -> (ExpressionId, Type, bool) {
        if self.reject_guard_mutation(target, span) {
            return (self.error_node(Vec::new(), target.span), Type::Error, false);
        }
        let (target_id, target_ty, is_mutable) = match (&target.kind, binding_name(target)) {
            (ExprKind::This, _) if self.env.lookup(THIS).is_none() => {
                self.errors
                    .push(TypeError::ThisOutsideExtension { span: target.span });
                (self.error_node(Vec::new(), target.span), Type::Error, false)
            }
            (_, Some(name)) => match self.env.resolve(name, target.span) {
                Ok(sym) => {
                    let sym = sym.clone();
                    match &sym.kind {
                        SymbolKind::Variable { ty, mutability, .. } => {
                            let node = self.symbol_node(&sym, name, target.span);
                            (node, ty.clone(), *mutability == Mutability::Var)
                        }
                        _ => {
                            self.errors.push(TypeError::ImmutabilityViolation {
                                message: format!("cannot assign to non-variable '{}'", name),
                                span,
                                help: None,
                            });
                            (self.error_node(Vec::new(), target.span), Type::Error, false)
                        }
                    }
                }
                Err(err) => {
                    self.errors.push(err);
                    (self.error_node(Vec::new(), target.span), Type::Error, false)
                }
            },
            (
                ExprKind::MemberAccess {
                    target: base,
                    member,
                    is_safe_nav: false,
                },
                _,
            ) => {
                let (field_id, base_ty, base_writable, is_var) =
                    match self.type_name_member(base, member, target.span) {
                        Some((node, is_var)) => (node, self.ty(node), false, is_var),
                        None => {
                            let (base_id, base_writable) = self.infer_place(base);
                            let base_ty = self.ty(base_id);
                            let (field_id, is_var) =
                                self.build_member_access(base_id, member, false, target.span);
                            (field_id, base_ty, base_writable, is_var)
                        }
                    };
                let field_ty = self.ty(field_id);
                match is_var {
                    Some(false) => {
                        self.errors.push(TypeError::ImmutabilityViolation {
                            message: format!(
                                "cannot assign to immutable 'val' struct field '{}'",
                                member
                            ),
                            span,
                            help: None,
                        });
                        (field_id, field_ty, false)
                    }
                    Some(true) if !contents_writable(&base_ty, base_writable) => {
                        self.errors.push(TypeError::ImmutabilityViolation {
                            message: format!(
                                "cannot assign to field '{}' of a value that is not mutable (a 'val' binding or field, or a shared '&' borrow)",
                                member
                            ),
                            span,
                            help: None,
                        });
                        (field_id, field_ty, false)
                    }
                    Some(true) => (field_id, field_ty, true),
                    None => {
                        if field_ty != Type::Error {
                            self.errors.push(TypeError::UnknownField {
                                target_type: base_ty.clone(),
                                field_name: member.clone(),
                                span,
                                help: None,
                            });
                        }
                        (
                            self.error_node(vec![field_id], target.span),
                            Type::Error,
                            false,
                        )
                    }
                }
            }
            (
                ExprKind::Unary {
                    op: UnaryOp::Deref,
                    expr: reference,
                },
                _,
            ) => {
                let (node, referent, _) = self.write_through(reference, span);
                (node, referent, true)
            }
            (ExprKind::Index { .. }, _) => {
                let (node, writable) = self.with_origin(Origin::Value, |c| c.infer_place(target));
                if matches!(self.builder.kind(node), ExpressionKind::Index { .. }) {
                    // An element of a list (D7 of `collections`).
                    let element_ty = self.ty(node);
                    if !writable {
                        self.errors.push(TypeError::ImmutabilityViolation {
                            message: format!(
                                "cannot assign to '{}': the list is not a mutable place (a 'val' binding or field, or reached through a shared '&' borrow)",
                                target
                            ),
                            span,
                            help: Some(
                                "a list is changed through a 'var' or a '&mut'".to_string(),
                            ),
                        });
                    }
                    return (node, element_ty, writable);
                }
                if self.ty(node) != Type::Error {
                    self.errors.push(TypeError::ImmutabilityViolation {
                        message: format!("cannot assign to '{}'", target),
                        span,
                        help: Some(
                            "the characters of a string cannot be assigned: strings are immutable"
                                .to_string(),
                        ),
                    });
                }
                (self.wrap_error(node), Type::Error, false)
            }
            _ => {
                self.errors.push(TypeError::ImmutabilityViolation {
                    message: "invalid assignment target".to_string(),
                    span,
                    help: None,
                });
                (self.error_node(Vec::new(), target.span), Type::Error, false)
            }
        };
        // The target is a place; anything else was reported above.
        let target_id = if self.builder.kind(target_id).is_place() {
            target_id
        } else {
            self.wrap_error(target_id)
        };
        (target_id, target_ty, is_mutable)
    }

    /// Checks the assignment `target = value`, building its statement over the target place
    /// (D10 of `typed-tree`).
    fn check_assignment(&mut self, target: &Expr, value: &Expr, span: Span) -> StatementId {
        if let Some(field) = self.implicit_this_field(target) {
            return self.check_assignment(&field, value, span);
        }
        let (target_id, target_ty, is_mutable) = self.assignment_target(target, span);

        if let (Some(name), Type::Reference { mutable, inner }) = (binding_name(target), &target_ty)
        {
            let value_id =
                self.check_repoint(name, is_mutable, *mutable, inner, &target_ty, value, span);
            let kind = TreeStatementKind::Assign {
                target: target_id,
                value: value_id,
            };
            return self.builder.add_statement(kind, span);
        }

        if !is_mutable && target_ty != Type::Error {
            if let Some(name) = binding_name(target) {
                self.errors.push(TypeError::ImmutabilityViolation {
                    message: format!("cannot reassign to immutable 'val' binding '{}'", name),
                    span,
                    help: None,
                });
            }
        }

        let reported = self.errors.len();
        let value_id = self.with_consuming(true, |c| c.check_expr(value, &target_ty));
        // `m = 5` for a box `m`: the value fits the contents, which `*m = 5` writes (D5 of
        // `heap-boxes`).
        if let (Some(name), Type::Heap(contents), [TypeError::TypeMismatch { found, help, .. }]) =
            (binding_name(target), &target_ty, &self.errors[reported..])
        {
            let fits_contents =
                matches!(self.coerce(found, contents, Site::Value), Coercion::Fits(_));
            if help.is_none() && fits_contents {
                let fix = format!(
                    "to write the contents of the box, use '*{} = {}'",
                    name, value
                );
                if let Some(TypeError::TypeMismatch { help, .. }) = self.errors.last_mut() {
                    *help = Some(fix);
                }
            }
        }
        let kind = TreeStatementKind::Assign {
            target: target_id,
            value: value_id,
        };
        self.builder.add_statement(kind, span)
    }

    /// Checks the compound assignment `target op= value` (`compound-assignment`), building its
    /// statement over the target place (D4). It is `target = target op value` with the target
    /// written once: the target must be writable as for `=` (D7), the value is an operator
    /// operand, read through references and never consumed, and the operator must apply to the
    /// target's and the value's types, its result then having the target's type (D3). A binding
    /// of reference type is rejected as `r = 5` is (D6 of `reference-access`).
    fn check_compound_assignment(
        &mut self,
        operator: BinaryOp,
        target: &Expr,
        value: &Expr,
        span: Span,
    ) -> StatementId {
        if let Some(field) = self.implicit_this_field(target) {
            return self.check_compound_assignment(operator, &field, value, span);
        }
        let (target_id, target_ty, is_mutable) = self.assignment_target(target, span);
        let value_id = self.with_consuming(false, |c| c.infer_operand(value));
        let value_ty = self.ty(value_id);
        let compound = format!("{}=", operator.as_str());
        let checked = target_ty != Type::Error && value_ty != Type::Error;

        let fits = match (binding_name(target), &target_ty) {
            (Some(name), Type::Reference { mutable, .. }) => {
                if checked {
                    self.reject_compound_repoint(
                        name, is_mutable, *mutable, &compound, &target_ty, &value_ty, value, span,
                    );
                }
                !checked
            }
            _ => {
                if !is_mutable && target_ty != Type::Error {
                    if let Some(name) = binding_name(target) {
                        self.errors.push(TypeError::ImmutabilityViolation {
                            message: format!(
                                "cannot reassign to immutable 'val' binding '{}'",
                                name
                            ),
                            span,
                            help: None,
                        });
                    }
                }
                if checked && arithmetic_result(operator, &target_ty, &value_ty).is_none() {
                    let help = type_parameter_operator_help(operator, &target_ty, &value_ty)
                        .or_else(|| Some(compound_operand_help(operator, &compound)));
                    self.errors.push(TypeError::OperatorMismatch {
                        operator: compound,
                        left: target_ty,
                        right: value_ty,
                        span,
                        help,
                    });
                    false
                } else {
                    true
                }
            }
        };
        let value_id = if fits {
            value_id
        } else {
            self.wrap_error(value_id)
        };
        let kind = TreeStatementKind::CompoundAssign {
            operator: binary_operator(operator),
            target: target_id,
            value: value_id,
        };
        self.builder.add_statement(kind, span)
    }

    /// Reports `name op= value` for a binding of reference type `target_ty`, which would
    /// re-point the binding to a number or a string (D7 of `compound-assignment`): a `val`
    /// reports only `E4003`, a `var` `E4002`, each with help pointing at `*name op= value` for a
    /// `&mut` reference.
    #[allow(clippy::too_many_arguments)]
    fn reject_compound_repoint(
        &mut self,
        name: &str,
        is_var: bool,
        is_mutable_reference: bool,
        compound: &str,
        target_ty: &Type,
        value_ty: &Type,
        value: &Expr,
        span: Span,
    ) {
        let help = Some(if is_mutable_reference {
            format!(
                "to write through the reference, use '*{} {} {}'",
                name, compound, value
            )
        } else {
            format!(
                "'{}' is a shared '&' borrow, which cannot be written through",
                name
            )
        });
        self.errors.push(if is_var {
            TypeError::TypeMismatch {
                expected: target_ty.clone(),
                found: value_ty.clone(),
                span: value.span,
                help,
            }
        } else {
            TypeError::ImmutabilityViolation {
                message: format!(
                    "cannot reassign to immutable 'val' binding '{}' of reference type",
                    name
                ),
                span,
                help,
            }
        });
    }

    /// Types the place written by `*reference = ...` (D5 of `reference-access`, D5 of
    /// `heap-boxes`): each `*` peels exactly one layer, a reference or a box, of `reference`,
    /// which is read in place. A reference layer must be `&mut`, reached by a path that may be
    /// written through, and a box layer must be a mutable place (a `var` binding, a `var` field
    /// on a writable path, or a box reached through `&mut`), as field writes through boxes are
    /// (`contents_writable`). Returns the written place (a dereference), its type and whether
    /// the write is allowed; a refusal is reported once, at the first layer that refuses it.
    fn write_through(&mut self, reference: &Expr, span: Span) -> (ExpressionId, Type, bool) {
        let (node, ty, writable, _) = self.write_through_layer(reference, span);
        (node, ty, writable)
    }

    /// One layer of [`Self::write_through`]: the place `*reference` denotes, its type, whether
    /// it may be written, and whether it lies behind a reference.
    fn write_through_layer(
        &mut self,
        reference: &Expr,
        span: Span,
    ) -> (ExpressionId, Type, bool, bool) {
        let (node, ty, writable, behind_reference) = match &reference.kind {
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr: inner,
            } => self.write_through_layer(inner, span),
            _ if self.is_place(reference) => {
                let (id, writable) = self.infer_place(reference);
                let ty = self.ty(id);
                (id, ty, writable, false)
            }
            _ => {
                let id = self.infer_expr(reference);
                (id, self.ty(id), true, false)
            }
        };
        // Whether a refusal was reported at an inner layer: a write through a layer that may
        // not be written is refused there, not again.
        let refused = matches!(
            &reference.kind,
            ExprKind::Unary {
                op: UnaryOp::Deref,
                ..
            }
        ) && !writable;
        match ty {
            Type::Error => (node, Type::Error, false, behind_reference),
            Type::Reference { mutable, inner } => {
                let through = if behind_reference {
                    writable && mutable
                } else {
                    mutable
                };
                if !through && !refused {
                    // The variable of a loop over a list borrowed shared (D9 of `collections`).
                    let loop_help = binding_name(reference)
                        .and_then(|name| self.env.lookup(name))
                        .and_then(|sym| self.shared_loop_variables.get(&sym.span))
                        .map(|list| {
                            format!(
                                "iterate the list mutably to write its elements: 'for ({} in &mut {})'",
                                reference, list
                            )
                        });
                    self.errors.push(TypeError::ImmutabilityViolation {
                        message: format!(
                            "cannot write through '{}': it is a shared '&' borrow, which cannot be written through",
                            reference
                        ),
                        span,
                        help: Some(loop_help.unwrap_or_else(|| {
                            "take a mutable borrow '&mut' where the reference is created"
                                .to_string()
                        })),
                    });
                }
                let referent = self.deref(node);
                (referent, *inner, through, true)
            }
            Type::Heap(inner) => {
                if !writable && !refused {
                    self.errors.push(TypeError::ImmutabilityViolation {
                        message: format!(
                            "cannot write the contents of the box '{}': it is not in a mutable place",
                            reference
                        ),
                        span,
                        help: Some(box_write_help(reference, behind_reference)),
                    });
                }
                let contents = self.deref(node);
                (contents, *inner, writable, behind_reference)
            }
            other if refused => (self.wrap_error(node), other, false, behind_reference),
            other => {
                self.errors.push(TypeError::NotAReference {
                    ty: other,
                    span: reference.span,
                });
                (self.wrap_error(node), Type::Error, false, behind_reference)
            }
        }
    }

    /// Checks `name = value` for a binding of reference type `target_ty` (D6 of
    /// `reference-access`). Assigning a reference re-points the binding. Assigning a value of the
    /// referent's type is rejected with help pointing at `*name = value`: a `val` reports only
    /// `E4003`, a `var` `E4002`. Returns the value's node.
    #[allow(clippy::too_many_arguments)]
    fn check_repoint(
        &mut self,
        name: &str,
        is_var: bool,
        is_mutable_reference: bool,
        referent: &Type,
        target_ty: &Type,
        value: &Expr,
        span: Span,
    ) -> ExpressionId {
        let deferred_move = is_move_leaf(value);
        let found_id = self.with_consuming(!deferred_move, |c| c.infer_expr(value));
        let found = self.ty(found_id);
        if found == Type::Error
            || matches!(
                self.coerce(&found, target_ty, Site::Value),
                Coercion::Fits(_)
            )
            || !matches!(
                self.coerce(&found, referent, Site::Value),
                Coercion::Fits(_)
            )
        {
            // A re-point, or a value that fits neither: the usual checks apply.
            if !is_var && found != Type::Error {
                self.errors.push(TypeError::ImmutabilityViolation {
                    message: format!("cannot reassign to immutable 'val' binding '{}'", name),
                    span,
                    help: None,
                });
            }
            return self.fit(value, found_id, target_ty, Site::Value, deferred_move);
        }
        let help = Some(if is_mutable_reference {
            format!(
                "to write through the reference instead of re-pointing it, use '*{} = {}'",
                name, value
            )
        } else {
            format!(
                "'{}' is a shared '&' borrow, which cannot be written through; assign a reference to re-point it",
                name
            )
        });
        self.errors.push(if is_var {
            TypeError::TypeMismatch {
                expected: target_ty.clone(),
                found,
                span: value.span,
                help,
            }
        } else {
            TypeError::ImmutabilityViolation {
                message: format!(
                    "cannot reassign to immutable 'val' binding '{}' of reference type",
                    name
                ),
                span,
                help,
            }
        });
        self.wrap_error(found_id)
    }

    /// Typechecks a local declaration and binds it in the current scope.
    fn check_local(
        &mut self,
        is_var: bool,
        name: &str,
        annotation: Option<&prism_ast::types::Type>,
        initializer: &Expr,
        span: Span,
    ) -> StatementId {
        let (initializer, var_ty) = if let Some(a) = annotation {
            self.check_annotation_names(a);
            let expected = self.lower_type(a);
            let id = self.with_consuming(true, |c| c.check_expr(initializer, &expected));
            (id, expected)
        } else {
            let id = self.with_consuming(true, |c| c.infer_expr(initializer));
            (id, self.ty(id))
        };

        let mutability = if is_var {
            Mutability::Var
        } else {
            Mutability::Val
        };
        let sym = self.bind_local(name, var_ty, mutability, LocalKind::Let, span);
        let local = Self::local_of(&sym).expect("a bound local");
        self.define_local(sym);
        self.builder
            .add_statement(TreeStatementKind::Let { local, initializer }, span)
    }

    /// Typechecks a statement of a block.
    fn check_statement(&mut self, statement: &Statement) -> StatementId {
        match &statement.kind {
            StatementKind::Local {
                is_var,
                name,
                ty,
                initializer,
            } => self.check_local(*is_var, name, ty.as_ref(), initializer, statement.span),
            StatementKind::Destructure {
                is_var,
                entries,
                initializer,
            } => self.check_destructure(*is_var, entries, initializer, statement.span),
            StatementKind::Assign { target, value } => {
                self.check_assignment(target, value, statement.span)
            }
            StatementKind::CompoundAssign {
                operator,
                target,
                value,
            } => self.check_compound_assignment(*operator, target, value, statement.span),
            StatementKind::Expression(expr) => {
                let value = self.infer_discarded_expr(expr);
                self.builder
                    .add_statement(TreeStatementKind::Expression { value }, statement.span)
            }
            StatementKind::While { condition, body } => {
                let condition = self.check_operand(condition, &Type::Bool);
                let id = self.builder.reserve_statement(statement.span);
                let body = self.check_loop_body(body, id);
                self.builder
                    .set_statement(id, TreeStatementKind::While { condition, body });
                id
            }
            StatementKind::DoWhile { body, condition } => {
                // The condition is checked in the body's scope, so it sees the body's locals.
                self.env.enter_scope();
                let id = self.builder.reserve_statement(statement.span);
                self.builder.loops.push(id);
                let mut statements = Vec::new();
                for statement in &body.statements {
                    statements.push(self.check_statement(statement));
                }
                if let Some(tail) = &body.tail {
                    let value = self.infer_discarded_expr(tail);
                    statements.push(
                        self.builder
                            .add_statement(TreeStatementKind::Expression { value }, tail.span),
                    );
                }
                self.builder.loops.pop();
                let kind = ExpressionKind::Block {
                    statements,
                    tail: None,
                    close_brace: body.close_brace,
                };
                let body_id = self.add(kind, Type::Unit, body.span);
                let condition = self.check_operand(condition, &Type::Bool);
                self.env.exit_scope();
                self.builder.set_statement(
                    id,
                    TreeStatementKind::DoWhile {
                        body: body_id,
                        condition,
                    },
                );
                id
            }
            StatementKind::For {
                variable,
                variable_span,
                iterable,
                body,
            } => {
                let (iterable_id, iteration, variable_ty) = self.check_iterable(iterable);
                if iteration == (ForIteration::List { mutable: false }) {
                    self.shared_loop_variables
                        .insert(*variable_span, iterable.to_string());
                }
                // The loop variable sits in a scope level above the body, so the body may
                // shadow it, as a function body may shadow a parameter.
                self.env.enter_scope();
                let sym = self.bind_local(
                    variable,
                    variable_ty,
                    Mutability::Val,
                    LocalKind::ForVariable,
                    *variable_span,
                );
                let local = Self::local_of(&sym).expect("a bound local");
                self.define_local(sym);
                let id = self.builder.reserve_statement(statement.span);
                let body = self.check_loop_body(body, id);
                self.env.exit_scope();
                self.builder.set_statement(
                    id,
                    TreeStatementKind::For {
                        variable: local,
                        iterable: iterable_id,
                        iteration,
                        body,
                    },
                );
                id
            }
        }
    }

    /// Checks the iterable of a `for` loop (the `loops` spec, "For Loops Over Ranges", and D9 of
    /// `collections`): read in place when it is a place and evaluated into a temporary
    /// otherwise, then read through its `&`, `&mut` and `Heap<T>` layers. A `Range` is iterated
    /// by value, binding an `Int`. A list is borrowed for the whole loop, mutably when the
    /// iterable is a `&mut` (`&mut xs`, or a `&mut List<T>` parameter), binding a `&T` or
    /// `&mut T`. Anything else is `E4002`. Returns the iterable's node, how the loop iterates and
    /// the variable's type.
    fn check_iterable(&mut self, iterable: &Expr) -> (ExpressionId, ForIteration, Type) {
        let id = self.with_origin(Origin::Value, |c| c.infer_read(iterable));
        let iterable_ty = self.ty(id);
        let peeled = self.peel_indirection_nodes(id);
        let range = Type::plain_struct(NominalId::builtin_range());
        match self.ty(peeled) {
            Type::List(element) => {
                let mutable = matches!(iterable_ty, Type::Reference { mutable: true, .. })
                    && contents_writable(&iterable_ty, false);
                let borrow = self.borrow(mutable, BorrowOrigin::Iteration, peeled);
                let variable_ty = Type::Reference {
                    mutable,
                    inner: element,
                };
                (borrow, ForIteration::List { mutable }, variable_ty)
            }
            ty if ty.is_assignable_to(&range) => (
                self.coerce_to(peeled, &range),
                ForIteration::Range,
                Type::Int,
            ),
            ty => {
                self.with_origin(Origin::Value, |c| {
                    c.report_mismatch_with_help(
                        &range,
                        ty,
                        iterable.span,
                        Some("a 'for' loop iterates a 'Range' or a 'List<T>'".to_string()),
                    )
                });
                (self.wrap_error(peeled), ForIteration::Range, Type::Int)
            }
        }
    }

    /// Typechecks the body of the `while` or `for` loop `id`, whose value is discarded.
    fn check_loop_body(&mut self, body: &Block, id: StatementId) -> ExpressionId {
        self.builder.loops.push(id);
        let body = self.check_block(body, Some(&Type::Unit));
        self.builder.loops.pop();
        body
    }

    /// Typechecks an expression whose value is discarded. Only `when` differs from value
    /// position: it needs neither an `else` nor a common branch type.
    fn infer_discarded_expr(&mut self, expr: &Expr) -> ExpressionId {
        match &expr.kind {
            ExprKind::When { target, branches } => {
                self.infer_when_expr(target.as_deref(), branches, WhenUse::Statement, expr.span)
            }
            _ => self.infer_expr(expr),
        }
    }

    /// Typechecks a block. With an expected type its final expression is checked against it,
    /// except that a block expected to produce `Unit` (the body of a `Unit` function or of a
    /// closure expected to return `Unit`) discards its final value: the expression is
    /// typechecked as a statement and its type ignored (D8 of `typechecker-soundness`), and the
    /// block's node holds it as a statement, with no tail (D9 of `typed-tree`).
    fn check_block(&mut self, block: &Block, expected: Option<&Type>) -> ExpressionId {
        // The tail produces the block's value, so it is consumed when the block is.
        let tail_consuming = std::mem::replace(&mut self.consuming, false);
        let saved_parent = std::mem::replace(&mut self.parent_consuming, false);
        self.env.enter_scope();
        let mut statements = self.with_origin(Origin::Value, |checker| {
            block
                .statements
                .iter()
                .map(|statement| checker.check_statement(statement))
                .collect::<Vec<_>>()
        });

        // The tail of a block whose expected type is `Unit` is a statement too. A checked tail
        // produces the block's value, so it keeps the expectation origin.
        let (tail, result_ty) = match (&block.tail, expected) {
            (Some(tail), Some(Type::Unit)) => {
                let value = self.with_origin(Origin::Value, |c| c.infer_discarded_expr(tail));
                statements.push(
                    self.builder
                        .add_statement(TreeStatementKind::Expression { value }, tail.span),
                );
                (None, Type::Unit)
            }
            (Some(tail), Some(exp)) => {
                let id = self.with_consuming(tail_consuming, |c| c.check_expr(tail, exp));
                (Some(id), self.ty(id))
            }
            (Some(tail), None) => {
                let id = self.with_origin(Origin::Value, |c| {
                    c.with_consuming(tail_consuming, |c| c.infer_expr(tail))
                });
                (Some(id), self.ty(id))
            }
            // A block without a tail is `Unit`. When a value was expected, the mismatch is
            // reported at the last statement, which is where the value is missing.
            (None, Some(exp)) if exp != &Type::Unit && exp != &Type::Error => {
                match block.statements.last() {
                    Some(last) => {
                        self.report_mismatch(exp, Type::Unit, last.span);
                        (None, Type::Error)
                    }
                    None => (None, Type::Unit),
                }
            }
            (None, _) => (None, Type::Unit),
        };

        self.env.exit_scope();
        self.parent_consuming = saved_parent;
        let kind = ExpressionKind::Block {
            statements,
            tail,
            close_brace: block.close_brace,
        };
        self.add(kind, result_ty, block.span)
    }

    /// Typechecks a `when`. When its value is used it must be exhaustive (`E4009`), and its
    /// branches either join to a common type (`WhenUse::Infer`) or are each checked against the
    /// expected type (`WhenUse::Check`). As a statement its value is discarded, so neither
    /// applies and its type is `Unit`.
    ///
    /// Each pattern is checked against the subject (`patterns.rs`); the bindings of a branch are
    /// defined, read-only, for its guard and then for its body (D9 and D10 of
    /// `payload-patterns`). A pattern that moves a part takes the subject apart, which must then
    /// be a whole owned value (D6).
    fn infer_when_expr(
        &mut self,
        target: Option<&Expr>,
        branches: &[WhenBranch],
        use_: WhenUse,
        span: Span,
    ) -> ExpressionId {
        let value_position = !matches!(use_, WhenUse::Statement);
        // The branches produce the value, so they are consumed when it is.
        let branch_consuming = value_position && self.parent_consuming;
        // The subject and the patterns are operands, so their expectations are plain values.
        let branch_origin = self.origin;
        self.origin = Origin::Value;
        // Patterns match the borrowed value, so a `&Enum` or `&mut Enum` subject matches as
        // `Enum`; the subject is read in place through its dereferences (D9 of `typed-tree`),
        // and the layers peeled decide the binding modes (D5 of `payload-patterns`).
        let mut layers = Vec::new();
        let mut subject = target.map(|t| {
            let mut id = self.infer_read(t);
            while let Type::Reference { mutable, .. } = self.builder.type_of(id) {
                layers.push(*mutable);
                id = self.deref(id);
            }
            let ty = self.ty(id);
            // Matching on variants needs the enum's definition, which needs its module.
            if self.reject_unimported_members(&ty, t.span) {
                self.wrap_error(id)
            } else {
                id
            }
        });
        let mode = Self::subject_mode(&layers);
        let subject_ty = subject.map(|id| self.ty(id));
        // A pattern that does not check covers nothing reliably: its `when` is not reported as
        // non-exhaustive on top of its own error.
        let errors_before = self.errors.len();
        let mut pattern_errors = false;
        let mut unified_ty: Option<Type> = None;
        let mut tree_branches = Vec::new();

        for branch in branches {
            let mut patterns = Vec::new();
            // The bindings of the branch's first pattern, which its alternatives must repeat.
            let mut first: Option<Vec<patterns::PatternBinding>> = None;
            for pattern in &branch.patterns {
                let pattern = match pattern {
                    WhenPattern::Else => {
                        patterns.push(Pattern::Else);
                        continue;
                    }
                    WhenPattern::Pattern(pattern) => pattern,
                };
                let Some(s_ty) = &subject_ty else {
                    patterns.push(self.condition_pattern(pattern));
                    continue;
                };
                let mut bindings = patterns::PatternBindings {
                    bound: Vec::new(),
                    first: first.clone(),
                };
                patterns.push(self.check_pattern(pattern, s_ty, mode, &mut bindings));
                match &first {
                    None => first = Some(bindings.bound),
                    Some(first) => {
                        self.check_alternative_bindings(first, &bindings.bound, pattern.span)
                    }
                }
            }
            let bindings = first.unwrap_or_default();
            pattern_errors |= self.errors.len() > errors_before;

            let guard = branch.guard.as_ref().map(|guard| {
                self.env.enter_scope();
                for binding in &bindings {
                    self.define_local(binding.symbol(true));
                }
                let guarded = bindings.iter().map(|b| (b.local, b.mode)).collect();
                let saved = std::mem::replace(&mut self.guard_bindings, guarded);
                let guard = self.check_operand(guard, &Type::Bool);
                self.guard_bindings = saved;
                self.env.exit_scope();
                guard
            });

            self.env.enter_scope();
            for binding in &bindings {
                self.define_local(binding.symbol(false));
            }
            let body = if let WhenUse::Check(expected) = use_ {
                self.with_consuming(branch_consuming, |c| {
                    c.check_expr_at(&branch.body, expected, branch_origin)
                })
            } else {
                let body = self.with_consuming(branch_consuming, |c| c.infer_expr(&branch.body));
                let branch_ty = self.ty(body);
                if value_position && branch_ty != Type::Nothing {
                    if let Some(current) = &unified_ty {
                        if let Some(joined) = current.join(&branch_ty) {
                            unified_ty = Some(joined);
                        } else {
                            self.errors.push(TypeError::TypeMismatch {
                                expected: current.clone(),
                                found: branch_ty.clone(),
                                span: branch.span,
                                help: None,
                            });
                        }
                    } else {
                        unified_ty = Some(branch_ty);
                    }
                }
                body
            };
            self.env.exit_scope();
            tree_branches.push(prism_typed_tree::WhenBranch {
                patterns,
                guard,
                body,
                span: branch.span,
            });
        }

        // A pattern that moves a part takes the whole subject apart (D6 of `payload-patterns`).
        let takes_apart = tree_branches
            .iter()
            .any(|branch| branch.patterns.iter().any(Pattern::moves));
        if let (true, Some(target), Some(id)) = (takes_apart, target, subject) {
            self.check_take_apart_source(target, "match on");
            subject = Some(self.as_place(id));
        }

        self.origin = branch_origin;
        let missing = match subject_ty {
            Some(Type::Error) => None,
            _ => self.missing_value(subject_ty.as_ref(), &tree_branches),
        };
        let covered = missing.is_none();
        let build = |checker: &mut Self, branches, ty| {
            let kind = ExpressionKind::When {
                subject,
                branches,
                exhaustive: covered,
            };
            checker.add(kind, ty, span)
        };
        if !value_position {
            for branch in &mut tree_branches {
                branch.body = self.discard_branch(branch.body);
            }
            return build(self, tree_branches, Type::Unit);
        }

        // Exhaustiveness check: a `when` in value position needs an `else` or every variant. A
        // subject whose type is already an error has nothing more to report.
        let is_exhaustive = covered || subject_ty == Some(Type::Error) || pattern_errors;

        if !is_exhaustive {
            let message = match (&missing, target) {
                (Some(missing), Some(_)) => format!(
                    "'when' expression in value position is not exhaustive: '{}' is not covered",
                    missing
                ),
                _ => "'when' expression without a subject in value position must have an 'else' branch"
                    .to_string(),
            };
            self.errors.push(TypeError::NonExhaustiveWhen {
                message,
                witness: missing.filter(|_| target.is_some()),
                span,
            });
        }
        match use_ {
            WhenUse::Check(expected) => build(self, tree_branches, expected.clone()),
            _ => {
                let ty = unified_ty.unwrap_or(Type::Unit);
                // Each branch is coerced to the join (D6 of `typed-tree`).
                for branch in &mut tree_branches {
                    branch.body = self.coerce_to(branch.body, &ty);
                }
                build(self, tree_branches, ty)
            }
        }
    }

    /// A branch pattern of a `when` without a subject: a condition. A range is not one: its
    /// ends are checked as a range's, and the range rejected.
    fn condition_pattern(&mut self, pattern: &prism_ast::Pattern) -> Pattern {
        match &pattern.kind {
            PatternKind::InRange { start, end, .. } => {
                let start = self.check_operand(start, &Type::Int);
                let end = self.check_operand(end, &Type::Int);
                let span = self.builder.span_of(start).merge(self.builder.span_of(end));
                self.report_mismatch(
                    &Type::Bool,
                    Type::plain_struct(NominalId::builtin_range()),
                    span,
                );
                Pattern::Condition(self.error_node(vec![start, end], span))
            }
            PatternKind::Value(condition) => {
                Pattern::Condition(self.check_operand(condition, &Type::Bool))
            }
            // The parser accepts only conditions and ranges without a subject.
            _ => Pattern::Condition(self.error_node(Vec::new(), pattern.span)),
        }
    }

    /// Whether the branches cover every value of the subject (D11 of `payload-patterns`): with
    /// a subject, by the usefulness check, which also names a value no branch covers; without
    /// one, only `else` covers everything, since conditions cover nothing.
    fn missing_value(
        &self,
        subject_ty: Option<&Type>,
        tree_branches: &[prism_typed_tree::WhenBranch],
    ) -> Option<String> {
        let Some(subject_ty) = subject_ty else {
            let has_else = tree_branches
                .iter()
                .any(|b| b.patterns.iter().any(|p| matches!(p, Pattern::Else)));
            return (!has_else).then(|| "_".to_string());
        };
        let rows: Vec<exhaustiveness::Row> = tree_branches
            .iter()
            .map(|branch| exhaustiveness::Row {
                patterns: branch.patterns.iter().collect(),
                guarded: branch.guard.is_some(),
            })
            .collect();
        let literal = |mut id: ExpressionId| loop {
            match self.builder.kind(id) {
                ExpressionKind::Coerce { value, .. } => id = *value,
                ExpressionKind::Literal(literal) => return Some(literal.clone()),
                _ => return None,
            }
        };
        exhaustiveness::missing_value(&self.types, subject_ty, &rows, &literal)
    }

    /// Infers the call `callee<type_arguments>(args)`, checked against `expected` when the call
    /// is checked against a type, which generic callees use to infer their type arguments.
    fn infer_call_expr(
        &mut self,
        callee: &Expr,
        type_arguments: &[prism_ast::types::Type],
        args: &[CallArg],
        span: Span,
        expected: Option<Type>,
    ) -> ExpressionId {
        // `Heap(...)` always constructs a box: the name is built in, like a type name, which
        // no local value hides (D2 of `heap-boxes`).
        if matches!(&callee.kind, ExprKind::Ident(name) if name == HEAP) {
            return self.infer_heap_construction(type_arguments, args, expected.as_ref(), span);
        }
        // In an extension body, `name(...)` may call a field or another extension of `this`.
        if let Some(member) = self.implicit_this_callee(callee) {
            return self.infer_call_expr(&member, type_arguments, args, span, expected);
        }
        // A call of a rule-typed intrinsic of the library, found as any name is, so a local,
        // parameter, declaration or import of the name shadows the prelude's (D4 and D6 of
        // `standard-library`): `print` and `println` take any argument with a text form, and
        // `listOf` any number of elements.
        match self.intrinsic_callee(callee) {
            Some(builtin @ (BuiltinFunction::Print | BuiltinFunction::PrintLine)) => {
                return self.infer_output_call(builtin, type_arguments, args, span);
            }
            Some(BuiltinFunction::ListOf) => {
                return self.infer_list_construction(type_arguments, args, expected.as_ref(), span);
            }
            Some(BuiltinFunction::Remember) => {
                return self.infer_remember(type_arguments, args, expected.as_ref(), span);
            }
            _ => {}
        }
        // `recv.name(...)`: a field, the built-in `clone()` or an extension function.
        if let ExprKind::MemberAccess {
            target,
            member,
            is_safe_nav,
        } = &callee.kind
        {
            // A receiver that is a field through implicit `this` is checked, and reported, as
            // the explicit `this.name` it stands for.
            if let Some(field) = self.implicit_this_field(target) {
                let callee = Expr::new(
                    ExprKind::MemberAccess {
                        target: Box::new(field),
                        member: member.clone(),
                        is_safe_nav: *is_safe_nav,
                    },
                    callee.span,
                );
                return self.infer_call_expr(&callee, type_arguments, args, span, expected);
            }
            if self.type_name_symbol(target).is_none() {
                let call = MethodCall {
                    callee,
                    target,
                    member,
                    is_safe_nav: *is_safe_nav,
                    type_arguments,
                    args,
                    expected: expected.as_ref(),
                };
                return self.infer_method_call(call, span);
            }
        }
        // A struct name is its constructor; an enum name is only a namespace for its variants.
        let type_name = self.type_name_symbol(callee);
        let (callee_node, callee_ty) = match &type_name {
            Some(sym) if !self.check_visibility(sym, callee.span) => (None, Type::Error),
            Some(sym) => match sym.kind {
                SymbolKind::Struct { .. } => (None, sym.ty()),
                _ => {
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message: format!(
                            "enum '{}' cannot be instantiated directly without a variant",
                            sym.name
                        ),
                        span,
                    });
                    (None, Type::Error)
                }
            },
            None => {
                // The callee may name a generic function or variant (D8 of `generics`).
                if let ExprKind::Ident(name) = &callee.kind {
                    self.ensure_inferred(name, callee.span);
                }
                self.generic_callee = matches!(
                    callee.kind,
                    ExprKind::Ident(_) | ExprKind::MemberAccess { .. }
                );
                // A function value is called in place. A callee that is not a name or a path
                // is a temporary, so what its branches and `?:` operands give is consumed (the
                // `ownership` spec, "Consuming Positions").
                // A signature-typed intrinsic function is named in its call, whose callee
                // becomes its built-in (D6 of `standard-library`).
                self.intrinsic_call = self.intrinsic_callee(callee).is_some();
                let id = match callee.kind {
                    ExprKind::Ident(_) | ExprKind::MemberAccess { .. } => self.infer_expr(callee),
                    _ => self.infer_read(callee),
                };
                self.intrinsic_call = false;
                self.generic_callee = false;
                (Some(id), self.ty(id))
            }
        };
        self.call_with_type(
            callee,
            callee_node,
            callee_ty,
            type_name.is_some(),
            type_arguments,
            args,
            expected.as_ref(),
            span,
        )
    }

    /// Checks `listOf(e1, ..., en)`, typed as a call of `fun <T> listOf(e1: T, ..., en: T):
    /// List<T>` with as many parameters as arguments (D4 of `collections`): the element type
    /// comes from the written type argument, else from an expected `List<T>` (or `List<T>?`),
    /// else from the joined arguments, each checked against it in a consuming position.
    /// Named arguments are `E4004`; an element type nothing gives is `E4013`.
    fn infer_list_construction(
        &mut self,
        type_arguments: &[prism_ast::types::Type],
        args: &[CallArg],
        expected: Option<&Type>,
        span: Span,
    ) -> ExpressionId {
        if args.iter().any(|arg| arg.name.is_some()) {
            self.errors.push(TypeError::CallSignatureMismatch {
                message: format!("'{}' takes no named arguments", LIST_OF),
                span,
            });
            let children = args
                .iter()
                .map(|arg| self.with_origin(Origin::Value, |c| c.infer_expr(&arg.value)))
                .collect();
            return self.error_node(children, span);
        }
        let parameter = BuiltinFunction::list_element_parameter();
        let element = parameter.as_type();
        let signature = GenericSignature {
            owner: format!("'{}'", LIST_OF),
            type_parameters: vec![parameter],
            params: vec![element.clone(); args.len()],
            ret: Type::List(Box::new(element)),
            receiver: None,
        };
        // A list assigned to a nullable list takes its element type from it too.
        let expected = match expected {
            Some(Type::Nullable(inner)) if matches!(**inner, Type::List(_)) => Some(&**inner),
            other => other,
        };
        let names: Vec<String> = (1..=args.len()).map(|i| format!("e{}", i)).collect();
        let has_default = vec![false; args.len()];
        let checked = self.check_generic_call_arguments(
            &signature,
            type_arguments,
            expected,
            &names,
            &has_default,
            args,
            span,
        );
        match checked {
            Err(CheckedArguments(children)) => self.error_node(children, span),
            Ok((
                NormalizedArguments {
                    arguments,
                    evaluation_order,
                },
                solution,
            )) => {
                let ty = signature.ret.substitute(&solution);
                let call = Call {
                    callee: Callee::Builtin(BuiltinFunction::ListOf),
                    arguments,
                    parameters: parameter_list(&names, &signature.params),
                    escaping: vec![false; signature.params.len()],
                    type_arguments: solution,
                    evaluation_order,
                    has_receiver: false,
                    has_context: false,
                    is_compose: false,
                };
                self.add(ExpressionKind::Call(call), ty, span)
            }
        }
    }

    /// Checks `remember(k1, ..., kn, calculation)`, typed as a call of `fun <T : Copy, K1 :
    /// Copy, ..., Kn : Copy> remember(k1: K1, ..., kn: Kn, calculation: () -> T): T` with up to
    /// three keys before the calculation (D12 of `compose-transform`), so a key or a value that
    /// is not `Copy` is `E4021` and a borrowed one `E4022`, through the bounds of the type
    /// parameters. Named arguments and a fourth key are `E4004`; outside a composable context
    /// the call is `E4008`. Its type arguments are the value's type, then each key's.
    fn infer_remember(
        &mut self,
        type_arguments: &[prism_ast::types::Type],
        args: &[CallArg],
        expected: Option<&Type>,
        span: Span,
    ) -> ExpressionId {
        const MAXIMUM_KEYS: usize = 3;
        let name = BuiltinFunction::Remember.name();
        let problem = if args.iter().any(|arg| arg.name.is_some()) {
            Some(format!("'{}' takes no named arguments", name))
        } else if args.is_empty() {
            Some(format!(
                "'{}' takes a calculation, as in 'remember {{ 0 }}'",
                name
            ))
        } else if args.len() > MAXIMUM_KEYS + 1 {
            Some(format!(
                "'{}' takes at most {} keys before its calculation, found {}",
                name,
                MAXIMUM_KEYS,
                args.len() - 1
            ))
        } else {
            None
        };
        if let Some(message) = problem {
            self.errors
                .push(TypeError::CallSignatureMismatch { message, span });
            let children = args
                .iter()
                .map(|arg| self.with_origin(Origin::Value, |c| c.infer_expr(&arg.value)))
                .collect();
            return self.error_node(children, span);
        }
        if !self.is_in_compose() {
            self.report_compose_context(name.to_string(), span);
        }
        let keys = args.len() - 1;
        let value_parameter = TypeParameter {
            index: 0,
            name: "T".to_string(),
            bound: Some(prism_ast::TypeMarker::Copy),
        };
        let mut type_parameters = vec![value_parameter.clone()];
        let mut params = Vec::new();
        let mut names = Vec::new();
        for key in 0..keys {
            let parameter = TypeParameter {
                index: (key + 1) as u32,
                name: format!("K{}", key + 1),
                bound: Some(prism_ast::TypeMarker::Copy),
            };
            params.push(parameter.as_type());
            names.push(format!("key{}", key + 1));
            type_parameters.push(parameter);
        }
        params.push(Type::Fn {
            is_compose: false,
            receiver: false,
            params: Vec::new(),
            escaping: Vec::new(),
            ret: Box::new(value_parameter.as_type()),
        });
        names.push("calculation".to_string());
        let signature = GenericSignature {
            owner: format!("'{}'", name),
            type_parameters,
            params,
            ret: value_parameter.as_type(),
            receiver: None,
        };
        let has_default = vec![false; args.len()];
        let checked = self.check_generic_call_arguments(
            &signature,
            type_arguments,
            expected,
            &names,
            &has_default,
            args,
            span,
        );
        match checked {
            Err(CheckedArguments(children)) => self.error_node(children, span),
            Ok((
                NormalizedArguments {
                    arguments,
                    evaluation_order,
                },
                solution,
            )) => {
                let ty = signature.ret.substitute(&solution);
                let call = Call {
                    callee: Callee::Builtin(BuiltinFunction::Remember),
                    arguments,
                    parameters: parameter_list(&names, &signature.params),
                    escaping: vec![false; signature.params.len()],
                    type_arguments: solution,
                    evaluation_order,
                    has_receiver: false,
                    has_context: false,
                    is_compose: true,
                };
                self.add(ExpressionKind::Call(call), ty, span)
            }
        }
    }

    /// Checks a call of the output built-in `builtin`: `print` takes one positional argument and
    /// `println` zero or one, of a printable type (the `type-system` spec, "Output Built-In
    /// Arguments"). The call has type `Unit`.
    fn infer_output_call(
        &mut self,
        builtin: BuiltinFunction,
        type_arguments: &[prism_ast::types::Type],
        args: &[CallArg],
        span: Span,
    ) -> ExpressionId {
        let name = builtin.name();
        self.reject_type_arguments(&format!("'{}'", name), type_arguments);
        let maximum = 1;
        let minimum = usize::from(builtin == BuiltinFunction::Print);
        let problem = if args.iter().any(|arg| arg.name.is_some()) {
            Some(format!("'{}' takes no named arguments", name))
        } else if args.len() < minimum || args.len() > maximum {
            let expected = if minimum == maximum {
                "exactly one argument"
            } else {
                "at most one argument"
            };
            Some(format!(
                "'{}' takes {}, found {}",
                name,
                expected,
                args.len()
            ))
        } else {
            None
        };
        if let Some(message) = problem {
            self.errors
                .push(TypeError::CallSignatureMismatch { message, span });
            let children = args
                .iter()
                .map(|arg| self.with_origin(Origin::Value, |c| c.infer_expr(&arg.value)))
                .collect();
            return self.error_node(children, span);
        }
        let mut arguments = Vec::new();
        let mut parameters = Vec::new();
        if let Some(arg) = args.first() {
            match self.check_printed_argument(&arg.value) {
                Ok(value) => {
                    parameters.push((None, self.ty(value)));
                    arguments.push(Argument::Written(value));
                }
                Err(value) => return self.error_node(vec![value], span),
            }
        }
        let call = Call {
            callee: Callee::Builtin(builtin),
            evaluation_order: (0..arguments.len()).collect(),
            escaping: vec![false; parameters.len()],
            arguments,
            parameters,
            type_arguments: Vec::new(),
            has_receiver: false,
            has_context: false,
            is_compose: false,
        };
        self.add(ExpressionKind::Call(call), Type::Unit, span)
    }

    /// Checks `Heap(value)` or `Heap<T>(value)`, a call of the built-in box constructor, typed as
    /// `fun <T> Heap(value: T): Heap<T>` (D2 of `heap-boxes`): the contents' type comes from the
    /// written type argument, else from an expected `Heap<T>` (or `Heap<T>?`), else from the
    /// argument, which is checked against it in a consuming position. It takes exactly one
    /// positional argument (`E4004`), and the contents' type may hold no borrow (`E4022`).
    fn infer_heap_construction(
        &mut self,
        type_arguments: &[prism_ast::types::Type],
        args: &[CallArg],
        expected: Option<&Type>,
        span: Span,
    ) -> ExpressionId {
        if args.iter().any(|arg| arg.name.is_some()) {
            self.errors.push(TypeError::CallSignatureMismatch {
                message: format!("'{}' takes no named arguments", HEAP),
                span,
            });
            let children = args
                .iter()
                .map(|arg| self.with_origin(Origin::Value, |c| c.infer_expr(&arg.value)))
                .collect();
            return self.error_node(children, span);
        }
        let parameter = BuiltinFunction::heap_contents_parameter();
        let signature = GenericSignature {
            owner: format!("'{}'", HEAP),
            type_parameters: vec![parameter.clone()],
            params: vec![parameter.as_type()],
            ret: Type::Heap(Box::new(parameter.as_type())),
            receiver: None,
        };
        // A box assigned to a nullable box takes its contents' type from it too.
        let expected = match expected {
            Some(Type::Nullable(inner)) if matches!(**inner, Type::Heap(_)) => Some(&**inner),
            other => other,
        };
        let names = vec!["value".to_string()];
        let checked = self.check_generic_call_arguments(
            &signature,
            type_arguments,
            expected,
            &names,
            &[false],
            args,
            span,
        );
        match checked {
            Err(CheckedArguments(children)) => self.error_node(children, span),
            Ok((
                NormalizedArguments {
                    arguments,
                    evaluation_order,
                },
                solution,
            )) => {
                let ty = signature.ret.substitute(&solution);
                let call = Call {
                    callee: Callee::Builtin(BuiltinFunction::HeapAllocate),
                    arguments,
                    parameters: parameter_list(&names, &signature.params),
                    escaping: vec![false; signature.params.len()],
                    type_arguments: solution,
                    evaluation_order,
                    has_receiver: false,
                    has_context: false,
                    is_compose: false,
                };
                self.add(ExpressionKind::Call(call), ty, span)
            }
        }
    }

    /// Checks the argument of an output built-in, adjusted as for a parameter of its type: a
    /// `Copy` scalar is passed by value, copied out of references, and a `String` is borrowed
    /// implicitly, as for a `&String` parameter, without being moved. Any other type is printed
    /// through its text form, the rule of template parts (D9 of `standard-library`): the
    /// `toString()` that `expr.toString()` resolves, called on a shared borrow, its result a
    /// temporary of the statement that is borrowed. A type without one is `E4026`; the error
    /// result is the node checked so far.
    fn check_printed_argument(&mut self, expr: &Expr) -> Result<ExpressionId, ExpressionId> {
        let is_place = self.is_place(expr);
        let found = self.with_origin(Origin::Value, |c| {
            c.with_consuming(!is_place, |c| c.infer_expr(expr))
        });
        let found_ty = self.ty(found);
        // Printed values are read through references and boxes (D4 of `heap-boxes`).
        let referent = peel_indirection(&found_ty);
        let parameter = match referent {
            Type::Int | Type::Float | Type::Bool | Type::Char => referent.clone(),
            Type::String => Type::Reference {
                mutable: false,
                inner: Box::new(Type::String),
            },
            Type::Error => return Err(found),
            _ => {
                let is_place = is_place || self.is_optional_place(expr);
                let text = self.check_text_form(expr, found, is_place, TextFormUse::Printed);
                if self.ty(text) == Type::Error {
                    return Err(text);
                }
                return Ok(self.borrow(false, BorrowOrigin::Argument, text));
            }
        };
        Ok(self.with_origin(Origin::Argument, |c| {
            c.fit(expr, found, &parameter, Site::Argument { is_place }, false)
        }))
    }

    /// The parameter names, defaults and type parameters of the callee of a call: a declared
    /// function or payload variant named by `callee` (whose node is `callee_node`), or nothing
    /// for a function-typed value.
    fn callee_declaration(
        &self,
        callee: &Expr,
        callee_node: Option<ExpressionId>,
    ) -> (Vec<String>, Vec<bool>, Vec<TypeParameter>) {
        if let ExprKind::Ident(name) = &callee.kind {
            if let Some(SymbolKind::Function {
                param_names,
                has_default,
                type_parameters,
                ..
            }) = self.env.lookup(name).map(|sym| &sym.kind)
            {
                return (
                    param_names.clone(),
                    has_default.clone(),
                    type_parameters.clone(),
                );
            }
        }
        if let Some(ExpressionKind::VariantConstructor { enum_id, .. }) =
            callee_node.map(|node| self.builder.kind(node))
        {
            return (
                Vec::new(),
                Vec::new(),
                self.types.parameters(enum_id).to_vec(),
            );
        }
        (Vec::new(), Vec::new(), Vec::new())
    }

    /// The context parameter type of the declared function `callee` names, if it has one.
    fn callee_context(&self, callee: &Expr) -> Option<Type> {
        let ExprKind::Ident(name) = &callee.kind else {
            return None;
        };
        self.env.lookup(name)?.context().cloned()
    }

    /// Puts the context argument `context` and its parameter of type `&context_ty` before the
    /// other arguments of a call (D9 of `receiver-lambdas`), evaluated first.
    fn prepend_context(
        context: (ExpressionId, Type),
        arguments: &mut Vec<Argument>,
        parameters: &mut Vec<(Option<String>, Type)>,
        escaping: &mut Vec<bool>,
        evaluation_order: &mut Vec<usize>,
    ) {
        let (argument, context_ty) = context;
        arguments.insert(0, Argument::Written(argument));
        parameters.insert(
            0,
            (
                Some(CONTEXT.to_string()),
                Type::Reference {
                    mutable: false,
                    inner: Box::new(context_ty),
                },
            ),
        );
        escaping.insert(0, false);
        for index in evaluation_order.iter_mut() {
            *index += 1;
        }
        evaluation_order.insert(0, 0);
    }

    /// Checks a call of `callee`, of type `callee_ty`, with `args`: a function or function-typed
    /// value whose node is `callee_node`, or a struct constructor when `is_constructor`.
    #[allow(clippy::too_many_arguments)]
    fn call_with_type(
        &mut self,
        callee: &Expr,
        callee_node: Option<ExpressionId>,
        callee_ty: Type,
        is_constructor: bool,
        type_arguments: &[prism_ast::types::Type],
        args: &[CallArg],
        expected: Option<&Type>,
        span: Span,
    ) -> ExpressionId {
        match callee_ty {
            Type::Fn {
                is_compose,
                receiver,
                params,
                escaping,
                ret,
            } => {
                // Composable context enforcement (first-class compose invariant, E4008)
                if is_compose && !self.is_in_compose() {
                    let callee_name = if let ExprKind::Ident(n) = &callee.kind {
                        n.clone()
                    } else {
                        "<closure>".to_string()
                    };
                    self.report_compose_context(callee_name, span);
                }

                let (param_names, has_default, type_parameters) =
                    self.callee_declaration(callee, callee_node);
                let callee_node = callee_node.expect("a function type comes from a node");
                // A value with a receiver called without it takes the innermost `this` (D5 of
                // `receiver-lambdas`); a declared function never has a receiver type.
                if receiver
                    && param_names.is_empty()
                    && args.len() + 1 == params.len()
                    && args.iter().all(|arg| arg.name.is_none())
                {
                    return self.call_receiver_value_with_this(
                        callee,
                        callee_node,
                        type_arguments,
                        args,
                        span,
                    );
                }
                // A function with a context parameter takes its context argument first (D7 of
                // `receiver-lambdas`).
                let context = match self.callee_context(callee) {
                    Some(context_ty) => {
                        match self.context_argument(&callee.to_string(), &context_ty, callee.span) {
                            Some(argument) => Some((argument, context_ty)),
                            None => {
                                let mut children = vec![callee_node];
                                for arg in args {
                                    children.push(self.infer_expr(&arg.value));
                                }
                                return self.error_node(children, span);
                            }
                        }
                    }
                    None => None,
                };
                let has_context = context.is_some();
                if type_parameters.is_empty() {
                    self.reject_type_arguments(&format!("'{}'", callee), type_arguments);
                    let checked =
                        self.check_call_arguments(&params, &param_names, &has_default, args, span);
                    return match checked {
                        Err(CheckedArguments(children)) => {
                            let mut all = vec![callee_node];
                            all.extend(children);
                            self.error_node(all, span)
                        }
                        Ok(NormalizedArguments {
                            mut arguments,
                            mut evaluation_order,
                        }) => {
                            let mut parameters = parameter_list(&param_names, &params);
                            let mut escaping = escaping;
                            if let Some(context) = context {
                                Self::prepend_context(
                                    context,
                                    &mut arguments,
                                    &mut parameters,
                                    &mut escaping,
                                    &mut evaluation_order,
                                );
                            }
                            self.build_call_with_context(
                                callee_node,
                                arguments,
                                parameters,
                                escaping,
                                Vec::new(),
                                evaluation_order,
                                is_compose,
                                has_context,
                                *ret,
                                span,
                            )
                        }
                    };
                }
                let signature = GenericSignature {
                    owner: format!("'{}'", callee),
                    type_parameters,
                    params,
                    ret: *ret,
                    receiver: None,
                };
                let checked = self.check_generic_call_arguments(
                    &signature,
                    type_arguments,
                    expected,
                    &param_names,
                    &has_default,
                    args,
                    span,
                );
                match checked {
                    Err(CheckedArguments(children)) => {
                        let mut all = vec![callee_node];
                        all.extend(children);
                        self.error_node(all, span)
                    }
                    Ok((
                        NormalizedArguments {
                            mut arguments,
                            mut evaluation_order,
                        },
                        solution,
                    )) => {
                        // The call records the declared parameter types and the type
                        // arguments (D9 of `generics`).
                        let mut parameters = parameter_list(&param_names, &signature.params);
                        let ret = signature.ret.substitute(&solution);
                        let mut escaping = escaping;
                        if let Some(context) = context {
                            Self::prepend_context(
                                context,
                                &mut arguments,
                                &mut parameters,
                                &mut escaping,
                                &mut evaluation_order,
                            );
                        }
                        self.build_call_with_context(
                            callee_node,
                            arguments,
                            parameters,
                            escaping,
                            solution,
                            evaluation_order,
                            is_compose,
                            has_context,
                            ret,
                            span,
                        )
                    }
                }
            }
            Type::Struct { id, .. } if is_constructor => {
                self.construct_struct(id, type_arguments, args, expected, span)
            }
            // A function value is called in place, so it is reached through its references and
            // boxes, as a field is.
            Type::Reference { .. } | Type::Heap(_)
                if callee_node.is_some()
                    && matches!(peel_indirection(&callee_ty), Type::Fn { .. }) =>
            {
                let reached = self.peel_indirection_nodes(callee_node.expect("checked above"));
                let reached_ty = self.ty(reached);
                self.call_with_type(
                    callee,
                    Some(reached),
                    reached_ty,
                    false,
                    type_arguments,
                    args,
                    expected,
                    span,
                )
            }
            Type::Error => self.error_node(callee_node.into_iter().collect(), span),
            other => {
                self.errors.push(TypeError::CallSignatureMismatch {
                    message: format!("type '{}' cannot be called as a function", other),
                    span,
                });
                self.error_node(callee_node.into_iter().collect(), span)
            }
        }
    }

    /// Checks the arguments of a call of the generic `signature`, inferring its type arguments
    /// (D8 of `generics`). Returns the normalized arguments and the type arguments.
    #[allow(clippy::too_many_arguments)]
    fn check_generic_call_arguments(
        &mut self,
        signature: &GenericSignature,
        type_arguments: &[prism_ast::types::Type],
        expected: Option<&Type>,
        param_names: &[String],
        has_default: &[bool],
        args: &[CallArg],
        span: Span,
    ) -> Result<(NormalizedArguments, Vec<Type>), CheckedArguments> {
        let mut state = self.start_generic_arguments(signature, type_arguments, expected);
        let mut normalized = self.check_call_arguments_with(
            signature.params.len(),
            param_names,
            has_default,
            args,
            span,
            &mut |checker, index, value| checker.generic_argument(&mut state, index, value),
        )?;
        let (solution, fitted) = self.finish_generic_arguments(state, span);
        for (index, node) in fitted {
            normalized.arguments[index] = Argument::Written(node);
        }
        Ok((normalized, solution))
    }

    /// Reports `E4020` for type arguments written for `owner`, which is not generic.
    fn reject_type_arguments(&mut self, owner: &str, type_arguments: &[prism_ast::types::Type]) {
        if let (Some(first), Some(last)) = (type_arguments.first(), type_arguments.last()) {
            self.errors.push(TypeError::WrongTypeArgumentCount {
                owner: owner.to_string(),
                expected: 0,
                found: type_arguments.len(),
                span: first.span.merge(last.span),
            });
        }
    }

    /// Starts inferring the type arguments of a call of `signature` (D8 of `generics`, steps 1
    /// to 3): the written type arguments, when their number is right, fix every unknown;
    /// otherwise the receiver and the expected type give exact solutions.
    fn start_generic_arguments<'a>(
        &mut self,
        signature: &GenericSignature,
        type_arguments: &[prism_ast::types::Type],
        expected: Option<&Type>,
    ) -> GenericArguments<'a> {
        let count = signature.type_parameters.len();
        let mut unknowns = Unknowns::new(count);
        let mut explicit = false;
        if let (Some(first), Some(last)) = (type_arguments.first(), type_arguments.last()) {
            for argument in type_arguments {
                self.check_annotation_names(argument);
            }
            if type_arguments.len() != count {
                self.errors.push(TypeError::WrongTypeArgumentCount {
                    owner: signature.owner.clone(),
                    expected: count,
                    found: type_arguments.len(),
                    span: first.span.merge(last.span),
                });
            } else {
                explicit = true;
                for (index, argument) in type_arguments.iter().enumerate() {
                    let ty = self.lower_type(argument);
                    if ty.reference_outside_functions().is_some() {
                        self.errors.push(TypeError::BorrowInTypeArgument {
                            argument: ty.clone(),
                            owner: signature.owner.clone(),
                            span: argument.span,
                        });
                    }
                    unknowns.fix(index, ty);
                }
            }
        }
        if !explicit {
            if let Some((declared, reached)) = &signature.receiver {
                unknowns.match_exact(declared, reached);
            }
            if let Some(expected) = expected {
                unknowns.match_exact(&signature.ret, expected);
            }
        }
        GenericArguments {
            owner: signature.owner.clone(),
            type_parameters: signature.type_parameters.clone(),
            params: signature.params.clone(),
            unknowns,
            explicit,
            synthesized: Vec::new(),
            deferred: Vec::new(),
        }
    }

    /// Checks the written argument `expr` of parameter `index` of a generic call (D8 of
    /// `generics`, step 4). When the parameter's declared type mentions an unknown with no exact
    /// solution, the argument is inferred and its type gives lower bounds; its fit waits for the
    /// solution. A closure passed for a function type mentioning such an unknown is deferred.
    /// Any other argument is checked against its parameter type now.
    fn generic_argument<'a>(
        &mut self,
        state: &mut GenericArguments<'a>,
        index: usize,
        expr: &'a Expr,
    ) -> Option<ExpressionId> {
        let declared = state.params[index].clone();
        if !state.unknowns.mentions_inexact(&declared) {
            let ty = declared.substitute(&state.unknowns.partial());
            return Some(self.check_argument(expr, &ty, Origin::Argument));
        }
        if matches!(expr.kind, ExprKind::Closure { .. }) && closure_signature(&declared).is_some() {
            state.deferred.push((index, expr));
            return None;
        }
        let (found, site, deferred_move) = if is_reference_parameter(&declared) {
            let is_place = self.is_place(expr);
            let found = self.with_origin(Origin::Value, |c| {
                c.with_consuming(!is_place, |c| c.infer_expr(expr))
            });
            (found, Site::Argument { is_place }, false)
        } else {
            let deferred_move = is_move_leaf(expr);
            let found = self.with_origin(Origin::Value, |c| {
                c.with_consuming(!deferred_move, |c| c.infer_expr(expr))
            });
            (found, Site::Value, deferred_move)
        };
        let found_ty = self.ty(found);
        state.unknowns.match_lower(&declared, &found_ty, expr.span);
        state.synthesized.push(SynthesizedArgument {
            index,
            expr,
            found,
            site,
            deferred_move,
        });
        Some(found)
    }

    /// Finishes the arguments of a generic call (D8 of `generics`, steps 5 to 8): checks the
    /// deferred closures, solves the unknowns, reports those left unsolved (`E4013`), borrows
    /// in the solution (`E4022`) and unsatisfied bounds (`E4021`), and fits each synthesized
    /// argument to its parameter type. Returns the type arguments and the final node of every
    /// argument that was deferred or synthesized, by parameter index.
    fn finish_generic_arguments(
        &mut self,
        mut state: GenericArguments,
        span: Span,
    ) -> (Vec<Type>, Vec<(usize, ExpressionId)>) {
        let mut fitted = Vec::new();
        let deferred = std::mem::take(&mut state.deferred);
        for (index, expr) in deferred {
            let declared = state.params[index].clone();
            let node = self.check_deferred_closure(&mut state, &declared, expr);
            state.synthesized.push(SynthesizedArgument {
                index,
                expr,
                found: node,
                site: Site::Value,
                deferred_move: false,
            });
        }
        let (solution, unsolved, _) = state.unknowns.solve();
        if let Some(unsolved) = unsolved {
            let parameter = &state.type_parameters[unsolved.index];
            let help = if state.owner == format!("'{}'", LIST_OF) {
                LIST_ELEMENT_HELP
            } else {
                TYPE_ARGUMENTS_HELP
            };
            self.errors.push(TypeError::CannotInferType {
                help: Some(help.to_string()),
                message: format!(
                    "cannot infer the type argument '{}' of {}",
                    parameter.name, state.owner
                ),
                span,
            });
        }
        if !state.explicit {
            for argument in &solution {
                if argument.reference_outside_functions().is_some() {
                    self.errors.push(TypeError::BorrowInTypeArgument {
                        argument: argument.clone(),
                        owner: state.owner.clone(),
                        span,
                    });
                }
            }
        }
        for (parameter, argument) in state.type_parameters.iter().zip(&solution) {
            let satisfied = match parameter.bound {
                None => true,
                Some(TypeMarker::Copy) => self.is_copy(argument),
                Some(TypeMarker::Clone) => self.is_clone(argument),
            };
            if let (false, Some(bound)) = (satisfied, parameter.bound) {
                self.errors.push(TypeError::UnsatisfiedBound {
                    argument: argument.clone(),
                    parameter: parameter.name.clone(),
                    bound,
                    owner: state.owner.clone(),
                    span,
                });
            }
        }
        for argument in std::mem::take(&mut state.synthesized) {
            let expected = state.params[argument.index].substitute(&solution);
            let node = self.with_origin(Origin::Argument, |c| {
                c.fit(
                    argument.expr,
                    argument.found,
                    &expected,
                    argument.site,
                    argument.deferred_move,
                )
            });
            fitted.push((argument.index, node));
        }
        (solution, fitted)
    }

    /// Checks the closure `expr`, passed for a parameter of declared type `declared`, after the
    /// other arguments (D8 of `generics`, step 6): its parameters take their solved types, and a
    /// return type mentioning an unknown without an exact solution is inferred from the body,
    /// giving it a lower bound.
    fn check_deferred_closure(
        &mut self,
        state: &mut GenericArguments,
        declared: &Type,
        expr: &Expr,
    ) -> ExpressionId {
        let (is_compose, receiver, params, escaping, ret) =
            closure_signature(declared).expect("deferred closures have a function type");
        let ExprKind::Closure {
            params: written,
            arrow,
            body,
        } = &expr.kind
        else {
            unreachable!("only closure literals are deferred")
        };
        if let Some(unsolved) = params.iter().find(|p| state.unknowns.mentions_unsolved(p)) {
            self.errors.push(TypeError::CannotInferType {
                help: Some(TYPE_ARGUMENTS_HELP.to_string()),
                message: format!(
                    "cannot infer the type '{}' of a parameter of this closure, passed to {}",
                    unsolved, state.owner
                ),
                span: expr.span,
            });
            return self.error_node(Vec::new(), expr.span);
        }
        let partial = state.unknowns.partial();
        let params: Vec<Type> = params.iter().map(|p| p.substitute(&partial)).collect();
        let infer_return = state.unknowns.mentions_inexact(&ret);
        let expected_return = (!infer_return).then(|| ret.substitute(&partial));
        let closure_return = match &expected_return {
            Some(ret) => ClosureReturn::Expected(ret),
            // A by-value type parameter takes the referent of a borrowed result, as `match_lower`
            // takes the referent of a borrowed argument.
            None if matches!(ret, Type::Parameter(_)) => ClosureReturn::InferredReferent,
            None => ClosureReturn::Inferred,
        };
        let node = self.check_closure(
            written,
            arrow.is_some(),
            body,
            is_compose,
            receiver,
            &params,
            &escaping,
            closure_return,
            expr.span,
        );
        if infer_return {
            if let Type::Fn { ret: found, .. } = self.ty(node) {
                state.unknowns.match_lower(&ret, &found, expr.span);
            }
        }
        node
    }

    /// The node of a call whose normalized arguments are `arguments` (D8 of `typed-tree`). A
    /// call of a declared function names it by its path, and a payload variant called as a
    /// constructor is a construction; both take over the callee's node. Any other callee is a
    /// function-typed value read in place.
    #[allow(clippy::too_many_arguments)]
    fn build_call(
        &mut self,
        callee_node: ExpressionId,
        arguments: Vec<Argument>,
        parameters: Vec<(Option<String>, Type)>,
        escaping: Vec<bool>,
        type_arguments: Vec<Type>,
        evaluation_order: Vec<usize>,
        is_compose: bool,
        ret: Type,
        span: Span,
    ) -> ExpressionId {
        self.build_call_with_context(
            callee_node,
            arguments,
            parameters,
            escaping,
            type_arguments,
            evaluation_order,
            is_compose,
            false,
            ret,
            span,
        )
    }

    /// [`Self::build_call`], whose first argument is a context argument when `has_context`.
    #[allow(clippy::too_many_arguments)]
    fn build_call_with_context(
        &mut self,
        callee_node: ExpressionId,
        arguments: Vec<Argument>,
        parameters: Vec<(Option<String>, Type)>,
        escaping: Vec<bool>,
        type_arguments: Vec<Type>,
        evaluation_order: Vec<usize>,
        is_compose: bool,
        has_context: bool,
        ret: Type,
        span: Span,
    ) -> ExpressionId {
        match self.builder.kind(callee_node).clone() {
            ExpressionKind::FunctionReference(path) => {
                // An external is called by its symbol (D5 of `standard-library`), and an
                // intrinsic is its built-in.
                let intrinsic = if path.receiver.is_none() {
                    self.intrinsic_function(&path.module, &path.name)
                } else {
                    None
                };
                let callee = if let Some(builtin) = intrinsic {
                    self.reject_composition_in_composable(builtin, span);
                    Callee::Builtin(builtin)
                } else if self.is_external(&path) {
                    Callee::External(path)
                } else {
                    Callee::Function(path)
                };
                let call = Call {
                    callee,
                    arguments,
                    parameters,
                    escaping,
                    type_arguments,
                    evaluation_order,
                    has_receiver: false,
                    has_context,
                    is_compose,
                };
                self.builder
                    .replace(callee_node, ExpressionKind::Call(call), ret, span);
                callee_node
            }
            ExpressionKind::VariantConstructor {
                enum_id,
                index,
                name,
            } => {
                let fields = arguments
                    .iter()
                    .zip(&parameters)
                    .filter_map(|(argument, (_, ty))| match argument {
                        Argument::Written(value) => Some(ConstructField {
                            name: None,
                            ty: ty.clone(),
                            value: *value,
                        }),
                        Argument::Default => None,
                    })
                    .collect();
                let kind = ExpressionKind::Construct {
                    kind: ConstructKind::Variant {
                        enum_id,
                        index,
                        name,
                    },
                    fields,
                    evaluation_order,
                };
                self.builder.replace(callee_node, kind, ret, span);
                callee_node
            }
            _ => {
                let call = Call {
                    callee: Callee::Value(callee_node),
                    arguments,
                    parameters,
                    escaping,
                    type_arguments,
                    evaluation_order,
                    has_receiver: false,
                    has_context: false,
                    is_compose,
                };
                self.add(ExpressionKind::Call(call), ret, span)
            }
        }
    }

    /// Checks the construction of struct `id` with `args`, positional or named, building it with
    /// its fields in declaration order (D8 of `typed-tree`). A generic struct's type arguments
    /// are the written ones, or inferred like a call's from the expected type and the arguments
    /// (D8 of `generics`).
    fn construct_struct(
        &mut self,
        id: NominalId,
        type_arguments: &[prism_ast::types::Type],
        args: &[CallArg],
        expected: Option<&Type>,
        span: Span,
    ) -> ExpressionId {
        if let Some(intrinsic) = id.intrinsic() {
            // Only the intrinsic functions of its module make one (D16 of
            // `compose-transform`).
            self.errors.push(TypeError::CallSignatureMismatch {
                message: format!(
                    "'{}' is an intrinsic struct: it has no constructor and no fields",
                    intrinsic.name()
                ),
                span,
            });
            let children = args
                .iter()
                .map(|arg| self.with_origin(Origin::Value, |c| c.infer_expr(&arg.value)))
                .collect();
            return self.error_node(children, span);
        }
        let fields = self.struct_fields(&id).unwrap_or_default();
        let type_parameters = self.types.parameters(&id).to_vec();
        let field_types: Vec<Type> = fields.iter().map(|(_, _, ty)| ty.clone()).collect();
        if type_parameters.is_empty() {
            self.reject_type_arguments(&format!("struct '{}'", id.name), type_arguments);
            let mapped =
                self.construct_struct_with(&id, &fields, args, span, &mut |c, index, value| {
                    Some(c.check_expr_at(value, &field_types[index], Origin::Argument))
                });
            return self.build_construction(id, Vec::new(), &fields, mapped, span);
        }
        let signature = GenericSignature {
            owner: format!("struct '{}'", id.name),
            type_parameters: type_parameters.clone(),
            params: field_types,
            ret: Type::Struct {
                id: id.clone(),
                arguments: parameter_types(&type_parameters),
            },
            receiver: None,
        };
        let mut state = self.start_generic_arguments(&signature, type_arguments, expected);
        let mapped =
            self.construct_struct_with(&id, &fields, args, span, &mut |c, index, value| {
                c.generic_argument(&mut state, index, value)
            });
        let mapped = match mapped {
            Ok((mut values, order)) => {
                let (solution, fitted) = self.finish_generic_arguments(state, span);
                for (index, node) in fitted {
                    values[index] = Some(node);
                }
                (Ok((values, order)), solution)
            }
            Err(children) => (Err(children), Vec::new()),
        };
        self.build_construction(id, mapped.1, &fields, mapped.0, span)
    }

    /// The node of a construction of struct `id` with `type_arguments` whose fields were checked
    /// into `mapped`, or an error node keeping the checked fields.
    fn build_construction(
        &mut self,
        id: NominalId,
        type_arguments: Vec<Type>,
        fields: &[(String, bool, Type)],
        mapped: Result<(Vec<Option<ExpressionId>>, Vec<usize>), CheckedArguments>,
        span: Span,
    ) -> ExpressionId {
        let (values, evaluation_order) = match mapped {
            Ok(mapped) => mapped,
            Err(CheckedArguments(checked)) => return self.error_node(checked, span),
        };
        let fields = fields
            .iter()
            .zip(values)
            .map(|((name, _, ty), value)| ConstructField {
                name: Some(name.clone()),
                ty: ty.clone(),
                value: value.expect("every field is given"),
            })
            .collect();
        let kind = ExpressionKind::Construct {
            kind: ConstructKind::Struct(id.clone()),
            fields,
            evaluation_order,
        };
        let ty = Type::Struct {
            id,
            arguments: type_arguments,
        };
        self.add(kind, ty, span)
    }

    fn construct_struct_with<'a>(
        &mut self,
        id: &NominalId,
        fields: &[(String, bool, Type)],
        args: &'a [CallArg],
        span: Span,
        check: &mut ArgumentChecker<'a, '_>,
    ) -> Result<(Vec<Option<ExpressionId>>, Vec<usize>), CheckedArguments> {
        let name = id.name.clone();
        let mut values: Vec<Option<ExpressionId>> = vec![None; fields.len()];
        let mut evaluation_order = Vec::new();
        let mut checked = Vec::new();
        let mut failed = false;
        let has_named = args.iter().any(|a| a.name.is_some());
        // A trailing lambda binds to the last field (as to a function's last parameter).
        let trailing_lambda_in_order = args
            .last()
            .is_none_or(|last| !last.is_trailing_lambda || args.len() == fields.len());
        if !has_named && trailing_lambda_in_order {
            if args.len() != fields.len() {
                self.errors.push(TypeError::CallSignatureMismatch {
                    message: format!(
                        "struct '{}' expected {} arguments, found {}",
                        name,
                        fields.len(),
                        args.len()
                    ),
                    span,
                });
                return Err(CheckedArguments(Vec::new()));
            }
            for (index, arg) in args.iter().enumerate() {
                let value = check(self, index, &arg.value);
                values[index] = value;
                evaluation_order.push(index);
                checked.extend(value);
            }
        } else {
            let mut field_assigned = vec![false; fields.len()];
            let mut seen_fields = HashSet::new();
            let mut pos_idx = 0;

            for arg in args {
                if arg.is_trailing_lambda {
                    let Some(last) = fields.len().checked_sub(1) else {
                        self.errors.push(TypeError::CallSignatureMismatch {
                            message: format!(
                                "struct '{}' has no fields, so it cannot take a trailing lambda",
                                name
                            ),
                            span: arg.span,
                        });
                        failed = true;
                        continue;
                    };
                    if field_assigned[last] {
                        let given = if seen_fields.contains(&fields[last].0) {
                            "by name"
                        } else {
                            "a positional argument"
                        };
                        self.errors.push(TypeError::CallSignatureMismatch {
                            message: format!(
                                "the trailing lambda is passed to '{}', the last field of struct '{}', which is already given {}",
                                fields[last].0, name, given
                            ),
                            span: arg.span,
                        });
                        failed = true;
                        continue;
                    }
                    field_assigned[last] = true;
                    let value = check(self, last, &arg.value);
                    values[last] = value;
                    evaluation_order.push(last);
                    checked.extend(value);
                } else if let Some(arg_name) = &arg.name {
                    if !seen_fields.insert(arg_name.clone()) {
                        self.errors.push(TypeError::CallSignatureMismatch {
                            message: format!(
                                "duplicate field '{}' provided in struct '{}' instantiation",
                                arg_name, name
                            ),
                            span: arg.span,
                        });
                        failed = true;
                        continue;
                    }
                    if let Some(idx) = fields.iter().position(|(n, _, _)| n == arg_name) {
                        if field_assigned[idx] {
                            self.errors.push(TypeError::CallSignatureMismatch {
                                message: format!(
                                    "field '{}' of struct '{}' is already given a positional argument",
                                    arg_name, name
                                ),
                                span: arg.span,
                            });
                            failed = true;
                        }
                        field_assigned[idx] = true;
                        let value = check(self, idx, &arg.value);
                        values[idx] = value;
                        evaluation_order.push(idx);
                        checked.extend(value);
                    } else {
                        self.errors.push(TypeError::UnknownField {
                            target_type: Type::plain_struct(id.clone()),
                            field_name: arg_name.clone(),
                            span: arg.span,
                            help: None,
                        });
                        failed = true;
                    }
                } else if pos_idx < fields.len() {
                    if field_assigned[pos_idx] {
                        self.errors.push(TypeError::CallSignatureMismatch {
                            message: format!(
                                "positional argument assigns field '{}' of struct '{}', which is already given by name",
                                fields[pos_idx].0, name
                            ),
                            span: arg.span,
                        });
                        failed = true;
                    }
                    field_assigned[pos_idx] = true;
                    let value = check(self, pos_idx, &arg.value);
                    values[pos_idx] = value;
                    evaluation_order.push(pos_idx);
                    checked.extend(value);
                    pos_idx += 1;
                } else {
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message: format!(
                            "struct '{}' expected at most {} arguments, found more",
                            name,
                            fields.len()
                        ),
                        span: arg.span,
                    });
                    failed = true;
                }
            }

            for (idx, (f_name, _, _)) in fields.iter().enumerate() {
                if !field_assigned[idx] {
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message: format!(
                            "missing required field '{}' in struct '{}' instantiation",
                            f_name, name
                        ),
                        span,
                    });
                    failed = true;
                }
            }
        }

        if failed {
            return Err(CheckedArguments(checked));
        }
        Ok((values, evaluation_order))
    }

    /// Checks a call `target.member(args)` (or `target?.member(args)`) whose target is a value,
    /// resolving `member` in the order of D6 of `extension-methods`: a field of the receiver's
    /// struct, the built-in `clone()` of a `Clone` type, the built-in `toString()` of a scalar
    /// and `unbox()` of a box (D6 of `heap-boxes`), the file's own extension, the single
    /// import exporting one, and otherwise the built-in `clone()` (`E4018`) or an unknown member
    /// (`E4006`).
    ///
    /// The receiver is classified like an argument before its type is known: a place, or a `?.`
    /// chain of fields over one, is read in place, and anything else is evaluated into a consumed
    /// temporary.
    fn infer_method_call(&mut self, call: MethodCall, span: Span) -> ExpressionId {
        let MethodCall {
            callee,
            target,
            member,
            is_safe_nav,
            type_arguments,
            args,
            expected,
        } = call;
        let is_place = self.is_place(target) || self.is_optional_place(target);
        let (receiver, writable) = self.with_origin(Origin::Value, |c| {
            if is_place {
                c.infer_place(target)
            } else {
                (c.with_consuming(true, |c| c.infer_expr(target)), true)
            }
        });
        let call = MethodCall {
            callee,
            target,
            member,
            is_safe_nav,
            type_arguments,
            args,
            expected,
        };
        self.method_call_on(call, receiver, is_place, writable, span)
    }

    /// Types the method call `call` on `receiver`, the node of its already inferred target, a
    /// place (`is_place`) that may be written (`writable`) or a value: a field of function type
    /// first, then the built-ins, then an extension (the `extension-functions` spec). A string
    /// template's `toString()` part is resolved here too (D6 of `string-templates`).
    fn method_call_on(
        &mut self,
        call: MethodCall,
        receiver: ExpressionId,
        is_place: bool,
        writable: bool,
        span: Span,
    ) -> ExpressionId {
        let MethodCall {
            callee,
            target,
            member,
            is_safe_nav,
            type_arguments,
            args,
            expected,
        } = call;
        let receiver_ty = self.ty(receiver);
        if receiver_ty == Type::Error {
            return self.give_up_call(receiver, args, span);
        }

        // 1. A field takes precedence over everything else.
        if self.has_field(&receiver_ty, member) {
            let (field, _) = self.build_member_access(receiver, member, is_safe_nav, callee.span);
            let field_ty = self.ty(field);
            return self.call_with_type(
                callee,
                Some(field),
                field_ty,
                false,
                type_arguments,
                args,
                expected,
                span,
            );
        }

        // 2. The built-in `clone()` of a `Clone` type, and `toString()` of a scalar.
        if member == "clone" && self.is_clone(&clone_target(&receiver_ty, is_safe_nav).0) {
            return self.infer_clone_call(receiver, is_safe_nav, args, span);
        }
        let scalar = peel_indirection(&clone_target(&receiver_ty, is_safe_nav).0).clone();
        if member == TO_STRING
            && has_builtin_to_string(&scalar)
            && self.has_intrinsic_function(&scalar, TO_STRING)
        {
            return self.infer_to_string_call(receiver, is_safe_nav, type_arguments, args, span);
        }
        // The built-in `unbox()` of a box behind references (D6 of `heap-boxes`).
        if member == UNBOX && matches!(clone_target(&receiver_ty, is_safe_nav).0, Type::Heap(_)) {
            let call = UnboxCall {
                target,
                receiver,
                is_place,
                is_safe_nav,
                type_arguments,
                args,
            };
            return self.infer_unbox_call(call, span);
        }

        // 3. A local or parameter of a function type with a receiver of the receiver's type,
        // behind any `&`, `&mut` and `Heap<T>` (D5 of `receiver-lambdas`).
        if !is_safe_nav {
            if let Some(value) = self.receiver_value(member, &receiver_ty) {
                let site = ReceiverSite {
                    expr: target,
                    ty: &receiver_ty,
                    is_place,
                    writable,
                    layers: &value.layers,
                    through_nullable: false,
                };
                let argument = self.pass_value_receiver(&site, receiver, &value, member);
                let name = Expr::new(ExprKind::Ident(member.to_string()), callee.span);
                let callee_node = self.infer_expr(&name);
                return self.call_receiver_value(
                    callee,
                    callee_node,
                    argument,
                    type_arguments,
                    args,
                    true,
                    span,
                );
            }
        }

        // 4. An extension on the receiver's type, behind any `&`, `&mut` and `Heap<T>`.
        let (payload, through_nullable) = match peel_references(&receiver_ty) {
            Type::Nullable(inner) => (inner.as_ref().clone(), true),
            other => (other.clone(), false),
        };
        let mut layers = Vec::new();
        let start = if through_nullable {
            // The references around the nullable are looked through too.
            let mut outer = &receiver_ty;
            while let Type::Reference { mutable, inner } = outer {
                layers.push(Layer::Reference { mutable: *mutable });
                outer = inner;
            }
            payload
        } else {
            receiver_ty.clone()
        };
        let (lookup, reached) = self.find_extension_through_indirection(start, member, &mut layers);
        match lookup {
            ExtensionLookup::Ambiguous(modules) => {
                let call = self.give_up_call(receiver, args, span);
                self.errors.push(TypeError::AmbiguousImport {
                    name: format!("{}.{}", reached, member),
                    modules,
                    span: callee.span,
                });
                call
            }
            ExtensionLookup::MissingContext(needed) => {
                let call = self.give_up_call(receiver, args, span);
                let found = self.describe_context();
                self.errors.push(TypeError::MissingContext {
                    function: member.to_string(),
                    needed,
                    found,
                    span: callee.span,
                });
                call
            }
            ExtensionLookup::Found { .. } if through_nullable && !is_safe_nav => {
                let call = self.give_up_call(receiver, args, span);
                self.errors.push(TypeError::UnknownField {
                    target_type: receiver_ty.clone(),
                    field_name: member.to_string(),
                    span: callee.span,
                    help: Some(format!(
                        "'{}' may be null: call the extension with a safe call, '{}?.{}()'",
                        target, target, member
                    )),
                });
                call
            }
            ExtensionLookup::Found { extension, own } => {
                let extension = if own {
                    let key = InferenceKey::Extension {
                        receiver: ReceiverKey::of(&reached),
                        name: member.to_string(),
                        context: extension.context.clone(),
                    };
                    self.ensure_key_inferred(&key, callee.span);
                    // Inference may have refined the return type.
                    match self.find_extension(&reached, member) {
                        ExtensionLookup::Found { extension, .. } => extension,
                        _ => extension,
                    }
                } else {
                    extension
                };
                if !self.check_visibility(&extension.symbol, callee.span) {
                    return self.give_up_call(receiver, args, span);
                }
                // The receiver must match the declared one, at the layer whose type constructor
                // has the extension (D10 of `generics`).
                if !self.receiver_matches(&extension, &reached) {
                    let call = self.give_up_call(receiver, args, span);
                    self.errors.push(TypeError::UnknownField {
                        target_type: reached.clone(),
                        field_name: member.to_string(),
                        span: callee.span,
                        help: Some(format!(
                            "the extension '{}' is declared on '{}', not on '{}'",
                            member, extension.receiver, reached
                        )),
                    });
                    return call;
                }
                let receiver_site = ReceiverSite {
                    expr: target,
                    ty: &receiver_ty,
                    is_place,
                    writable,
                    layers: &layers,
                    through_nullable,
                };
                let adjustment = self.pass_receiver(
                    &receiver_site,
                    extension.mode,
                    extension.builtin.is_some(),
                    member,
                );
                let (joined, receiver_argument) =
                    self.apply_receiver_adjustment(receiver, adjustment, &reached);
                let call = self.call_extension(
                    MethodCall {
                        callee,
                        target,
                        member,
                        is_safe_nav,
                        type_arguments,
                        args,
                        expected,
                    },
                    &extension,
                    &reached,
                    receiver_argument,
                    span,
                );
                if through_nullable {
                    // A safe call evaluates its arguments inside the chain (D7 of `typed-tree`).
                    self.close_chain(joined, call, SafeNavigationOrigin::Written, span)
                } else {
                    call
                }
            }
            // 5. Nothing matched.
            ExtensionLookup::None if member == "clone" => {
                self.infer_clone_call(receiver, is_safe_nav, args, span)
            }
            ExtensionLookup::None => {
                let errors_before = self.errors.len();
                let (member_node, _) =
                    self.build_member_access(receiver, member, is_safe_nav, callee.span);
                if self.errors.len() > errors_before {
                    if let Some(TypeError::UnknownField {
                        help: help @ None, ..
                    }) = self.errors.last_mut()
                    {
                        *help = Some(format!(
                            "no field or extension function '{}' is declared on '{}' in this file or exported by its imports",
                            member,
                            peel_indirection(&reached)
                        ));
                    }
                }
                let member_ty = self.ty(member_node);
                self.call_with_type(
                    callee,
                    Some(member_node),
                    member_ty,
                    false,
                    type_arguments,
                    args,
                    expected,
                    span,
                )
            }
        }
    }

    /// Types a call of the built-in `unbox()` of a box, `h.unbox()` or `h?.unbox()` on a
    /// `Heap<T>?` (D6 of `heap-boxes`): the receiver is a consuming position, so a whole local,
    /// parameter or temporary is moved and a field or top-level `val` is `E4017`, and a box
    /// behind a reference is `E4017` naming the borrow. The call moves the contents out and frees
    /// the box, and has the contents' type. It takes no arguments (`E4004`).
    fn infer_unbox_call(&mut self, call: UnboxCall, span: Span) -> ExpressionId {
        let UnboxCall {
            target,
            receiver,
            is_place,
            is_safe_nav,
            type_arguments,
            args,
        } = call;
        self.reject_type_arguments("'unbox'", type_arguments);
        if !args.is_empty() {
            self.errors.push(TypeError::CallSignatureMismatch {
                message: format!("'unbox()' takes no arguments, found {}", args.len()),
                span,
            });
            return self.give_up_call(receiver, args, span);
        }
        let receiver_ty = self.ty(receiver);
        let (boxed, nullable) = clone_target(&receiver_ty, is_safe_nav);
        let behind_reference = match &receiver_ty {
            Type::Reference { .. } => true,
            Type::Nullable(inner) if nullable => matches!(**inner, Type::Reference { .. }),
            _ => false,
        };
        if behind_reference {
            let clone = self.is_clone(&boxed).then(|| match nullable {
                true => format!("{}?.clone()?.unbox()", target),
                false => format!("{}.clone().unbox()", target),
            });
            self.errors.push(TypeError::CannotMoveOut {
                source: format!("borrow '{}'", target),
                ty: boxed,
                borrow: None,
                clone,
                destructure: None,
                unbox: None,
                remove: None,
                captured: false,
                span: target.span,
            });
            return self.error_node(vec![receiver], span);
        }
        // A temporary was consumed when it was inferred.
        if is_place {
            self.check_move_source(target, &receiver_ty);
        }
        let (joined, base) = if nullable {
            self.open_chain(receiver)
        } else {
            (None, receiver)
        };
        let moved = self.consume(base);
        let call = self.unbox(moved, span);
        if nullable {
            self.close_chain(joined, call, SafeNavigationOrigin::Written, span)
        } else {
            call
        }
    }

    /// A call of the built-in `unbox` on `moved`, a moved box or a temporary one.
    fn unbox(&mut self, moved: ExpressionId, span: Span) -> ExpressionId {
        let boxed = self.ty(moved);
        let contents = match &boxed {
            Type::Heap(contents) => contents.as_ref().clone(),
            other => other.clone(),
        };
        let call = Call {
            callee: Callee::Builtin(BuiltinFunction::Unbox),
            arguments: vec![Argument::Written(moved)],
            parameters: vec![(Some(THIS.to_string()), boxed)],
            escaping: vec![false],
            type_arguments: Vec::new(),
            evaluation_order: vec![0],
            has_receiver: true,
            has_context: false,
            is_compose: false,
        };
        self.add(ExpressionKind::Call(call), contents, span)
    }

    /// Gives up on a call whose callee cannot be resolved: its arguments are still checked, and
    /// the call is an error node keeping the receiver and them.
    fn give_up_call(
        &mut self,
        receiver: ExpressionId,
        args: &[CallArg],
        span: Span,
    ) -> ExpressionId {
        let mut children = vec![receiver];
        for arg in args {
            children.push(self.infer_expr(&arg.value));
        }
        self.error_node(children, span)
    }

    /// The local or parameter `name` that a method call `value.name(...)` on a value of type
    /// `receiver_ty` calls (D5 of `receiver-lambdas`): the innermost binding of `name`, when it
    /// has a function type with a receiver whose type is `receiver_ty` behind its `&`, `&mut`
    /// and `Heap<T>` layers. An intrinsic member found on the way wins over it.
    fn receiver_value(&self, name: &str, receiver_ty: &Type) -> Option<ReceiverValue> {
        if !self.env.resolves_to_local(name) {
            return None;
        }
        let SymbolKind::Variable { ty, .. } = &self.env.lookup(name)?.kind else {
            return None;
        };
        let Type::Fn {
            receiver: true,
            params,
            ..
        } = peel_indirection(ty)
        else {
            return None;
        };
        let Some(Type::Reference { mutable, inner }) = params.first() else {
            return None;
        };
        let mut layers = Vec::new();
        let mut reached = receiver_ty.clone();
        loop {
            if self.member(&reached, name).is_some_and(|m| !m.property) {
                return None;
            }
            if reached.is_assignable_to(inner) && inner.is_assignable_to(&reached) {
                return Some(ReceiverValue {
                    mutable: *mutable,
                    layers,
                    reached,
                });
            }
            match reached {
                Type::Reference { mutable, inner } => {
                    layers.push(Layer::Reference { mutable });
                    reached = *inner;
                }
                Type::Heap(inner) => {
                    layers.push(Layer::Heap);
                    reached = *inner;
                }
                _ => return None,
            }
        }
    }

    /// The receiver argument of a call of a function value with a receiver: `receiver`, the
    /// node of `site`, borrowed as a `fun` or `mut fun` extension's receiver is, through its
    /// layers (`references`, "Receivers of Function Values Are Borrowed Like Extension
    /// Receivers"), reporting a mutable receiver that is no mutable place (`E4003`).
    fn pass_value_receiver(
        &mut self,
        site: &ReceiverSite,
        receiver: ExpressionId,
        value: &ReceiverValue,
        name: &str,
    ) -> ExpressionId {
        let mode = if value.mutable {
            ReceiverMode::Mutable
        } else {
            ReceiverMode::Shared
        };
        self.pass_receiver(site, mode, false, name);
        let place = self.deref_until(receiver, &value.reached);
        self.borrow(value.mutable, BorrowOrigin::Receiver, place)
    }

    /// The call of the function value `callee_node` (written `callee`), of a type with a
    /// receiver, with `receiver_argument` as its receiver and `args` as its other arguments
    /// (D9 of `receiver-lambdas`): a call of the value whose first argument is the receiver.
    /// The composable context is checked unless the caller did (`check_compose`).
    #[allow(clippy::too_many_arguments)]
    fn call_receiver_value(
        &mut self,
        callee: &Expr,
        callee_node: ExpressionId,
        receiver_argument: ExpressionId,
        type_arguments: &[prism_ast::types::Type],
        args: &[CallArg],
        check_compose: bool,
        span: Span,
    ) -> ExpressionId {
        let callee_node = self.peel_indirection_nodes(callee_node);
        let Type::Fn {
            is_compose,
            params,
            escaping,
            ret,
            ..
        } = self.ty(callee_node)
        else {
            return self.error_node(vec![callee_node, receiver_argument], span);
        };
        if check_compose && is_compose && !self.is_in_compose() {
            self.report_compose_context(callee.to_string(), span);
        }
        self.reject_type_arguments(&format!("'{}'", callee), type_arguments);
        match self.check_call_arguments(&params[1..], &[], &[], args, span) {
            Err(CheckedArguments(children)) => {
                let mut all = vec![callee_node, receiver_argument];
                all.extend(children);
                self.error_node(all, span)
            }
            Ok(NormalizedArguments {
                arguments,
                evaluation_order,
            }) => {
                // The receiver is the first argument, evaluated first.
                let mut all_arguments = vec![Argument::Written(receiver_argument)];
                all_arguments.extend(arguments);
                let mut order = vec![0];
                order.extend(evaluation_order.iter().map(|index| index + 1));
                let parameters = parameter_list(&[], &params);
                self.build_call(
                    callee_node,
                    all_arguments,
                    parameters,
                    escaping,
                    Vec::new(),
                    order,
                    is_compose,
                    *ret,
                    span,
                )
            }
        }
    }

    /// A call `f(a)` of the function value `callee_node`, of a type with a receiver, written
    /// without its receiver (D5 of `receiver-lambdas`): the innermost `this`, when its type is
    /// the receiver's, is passed as a reborrow; otherwise the call is `E4004`, saying how to
    /// pass one.
    fn call_receiver_value_with_this(
        &mut self,
        callee: &Expr,
        callee_node: ExpressionId,
        type_arguments: &[prism_ast::types::Type],
        args: &[CallArg],
        span: Span,
    ) -> ExpressionId {
        let receiver_parameter = match peel_indirection(&self.ty(callee_node)) {
            Type::Fn { params, .. } => params.first().cloned(),
            _ => None,
        };
        let Some(Type::Reference {
            mutable,
            inner: receiver_ty,
        }) = receiver_parameter
        else {
            return self.error_node(vec![callee_node], span);
        };
        let this_type = match self.env.lookup(THIS).map(|sym| &sym.kind) {
            Some(SymbolKind::Variable { ty, .. }) => Some(ty.clone()),
            _ => None,
        };
        let reached = this_type.as_ref().map(|ty| peel_references(ty).clone());
        let matches = reached.as_ref().is_some_and(|reached| {
            reached.is_assignable_to(&receiver_ty) && receiver_ty.is_assignable_to(reached)
        });
        if !matches {
            self.errors.push(TypeError::MissingFunctionValueReceiver {
                callee: callee.to_string(),
                receiver: (*receiver_ty).clone(),
                mutable,
                innermost: reached.clone(),
                span,
            });
            let mut children = vec![callee_node];
            for arg in args {
                children.push(self.infer_expr(&arg.value));
            }
            return self.error_node(children, span);
        }
        let this_expr = Expr::new(ExprKind::This, callee.span);
        let (this_node, writable) = self.with_origin(Origin::Value, |c| c.infer_place(&this_expr));
        let this_ty = self.ty(this_node);
        let mut layers = Vec::new();
        let mut layer = &this_ty;
        while let Type::Reference { mutable, inner } = layer {
            layers.push(Layer::Reference { mutable: *mutable });
            layer = inner;
        }
        let value = ReceiverValue {
            mutable,
            layers,
            reached: reached.expect("matched above"),
        };
        let site = ReceiverSite {
            expr: &this_expr,
            ty: &this_ty,
            is_place: true,
            writable,
            layers: &value.layers,
            through_nullable: false,
        };
        let mode = if mutable {
            ReceiverMode::Mutable
        } else {
            ReceiverMode::Shared
        };
        self.pass_receiver(&site, mode, false, &callee.to_string());
        let place = self.deref_until(this_node, &value.reached);
        let argument = self.borrow(mutable, BorrowOrigin::Argument, place);
        // The composable context was checked with the callee.
        self.call_receiver_value(
            callee,
            callee_node,
            argument,
            type_arguments,
            args,
            false,
            span,
        )
    }

    /// The first argument of an extension call: the receiver `receiver` passed as `adjustment`
    /// decides (D5 of `typed-tree`). Returns the written chain the call joins, for a safe call
    /// on one, and the argument.
    fn apply_receiver_adjustment(
        &mut self,
        receiver: ExpressionId,
        adjustment: ReceiverAdjustment,
        reached: &Type,
    ) -> (Option<ExpressionId>, ExpressionId) {
        let (joined, base) = if adjustment.through_nullable {
            self.open_chain(receiver)
        } else {
            (None, receiver)
        };
        if adjustment.pass == ReceiverPass::Unbox {
            let span = self.builder.span_of(base);
            let mut argument = self.consume(base);
            for _ in 0..adjustment.dereferences {
                argument = self.unbox(argument, span);
            }
            return (joined, argument);
        }
        let place = self.deref_until(base, reached);
        let argument = match adjustment.pass {
            ReceiverPass::SharedBorrow => self.borrow(false, BorrowOrigin::Receiver, place),
            ReceiverPass::MutableBorrow => self.borrow(true, BorrowOrigin::Receiver, place),
            // A whole receiver is moved (or copied); a temporary is the consumed value itself.
            ReceiverPass::Move => self.consume(place),
            ReceiverPass::CopyOut => self.copy(place),
            ReceiverPass::Unbox => unreachable!("unboxed above"),
        };
        (joined, argument)
    }

    /// A copy of the place `id`.
    fn copy(&mut self, id: ExpressionId) -> ExpressionId {
        let place = self.as_place(id);
        let (ty, span) = (self.ty(place), self.builder.span_of(place));
        self.add(ExpressionKind::Copy { place }, ty, span)
    }

    /// Whether `reached`, the type an extension call reaches, matches the extension's declared
    /// receiver: equals it, for a concrete receiver, or instantiates it, for a generic one.
    fn receiver_matches(&self, extension: &Extension, reached: &Type) -> bool {
        let type_parameters = extension.symbol.type_parameters();
        let declared = if type_parameters.is_empty() {
            extension.receiver.clone()
        } else {
            let mut unknowns = Unknowns::new(type_parameters.len());
            unknowns.match_exact(&extension.receiver, reached);
            extension.receiver.substitute(&unknowns.partial())
        };
        reached.is_assignable_to(&declared) && declared.is_assignable_to(reached)
    }

    /// Checks the arguments of a call of `extension` against its parameters, and its compose
    /// context (`E4008`), building the call with `receiver_argument` as its first argument. A
    /// generic extension's type arguments are inferred from the receiver first (D8 of
    /// `generics`).
    fn call_extension(
        &mut self,
        call: MethodCall,
        extension: &Extension,
        reached: &Type,
        receiver_argument: ExpressionId,
        span: Span,
    ) -> ExpressionId {
        let MethodCall {
            callee,
            member,
            type_arguments,
            args,
            expected,
            ..
        } = call;
        let SymbolKind::Function {
            fn_type:
                Type::Fn {
                    is_compose,
                    params,
                    escaping,
                    ret,
                    ..
                },
            param_names,
            has_default,
            type_parameters,
            ..
        } = &extension.symbol.kind
        else {
            return self.error_node(vec![receiver_argument], span);
        };
        if *is_compose && !self.is_in_compose() {
            self.report_compose_context(callee.to_string(), span);
        }
        // An extension with a context parameter takes its context argument first (D7 of
        // `receiver-lambdas`).
        let context = match &extension.context {
            Some(context_ty) => match self.context_argument(member, context_ty, callee.span) {
                Some(argument) => Some((argument, context_ty.clone())),
                None => {
                    let mut children = vec![receiver_argument];
                    for arg in args {
                        children.push(self.infer_expr(&arg.value));
                    }
                    return self.error_node(children, span);
                }
            },
            None => None,
        };
        // The receiver is never `escaping`.
        let escaping: Vec<bool> = std::iter::once(false)
            .chain(escaping.iter().copied())
            .collect();
        let (is_compose, params, param_names, has_default, ret) = (
            *is_compose,
            params.clone(),
            param_names.clone(),
            has_default.clone(),
            (**ret).clone(),
        );
        // The declared type of `this`, which the call records like every declared parameter type.
        let declared_receiver = extension.receiver.clone();
        let this_ty = match extension.mode {
            ReceiverMode::Shared => Type::Reference {
                mutable: false,
                inner: Box::new(declared_receiver),
            },
            ReceiverMode::Mutable => Type::Reference {
                mutable: true,
                inner: Box::new(declared_receiver),
            },
            ReceiverMode::Consuming => declared_receiver,
        };
        let checked = if type_parameters.is_empty() {
            self.reject_type_arguments(&format!("extension '{}'", member), type_arguments);
            self.check_call_arguments(&params, &param_names, &has_default, args, span)
                .map(|normalized| (normalized, params, ret, Vec::new()))
        } else {
            let signature = GenericSignature {
                owner: format!("extension '{}.{}'", extension.receiver, member),
                type_parameters: type_parameters.clone(),
                params,
                ret,
                receiver: Some((extension.receiver.clone(), reached.clone())),
            };
            self.check_generic_call_arguments(
                &signature,
                type_arguments,
                expected,
                &param_names,
                &has_default,
                args,
                span,
            )
            .map(|(normalized, solution)| {
                let ret = signature.ret.substitute(&solution);
                (normalized, signature.params, ret, solution)
            })
        };
        match checked {
            Err(CheckedArguments(children)) => {
                let mut all = vec![receiver_argument];
                all.extend(children);
                self.error_node(all, span)
            }
            Ok((
                NormalizedArguments {
                    arguments,
                    evaluation_order,
                },
                params,
                ret,
                type_arguments,
            )) => {
                // The receiver is the first argument, evaluated first (D8 of `typed-tree`).
                let mut all_arguments = vec![Argument::Written(receiver_argument)];
                all_arguments.extend(arguments);
                let mut order = vec![0];
                order.extend(evaluation_order.iter().map(|index| index + 1));
                let mut parameters = vec![(Some(THIS.to_string()), this_ty)];
                parameters.extend(parameter_list(&param_names, &params));
                // Searching compares elements with `==`, which a type parameter lacks (D12 of
                // `collections`).
                if let (
                    Some(BuiltinFunction::ListContains | BuiltinFunction::ListIndexOf),
                    Some(element),
                ) = (extension.builtin, type_arguments.first())
                {
                    if element.mentions_parameter_outside_functions() {
                        self.errors.push(TypeError::OperatorMismatch {
                            operator: "==".to_string(),
                            left: element.clone(),
                            right: element.clone(),
                            span,
                            help: Some(format!(
                                "'{}' compares the elements with '==', and type parameters have no equality, with or without a bound, until traits exist",
                                member
                            )),
                        });
                    }
                }
                let callee = match extension.builtin {
                    Some(builtin) => {
                        self.reject_composition_in_composable(builtin, span);
                        Callee::Builtin(builtin)
                    }
                    None => Callee::Function(extension.path(member)),
                };
                let has_context = context.is_some();
                let mut escaping = escaping;
                if let Some(context) = context {
                    Self::prepend_context(
                        context,
                        &mut all_arguments,
                        &mut parameters,
                        &mut escaping,
                        &mut order,
                    );
                }
                let call = Call {
                    callee,
                    arguments: all_arguments,
                    parameters,
                    escaping,
                    type_arguments,
                    evaluation_order: order,
                    has_receiver: true,
                    has_context,
                    is_compose,
                };
                self.add(ExpressionKind::Call(call), ret, span)
            }
        }
    }

    /// The extension named `name` on receiver type `receiver` visible in this file: its own one,
    /// which shadows imported ones, else the single import exporting one, else the prelude's.
    /// Within the first of these layers that declares an extension of the key, the one whose
    /// context is a candidate of the current context is chosen, else the one without a context
    /// (D8 of `receiver-lambdas`).
    fn find_extension(&self, receiver: &Type, name: &str) -> ExtensionLookup {
        let key = (ReceiverKey::of(receiver), name.to_string());
        if let Some(extensions) = self.own_extensions.get(&key) {
            return self.choose_by_context(extensions, true);
        }
        // A prelude extension is found last, so the file's own and imported ones shadow it
        // without `E4012` (D4 of `standard-library`).
        if let Some(extensions) = self.imported_extensions.get(&key) {
            return self.choose_by_context(extensions, false);
        }
        match self.prelude.as_ref().and_then(|p| p.extensions(&key)) {
            Some(extensions) => self.choose_by_context(extensions, false),
            None => ExtensionLookup::None,
        }
    }

    /// The context of the current position, for `E4034`: what its candidates are, or that it
    /// has none.
    fn describe_context(&self) -> String {
        let Some(first) = self.context_candidates.first() else {
            return "this position has no receiver".to_string();
        };
        if first.source == ContextSource::LambdaReceiver {
            return format!(
                "the innermost receiver here is '{}', the receiver of this lambda",
                first.referent
            );
        }
        let described: Vec<String> = self
            .context_candidates
            .iter()
            .map(|candidate| match &candidate.source {
                ContextSource::ContextParameter(name) => format!(
                    "the context parameter '{}' has type '{}'",
                    name, candidate.referent
                ),
                _ => format!(
                    "the receiver of the enclosing extension is '{}'",
                    candidate.referent
                ),
            })
            .collect();
        described.join(" and ")
    }

    /// The context argument of a call of `function`, whose context parameter has type
    /// `context`: a shared borrow of the referent of the first context candidate of that type
    /// (D7 of `receiver-lambdas`), or `None` after reporting `E4034`.
    fn context_argument(
        &mut self,
        function: &str,
        context: &Type,
        span: Span,
    ) -> Option<ExpressionId> {
        let candidate = self
            .context_candidates
            .iter()
            .find(|candidate| &candidate.referent == context)
            .cloned();
        let Some(candidate) = candidate else {
            let found = self.describe_context();
            self.errors.push(TypeError::MissingContext {
                function: function.to_string(),
                needed: vec![context.clone()],
                found,
                span,
            });
            return None;
        };
        self.builder.use_local(candidate.local, span);
        let node = self.add(ExpressionKind::Local(candidate.local), candidate.ty, span);
        let place = self.deref_until(node, &candidate.referent);
        Some(self.borrow(false, BorrowOrigin::Argument, place))
    }

    /// The context candidate of the binding `symbol`, a receiver or context parameter.
    fn context_candidate(symbol: &Symbol, source: ContextSource) -> Option<ContextCandidate> {
        let SymbolKind::Variable {
            ty,
            local: Some(local),
            ..
        } = &symbol.kind
        else {
            return None;
        };
        Some(ContextCandidate {
            local: *local,
            ty: ty.clone(),
            referent: peel_references(ty).clone(),
            source,
        })
    }

    /// The extension of one layer's `extensions` that a call in the current context resolves
    /// to: the one whose context type is the referent of the first context candidate that has
    /// one, else the one without a context, else none, naming the layer's contexts. A context
    /// several modules declare is an ambiguity (`E4012`).
    fn choose_by_context(&self, extensions: &ExtensionsOfName, own: bool) -> ExtensionLookup {
        let chosen = self
            .context_candidates
            .iter()
            .map(|candidate| extensions.of_context(&Some(candidate.referent.clone())))
            .find(|declared| !declared.is_empty())
            .unwrap_or_else(|| extensions.of_context(&None));
        match chosen {
            [] => ExtensionLookup::MissingContext(extensions.contexts()),
            [(_, extension)] => ExtensionLookup::Found {
                extension: Box::new(extension.clone()),
                own,
            },
            many => ExtensionLookup::Ambiguous(many.iter().map(|(m, _)| m.clone()).collect()),
        }
    }

    /// The extension `member` of `reached` or of what it reaches through its `&`, `&mut` and
    /// `Heap<T>` layers, the outermost first, with the type it was found on; each layer looked
    /// through is pushed onto `layers`.
    fn find_extension_through_indirection(
        &self,
        mut reached: Type,
        member: &str,
        layers: &mut Vec<Layer>,
    ) -> (ExtensionLookup, Type) {
        loop {
            // An intrinsic member takes precedence over every extension (D7 of
            // `standard-library`): the list members.
            if let Some(extension) = self.member(&reached, member).filter(|m| !m.property) {
                let found = ExtensionLookup::Found {
                    extension: Box::new(extension.clone()),
                    own: false,
                };
                return (found, reached);
            }
            match self.find_extension(&reached, member) {
                ExtensionLookup::None => {}
                found => return (found, reached),
            }
            match reached {
                Type::Reference { mutable, inner } => {
                    layers.push(Layer::Reference { mutable });
                    reached = *inner;
                }
                Type::Heap(inner) => {
                    layers.push(Layer::Heap);
                    reached = *inner;
                }
                _ => return (ExtensionLookup::None, reached),
            }
        }
    }

    /// Passes the receiver of a call of `extension` by its mode (D8 and D9 of
    /// `extension-methods`), reporting a `mut fun` receiver that is not a mutable place (`E4003`)
    /// and a `consuming fun` receiver that cannot be moved (`E4017`).
    fn pass_receiver(
        &mut self,
        receiver: &ReceiverSite,
        mode: ReceiverMode,
        builtin: bool,
        method: &str,
    ) -> ReceiverAdjustment {
        let expr = receiver.expr;
        let pass = match mode {
            ReceiverMode::Shared => ReceiverPass::SharedBorrow,
            ReceiverMode::Mutable => {
                // A temporary may be mutated unless it is reached through a shared borrow.
                if !self.reject_guard_mutation(expr, expr.span)
                    && !contents_writable(receiver.ty, receiver.writable)
                {
                    self.report_immutable_receiver(receiver, method);
                    // D3 of `collections`: there is no separate mutable list type.
                    if builtin {
                        if let Some(TypeError::ImmutabilityViolation { help, .. }) =
                            self.errors.last_mut()
                        {
                            let fix = help
                                .take()
                                .map_or(String::new(), |fix| format!(": {}", fix));
                            *help = Some(format!(
                                "a list is changed through a 'var' or a '&mut'{}",
                                fix
                            ));
                        }
                    }
                }
                ReceiverPass::MutableBorrow
            }
            ReceiverMode::Consuming if receiver.layers.is_empty() => {
                // A temporary was consumed when it was inferred.
                if receiver.is_place {
                    self.check_move_source(expr, receiver.ty);
                }
                ReceiverPass::Move
            }
            ReceiverMode::Consuming => {
                let reached = peel_indirection(match peel_references(receiver.ty) {
                    Type::Nullable(inner) if receiver.through_nullable => inner,
                    other => other,
                })
                .clone();
                let through_boxes_only = receiver.layers.iter().all(|layer| layer == &Layer::Heap);
                if self.is_copy(&reached) {
                    ReceiverPass::CopyOut
                } else if through_boxes_only {
                    // An owned receiver behind boxes is moved and unboxed, once per box (D7 of
                    // `heap-boxes`).
                    if receiver.is_place {
                        self.check_move_source(expr, receiver.ty);
                    }
                    ReceiverPass::Unbox
                } else {
                    let clone = self.is_clone(&reached).then(|| {
                        if receiver.through_nullable {
                            format!("{}?.clone()", expr)
                        } else {
                            format!("{}.clone()", expr)
                        }
                    });
                    self.errors.push(TypeError::CannotMoveOut {
                        source: format!("borrow '{}'", expr),
                        ty: reached,
                        borrow: None,
                        clone,
                        destructure: None,
                        unbox: None,
                        remove: None,
                        captured: false,
                        span: expr.span,
                    });
                    ReceiverPass::CopyOut
                }
            }
        };
        ReceiverAdjustment {
            dereferences: receiver.layers.len(),
            through_nullable: receiver.through_nullable,
            pass,
            temporary: !receiver.is_place,
        }
    }

    /// Reports `E4003` for the receiver of a call of `mut fun` `method` that is not a mutable
    /// place.
    fn report_immutable_receiver(&mut self, receiver: &ReceiverSite, method: &str) {
        let expr = receiver.expr;
        let shared_borrow = receiver
            .layers
            .iter()
            .any(|layer| layer == &Layer::Reference { mutable: false });
        let (message, help) = match (&expr.kind, binding_name(expr)) {
            (ExprKind::This, _) if shared_borrow => (
                format!(
                    "cannot call 'mut fun' '{}' on 'this': in a 'fun' extension, 'this' is a shared '&' borrow",
                    method
                ),
                Some("declare the enclosing extension 'mut fun' to mutate its receiver".to_string()),
            ),
            (ExprKind::This, _) => (
                format!(
                    "cannot call 'mut fun' '{}' on 'this': in a 'consuming fun', 'this' is an immutable 'val'",
                    method
                ),
                Some("move it into a mutable binding first: 'var value = this'".to_string()),
            ),
            (_, Some(name)) if shared_borrow => (
                format!(
                    "cannot call 'mut fun' '{}' on '{}': it is a shared '&' borrow",
                    method, name
                ),
                Some("take a mutable borrow '&mut' where the reference is created".to_string()),
            ),
            (_, Some(name)) => (
                format!(
                    "cannot call 'mut fun' '{}' on '{}': it is an immutable 'val' binding",
                    method, name
                ),
                Some(format!("declare it with 'var {}' to call a 'mut fun' on it", name)),
            ),
            _ => (
                format!(
                    "cannot call 'mut fun' '{}' on '{}': it is a 'val' field or is reached through a 'val' binding, 'val' field or shared '&' borrow",
                    method, expr
                ),
                None,
            ),
        };
        self.errors.push(TypeError::ImmutabilityViolation {
            message,
            span: expr.span,
            help,
        });
    }

    /// Whether `expr` is an optional place: a chain of `?.` (or `.`) field accesses which has at
    /// least one `?.` (`t?.lead` in `t?.lead?.greet()`), from any root, as a `.` field path is a
    /// place from any root: a root that is not a place is evaluated into a temporary first
    /// (`u()?.name`). As the receiver of a call it is borrowed where it lives when every link is
    /// present (D10 of `extension-methods`).
    fn is_optional_place(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::MemberAccess {
                target,
                is_safe_nav,
                ..
            } => {
                self.type_name_symbol(target).is_none()
                    && (*is_safe_nav || self.is_optional_place(target))
            }
            _ => false,
        }
    }

    /// In an extension body, the member access `this.name` that a bare name `name` stands for:
    /// a name that is no local or parameter, and that names a field of the receiver's struct (D5
    /// of `extension-methods`).
    fn implicit_this_field(&self, expr: &Expr) -> Option<Expr> {
        let ExprKind::Ident(name) = &expr.kind else {
            return None;
        };
        if self.env.resolves_to_local(name) {
            return None;
        }
        let receiver = self.current_receiver()?;
        self.has_field(&receiver, name)
            .then(|| this_member(name, expr.span))
    }

    /// In an extension body, the member access `this.name` that the callee `name` of a call
    /// stands for: a name that is no local or parameter, and that names a field of the
    /// receiver's struct or an extension on the receiver's type (D5 of `extension-methods`).
    fn implicit_this_callee(&self, callee: &Expr) -> Option<Expr> {
        if let Some(field) = self.implicit_this_field(callee) {
            return Some(field);
        }
        let ExprKind::Ident(name) = &callee.kind else {
            return None;
        };
        if self.env.resolves_to_local(name) {
            return None;
        }
        let receiver = self.current_receiver()?;
        // An intrinsic member wins over declared extensions (D7 of `standard-library`), so a
        // bare call of one in an extension on its receiver type is a call on `this` too.
        if self.has_intrinsic_function(&receiver, name) {
            return Some(this_member(name, callee.span));
        }
        match self.find_extension(&receiver, name) {
            ExtensionLookup::None => None,
            _ => Some(this_member(name, callee.span)),
        }
    }

    /// The receiver type `T` of the extension whose body is being checked, where `this` is a
    /// `&T`, `&mut T` or `T`.
    fn current_receiver(&self) -> Option<Type> {
        match &self.env.lookup(THIS)?.kind {
            SymbolKind::Variable { ty, .. } => Some(peel_references(ty).clone()),
            _ => None,
        }
    }

    /// Checks the arguments of a call against the parameters of a function, positional, named
    /// or omitted for a parameter with a default. `param_names` and `has_default` are empty when
    /// the callee is not a declared function (a function-typed value), which allows neither.
    ///
    /// Returns the arguments in parameter order with a default slot for each omitted one, and
    /// the parameter indices of the written arguments in source order (D8 of `typed-tree`); or,
    /// when the arguments do not match the parameters (`E4004`), the nodes of those checked.
    fn check_call_arguments(
        &mut self,
        params: &[Type],
        param_names: &[String],
        has_default: &[bool],
        args: &[CallArg],
        span: Span,
    ) -> Result<NormalizedArguments, CheckedArguments> {
        self.check_call_arguments_with(
            params.len(),
            param_names,
            has_default,
            args,
            span,
            &mut |checker, index, value| {
                Some(checker.check_argument(value, &params[index], Origin::Argument))
            },
        )
    }

    /// [`Self::check_call_arguments`] for a callee with `n_params` parameters, where `check`
    /// checks the argument for parameter `index`, in source order. `check` may defer an
    /// argument by returning `None`, leaving its slot `Argument::Default` for the caller to fill.
    fn check_call_arguments_with<'a>(
        &mut self,
        n_params: usize,
        param_names: &[String],
        has_default: &[bool],
        args: &'a [CallArg],
        span: Span,
        check: &mut ArgumentChecker<'a, '_>,
    ) -> Result<NormalizedArguments, CheckedArguments> {
        let min_args = if has_default.len() == n_params {
            has_default.iter().filter(|&&d| !d).count()
        } else {
            n_params
        };

        let has_named_args = args.iter().any(|a| a.name.is_some());
        let mut arguments = vec![Argument::Default; n_params];
        let mut evaluation_order = Vec::new();
        let mut checked = Vec::new();
        // A trailing lambda binds to the last parameter, so positional binding gives the same
        // parameters only when it is the argument for that parameter (`f(1) { }` of `f(a, b)`).
        let trailing_lambda_in_order = args
            .last()
            .is_none_or(|last| !last.is_trailing_lambda || args.len() == n_params);

        if !has_named_args && trailing_lambda_in_order {
            if args.len() < min_args || args.len() > n_params {
                let msg = if min_args == n_params {
                    format!(
                        "function expected {} arguments, found {}",
                        n_params,
                        args.len()
                    )
                } else {
                    format!(
                        "function expected between {} and {} arguments, found {}",
                        min_args,
                        n_params,
                        args.len()
                    )
                };
                self.errors
                    .push(TypeError::CallSignatureMismatch { message: msg, span });
                return Err(CheckedArguments(Vec::new()));
            }

            for (index, arg) in args.iter().enumerate() {
                if let Some(value) = check(self, index, &arg.value) {
                    arguments[index] = Argument::Written(value);
                }
                evaluation_order.push(index);
            }
            return Ok(NormalizedArguments {
                arguments,
                evaluation_order,
            });
        }

        let mut failed = false;
        let mut param_assigned = vec![false; n_params];
        let mut seen_names = HashSet::new();
        let mut positional_idx = 0;

        for arg in args {
            if arg.is_trailing_lambda {
                let Some(last) = n_params.checked_sub(1) else {
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message:
                            "function takes no parameters, so it cannot take a trailing lambda"
                                .to_string(),
                        span: arg.span,
                    });
                    failed = true;
                    continue;
                };
                if param_assigned[last] {
                    let given = match param_names.get(last) {
                        Some(name) if seen_names.contains(name) => "by name",
                        _ => "a positional argument",
                    };
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message: format!(
                            "the trailing lambda is passed to {}, the last parameter, which is already given {}",
                            describe_parameter(param_names, last),
                            given
                        ),
                        span: arg.span,
                    });
                    failed = true;
                    continue;
                }
                param_assigned[last] = true;
                if let Some(value) = check(self, last, &arg.value) {
                    arguments[last] = Argument::Written(value);
                    checked.push(value);
                }
                evaluation_order.push(last);
            } else if let Some(arg_name) = &arg.name {
                if !seen_names.insert(arg_name.clone()) {
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message: format!("duplicate argument '{}' provided", arg_name),
                        span: arg.span,
                    });
                    failed = true;
                    continue;
                }
                if let Some(pos) = param_names.iter().position(|pn| pn == arg_name) {
                    if param_assigned[pos] {
                        self.errors.push(TypeError::CallSignatureMismatch {
                            message: format!(
                                "parameter '{}' is already given a positional argument",
                                arg_name
                            ),
                            span: arg.span,
                        });
                        failed = true;
                    }
                    param_assigned[pos] = true;
                    if let Some(value) = check(self, pos, &arg.value) {
                        arguments[pos] = Argument::Written(value);
                        checked.push(value);
                    }
                    evaluation_order.push(pos);
                } else {
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message: format!("unknown parameter '{}'", arg_name),
                        span: arg.span,
                    });
                    failed = true;
                }
            } else if positional_idx < n_params {
                if param_assigned[positional_idx] {
                    self.errors.push(TypeError::CallSignatureMismatch {
                        message: format!(
                            "positional argument assigns parameter '{}', which is already given by name",
                            param_names
                                .get(positional_idx)
                                .map(String::as_str)
                                .unwrap_or("parameter")
                        ),
                        span: arg.span,
                    });
                    failed = true;
                }
                param_assigned[positional_idx] = true;
                if let Some(value) = check(self, positional_idx, &arg.value) {
                    arguments[positional_idx] = Argument::Written(value);
                    checked.push(value);
                }
                evaluation_order.push(positional_idx);
                positional_idx += 1;
            } else {
                self.errors.push(TypeError::CallSignatureMismatch {
                    message: format!(
                        "too many positional arguments (expected at most {})",
                        n_params
                    ),
                    span: arg.span,
                });
                failed = true;
            }
        }

        for (i, &assigned) in param_assigned.iter().enumerate().take(n_params) {
            let has_def = has_default.get(i).copied().unwrap_or(false);
            if !has_def && !assigned {
                let p_name = param_names
                    .get(i)
                    .map(|s| s.as_str())
                    .unwrap_or("parameter");
                self.errors.push(TypeError::CallSignatureMismatch {
                    message: format!("missing required argument '{}'", p_name),
                    span,
                });
                failed = true;
            }
        }
        if failed {
            return Err(CheckedArguments(checked));
        }
        Ok(NormalizedArguments {
            arguments,
            evaluation_order,
        })
    }

    /// Decides how a value of type `found` fits `expected` at `site`, in the order of D10 of
    /// `reference-access`: as it is, by an implicit borrow or reborrow (arguments only), by the
    /// nullable view, or by copying a `Copy` value out of borrows. Reports nothing.
    fn coerce(&self, found: &Type, expected: &Type, site: Site) -> Coercion {
        let is_place = matches!(site, Site::Argument { is_place: true });
        if found.is_assignable_to(expected) {
            return Coercion::Fits(
                if is_place && matches!(found, Type::Reference { mutable: true, .. }) {
                    Adjustment::Reborrow
                } else {
                    Adjustment::None
                },
            );
        }
        if let Site::Argument { is_place } = site {
            if let Some(adjustment) = implicit_borrow(found, expected, is_place) {
                return Coercion::Fits(adjustment);
            }
        }
        if let Some(adjustment) = box_contents(found, expected, site) {
            return Coercion::Fits(adjustment);
        }
        if nullable_view(found).is_some_and(|view| view.is_assignable_to(expected)) {
            return Coercion::Fits(Adjustment::NullableView);
        }
        self.copy_out(found, expected)
    }

    /// Copies a `Copy` value out of the references and boxes around `found` until it fits
    /// `expected`, directly or, for references, inside a `?` (`(&T)?` as a `T?`).
    fn copy_out(&self, found: &Type, expected: &Type) -> Coercion {
        let (found, expected, through_nullable) = match (found, expected) {
            (Type::Nullable(found), Type::Nullable(expected))
                if matches!(**found, Type::Reference { .. }) =>
            {
                (found.as_ref(), expected.as_ref(), true)
            }
            _ => (found, expected, false),
        };
        let mut referent = found;
        let mut layers = 0;
        let mut through_box = true;
        loop {
            referent = match referent {
                Type::Reference { inner, .. } => {
                    through_box = false;
                    inner
                }
                Type::Heap(inner) if !through_nullable => inner,
                _ => return Coercion::NoFit,
            };
            layers += 1;
            if !referent.is_assignable_to(expected) {
                continue;
            }
            return if self.is_copy(referent) {
                Coercion::Fits(Adjustment::CopyOut {
                    layers,
                    through_nullable,
                })
            } else {
                Coercion::MovesOutOfBorrow {
                    ty: referent.clone(),
                    nullable: through_nullable,
                    through_box,
                }
            };
        }
    }

    /// Checks `expr`, already inferred as the node `found_id`, against `expected` at `site`,
    /// reporting what does not fit, and returns the node that fits: wrapped in the adjustment,
    /// move, copy or coercion that makes it fit (D5 and D6 of `typed-tree`), or in an error
    /// node. `deferred_move` is set when `expr` is a place whose move check was held back until
    /// the coercion is known: a borrow or copy-out moves nothing, and a value that does not fit
    /// reports only the mismatch.
    fn fit(
        &mut self,
        expr: &Expr,
        found_id: ExpressionId,
        expected: &Type,
        site: Site,
        deferred_move: bool,
    ) -> ExpressionId {
        let found = self.ty(found_id);
        let coercion = self.coerce(&found, expected, site);
        // A function value keeps its representation when converted (D8 of `closures`). Only a
        // value used as it is, or a temporary converted before it is lent, converts: a place
        // is lent unchanged, since references are invariant.
        let converted_to = match coercion {
            Coercion::Fits(Adjustment::None) => Some(expected.clone()),
            Coercion::Fits(Adjustment::BorrowTemporary) => {
                Some(temporary_referent(&found, expected))
            }
            _ => None,
        };
        if let Some(to) =
            converted_to.filter(|to| !self.converts_function_value(found_id, &found, to))
        {
            // A named function's own conversions are its adapter's, so a closure calling it
            // would not help.
            let help = (!matches!(
                self.builder.kind(found_id),
                ExpressionKind::FunctionReference(_) | ExpressionKind::VariantConstructor { .. }
            ))
            .then(|| closure_call_help(expr, &to));
            self.report_mismatch_with_help(expected, found, expr.span, help);
            return self.wrap_error(found_id);
        }
        match coercion {
            Coercion::Fits(Adjustment::None) => {
                let id = if deferred_move {
                    self.check_move_source(expr, &found);
                    self.consume(found_id)
                } else if matches!(site, Site::Argument { is_place: true }) && self.is_copy(&found)
                {
                    // A shared borrow passed on is copied.
                    self.consume(found_id)
                } else {
                    found_id
                };
                self.coerce_to(id, expected)
            }
            // A mutable reborrow of a guard's binding would mutate it (D9 of `payload-patterns`).
            Coercion::Fits(Adjustment::Reborrow)
                if matches!(expected, Type::Reference { mutable: true, .. })
                    && self.reject_guard_mutation(expr, expr.span) =>
            {
                self.wrap_error(found_id)
            }
            Coercion::Fits(adjustment) => self.apply_adjustment(found_id, adjustment, expected),
            // The contents of a box move out only through `unbox()` (D6 of `heap-boxes`).
            Coercion::MovesOutOfBorrow {
                ty,
                through_box: true,
                ..
            } => {
                self.errors.push(TypeError::CannotMoveOut {
                    source: format!("the contents of 'Heap' '{}'", expr),
                    ty,
                    borrow: None,
                    clone: None,
                    destructure: None,
                    unbox: Some(format!("{}.unbox()", expr)),
                    remove: None,
                    captured: false,
                    span: expr.span,
                });
                self.wrap_error(found_id)
            }
            Coercion::MovesOutOfBorrow { ty, nullable, .. } => {
                let clone = self.is_clone(&ty).then(|| match nullable {
                    true => format!("{}?.clone()", expr),
                    false => format!("{}.clone()", expr),
                });
                self.errors.push(TypeError::CannotMoveOut {
                    source: format!("borrow '{}'", expr),
                    ty: if nullable { ty.nullable() } else { ty },
                    borrow: None,
                    clone,
                    destructure: None,
                    unbox: None,
                    remove: None,
                    captured: false,
                    span: expr.span,
                });
                self.wrap_error(found_id)
            }
            Coercion::NoFit => {
                if found == Type::Error {
                    return found_id;
                }
                let help = borrow_help(expr, &found, expected, site)
                    .or_else(|| invariant_box_help(&found, expected));
                self.report_mismatch_with_help(expected, found, expr.span, help);
                self.wrap_error(found_id)
            }
        }
    }

    /// Builds the nodes of `adjustment`, which makes the node `found_id` fit `expected` (the
    /// table of D5 of `typed-tree`): copies and borrows of dereferences, temporaries and
    /// payloads, in a chain for the nullable conversions.
    fn apply_adjustment(
        &mut self,
        found_id: ExpressionId,
        adjustment: Adjustment,
        expected: &Type,
    ) -> ExpressionId {
        let span = self.builder.span_of(found_id);
        let adjusted = match adjustment {
            Adjustment::None => found_id,
            Adjustment::CopyOut {
                layers,
                through_nullable,
            } => {
                let mut place = if through_nullable {
                    self.non_null(found_id)
                } else {
                    found_id
                };
                for _ in 0..layers {
                    // A box that is not a place is held in a temporary, which drops it at the
                    // end of the statement.
                    if matches!(self.ty(place), Type::Heap(_)) {
                        place = self.as_place(place);
                    }
                    place = self.deref(place);
                }
                let copy = self.copy(place);
                if through_nullable {
                    self.close_chain(None, copy, SafeNavigationOrigin::CopyOut, span)
                } else {
                    copy
                }
            }
            Adjustment::Borrow => self.borrow(false, BorrowOrigin::Argument, found_id),
            Adjustment::BorrowTemporary => match self.ty(found_id) {
                // A temporary is converted to the referent first, then borrowed: the borrow
                // must point at a value of the referent's representation.
                found
                    if !(is_null_type(&found) && matches!(expected, Type::Nullable(_)))
                        && found != temporary_referent(&found, expected) =>
                {
                    let referent = temporary_referent(&found, expected);
                    let converted = self.coerce_to(found_id, &referent);
                    return self.apply_adjustment(converted, Adjustment::BorrowTemporary, expected);
                }
                // A temporary lent as an optional borrow `(&T)?` lends its contents; `null`
                // lends nothing, so it is an empty optional borrow.
                Type::Nullable(inner)
                    if *inner == Type::Nothing && matches!(expected, Type::Nullable(_)) =>
                {
                    found_id
                }
                Type::Nullable(_) if matches!(expected, Type::Nullable(_)) => {
                    let temporary = self.as_place(found_id);
                    let payload = self.non_null(temporary);
                    let borrow = self.borrow(false, BorrowOrigin::Argument, payload);
                    self.close_chain(None, borrow, SafeNavigationOrigin::BorrowContents, span)
                }
                _ => self.borrow(false, BorrowOrigin::Argument, found_id),
            },
            Adjustment::BorrowContents => {
                let payload = self.non_null(found_id);
                let borrow = self.borrow(false, BorrowOrigin::Argument, payload);
                self.close_chain(None, borrow, SafeNavigationOrigin::BorrowContents, span)
            }
            Adjustment::Reborrow => {
                let mutable = matches!(
                    match expected {
                        Type::Nullable(inner) => inner.as_ref(),
                        other => other,
                    },
                    Type::Reference { mutable: true, .. }
                );
                let referent = self.deref(found_id);
                self.borrow(mutable, BorrowOrigin::Argument, referent)
            }
            Adjustment::NullableView => self.nullable_view_chain(found_id),
            Adjustment::BoxContents {
                steps,
                through_nullable,
                mutable,
                origin,
            } => {
                // An owned box is lent where it lives, or from a temporary that drops it at the
                // end of the statement; a borrowed one is reborrowed at its contents.
                let owned = !matches!(self.ty(found_id), Type::Reference { .. });
                let mut place = if owned {
                    self.as_place(found_id)
                } else {
                    found_id
                };
                for _ in 0..steps {
                    place = match self.ty(place) {
                        Type::Nullable(inner) => {
                            let inner = inner.as_ref().clone();
                            let kind = ExpressionKind::NonNullPayload { nullable: place };
                            self.add(kind, inner, span)
                        }
                        _ => self.deref(place),
                    };
                }
                let borrow = self.borrow(mutable, origin, place);
                if through_nullable {
                    let origin = if owned {
                        SafeNavigationOrigin::BorrowContents
                    } else {
                        SafeNavigationOrigin::NullableView
                    };
                    self.close_chain(None, borrow, origin, span)
                } else {
                    borrow
                }
            }
        };
        self.coerce_to(adjusted, expected)
    }

    /// Rejects moving `expr`, of type `ty`, out of where it lives (the `ownership` spec, "Moves
    /// Take Whole Values Only"). Only leaves are checked: a non-`Copy` value may be consumed from
    /// a whole local or parameter, or as a temporary, but not from a field, a safe navigation, an
    /// index, a top-level `val`, a binding a default value borrows or a binding the innermost
    /// closure captures. Pass-through forms (`if`, `when`, blocks, `?:`) mark their
    /// children as consumed instead.
    fn check_move_source(&mut self, expr: &Expr, ty: &Type) {
        if self.is_copy(ty) {
            return;
        }
        if let Some(field) = self.implicit_this_field(expr) {
            return self.check_move_source(&field, ty);
        }
        // A binding of the guard being checked may not move: the guard may be false (D9 of
        // `payload-patterns`).
        if let Some(name) = binding_name(expr) {
            if self.guard_binding(name).is_some() {
                self.errors.push(TypeError::MoveInGuard {
                    name: name.to_string(),
                    span: expr.span,
                });
                return;
            }
        }
        let Some((source, borrowed)) = self.move_source(expr) else {
            return;
        };
        let clone = self.is_clone(ty).then(|| match ty {
            Type::Nullable(_) => format!("{}?.clone()", expr),
            _ => format!("{}.clone()", expr),
        });
        // A safe navigation is not a place, so borrowing it would move the field too; a default
        // value's input is already borrowed.
        let is_borrowed = borrowed
            || matches!(
                expr.kind,
                ExprKind::MemberAccess {
                    is_safe_nav: true,
                    ..
                }
            );
        // A field of a whole owned struct local can be taken out by destructuring it (D7 of
        // `payload-patterns`).
        let destructure = match &expr.kind {
            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav: false,
            } => self
                .destructurable_local(target)
                .map(|local| format!("val ({}) = {}", member, local)),
            _ => None,
        };
        let captured = match &expr.kind {
            ExprKind::Ident(name) => !self.is_default_input(name) && self.is_capture(name),
            ExprKind::This => !self.is_default_input(THIS) && self.is_capture(THIS),
            _ => false,
        };
        // An element of a list is taken out by `removeAt` (D7 of `collections`).
        let remove = match &expr.kind {
            ExprKind::Index { target, index } => Some(format!("{}.removeAt({})", target, index)),
            _ => None,
        };
        self.errors.push(TypeError::CannotMoveOut {
            source,
            ty: ty.clone(),
            borrow: (!is_borrowed).then(|| format!("&{}", expr)),
            clone,
            destructure,
            unbox: None,
            remove,
            captured,
            span: expr.span,
        });
    }

    /// The name of the whole owned local of struct type that `expr` names, when a destructuring
    /// declaration could take it apart: a local or by-value parameter that no closure or default
    /// value borrows, or `this` of a `consuming fun`.
    fn destructurable_local(&self, expr: &Expr) -> Option<String> {
        let name = match &expr.kind {
            ExprKind::Ident(name) => name.as_str(),
            ExprKind::This => THIS,
            _ => return None,
        };
        let sym = self.env.lookup(name)?;
        let is_struct_local = matches!(
            &sym.kind,
            SymbolKind::Variable {
                ty: Type::Struct { .. },
                local: Some(_),
                ..
            }
        );
        let borrowed = self.is_default_input(name) || self.is_capture(name);
        (is_struct_local && !borrowed).then(|| name.to_string())
    }

    /// What a move or a take-apart of `expr` would take a value out of, when that is not a whole
    /// owned value: its description, and whether it is already a borrow (so that borrowing it
    /// is no fix). `None` for a whole local or parameter, a temporary, or a pass-through form,
    /// whose children are checked instead.
    fn move_source(&self, expr: &Expr) -> Option<(String, bool)> {
        let source = match &expr.kind {
            ExprKind::Ident(name) if self.is_default_input(name) => (
                format!("parameter '{}', which a default value borrows", name),
                true,
            ),
            ExprKind::This if self.is_default_input(THIS) => {
                ("'this', which a default value borrows".to_string(), true)
            }
            ExprKind::Ident(name) if self.is_capture(name) => {
                (format!("captured variable '{}'", name), true)
            }
            ExprKind::This if self.is_capture(THIS) => ("captured 'this'".to_string(), true),
            ExprKind::Ident(name) if self.is_top_level_value(name) => {
                (format!("top-level value '{}'", name), false)
            }
            // `Color.Red` names a variant: a fresh value, not a field.
            ExprKind::MemberAccess { target, .. } if self.type_name_symbol(target).is_some() => {
                return None
            }
            ExprKind::MemberAccess { target, member, .. } => {
                (format!("field '{}' of '{}'", member, target), false)
            }
            ExprKind::Index { target, .. } => (format!("an element of '{}'", target), false),
            _ => return None,
        };
        Some(source)
    }

    /// Whether `name` resolves to a binding that the default value being checked borrows (`this`
    /// or an earlier parameter of its function), outside any closure of the default.
    fn is_default_input(&self, name: &str) -> bool {
        let Some((inputs, depth)) = &self.default_inputs else {
            return false;
        };
        if self.builder.closure_depth() != *depth {
            return false;
        }
        self.env
            .lookup(name)
            .and_then(Self::local_of)
            .is_some_and(|local| inputs.contains(&local))
    }

    /// Whether `name` resolves to a binding that the innermost open closure captures, and so
    /// borrows (D11 of `borrowck-exclusivity`).
    fn is_capture(&self, name: &str) -> bool {
        self.env
            .lookup(name)
            .and_then(Self::local_of)
            .is_some_and(|local| self.builder.is_capture(local))
    }

    /// Whether `name` currently resolves to a top-level `val`, declared in this file or imported,
    /// rather than to a local, a parameter, a function or a payload-free enum variant.
    fn is_top_level_value(&self, name: &str) -> bool {
        let is_variable = matches!(
            self.env.lookup(name).map(|sym| &sym.kind),
            Some(SymbolKind::Variable { .. })
        );
        is_variable && !self.env.resolves_to_local(name) && !self.variant_owners.contains_key(name)
    }

    /// Whether the struct behind `ty` (looking through borrows, `Heap<T>` and `?`) declares a
    /// field named `name`. The built-in `length` of a `String` counts as a field (D6 of
    /// `runtime-strings-and-drops`).
    fn has_field(&self, ty: &Type, name: &str) -> bool {
        let mut ty = ty;
        loop {
            match ty {
                Type::Reference { inner, .. } | Type::Heap(inner) | Type::Nullable(inner) => {
                    ty = inner
                }
                Type::Struct { id, .. } => {
                    return self
                        .struct_fields(id)
                        .is_some_and(|fields| fields.iter().any(|(f, _, _)| f == name))
                }
                // Intrinsic properties read like fields: `length` of a `String`, `size` and
                // `indices` of a list (D7 of `standard-library`).
                Type::String | Type::List(_) => return self.has_intrinsic_property(ty, name),
                _ => return false,
            }
        }
    }

    /// Types a call to the built-in `clone()` member, `receiver.clone()` or `receiver?.clone()`
    /// (the `ownership` spec, "Explicit Clone"). The receiver is read in place, through any
    /// `&`/`&mut`, and the result is an owned value of the borrowed type. The call is a call of
    /// the built-in clone function with a shared receiver borrow (D8 of `typed-tree`).
    fn infer_clone_call(
        &mut self,
        receiver: ExpressionId,
        is_safe_nav: bool,
        args: &[CallArg],
        span: Span,
    ) -> ExpressionId {
        let receiver_ty = self.ty(receiver);
        let mut children = vec![receiver];
        for arg in args {
            children.push(self.infer_expr(&arg.value));
        }
        if !args.is_empty() {
            self.errors.push(TypeError::CallSignatureMismatch {
                message: format!("'clone()' takes no arguments, found {}", args.len()),
                span,
            });
        }
        let (cloned, nullable) = clone_target(&receiver_ty, is_safe_nav);
        if cloned == Type::Error {
            return self.error_node(children, span);
        }
        if !self.is_clone(&cloned) {
            self.errors
                .push(TypeError::NotCloneable { ty: cloned, span });
            return self.error_node(children, span);
        }
        if !args.is_empty() {
            return self.error_node(children, span);
        }
        let (joined, base) = if nullable {
            self.open_chain(receiver)
        } else {
            (None, receiver)
        };
        let place = self.peel_reference_nodes(base);
        let borrow = self.borrow(false, BorrowOrigin::Receiver, place);
        let borrow_ty = self.ty(borrow);
        let call = Call {
            callee: Callee::Builtin(BuiltinFunction::Clone),
            arguments: vec![Argument::Written(borrow)],
            parameters: vec![(Some(THIS.to_string()), borrow_ty)],
            escaping: vec![false],
            type_arguments: Vec::new(),
            evaluation_order: vec![0],
            has_receiver: true,
            has_context: false,
            is_compose: false,
        };
        let call = self.add(ExpressionKind::Call(call), cloned, span);
        if nullable {
            self.close_chain(joined, call, SafeNavigationOrigin::Written, span)
        } else {
            call
        }
    }

    /// A call of the built-in string member `builtin` (`length` or `get`) on the string `base`,
    /// with a shared receiver borrow of it and then `arguments` (D5 and D6 of
    /// `runtime-strings-and-drops`). A string that is not a place is borrowed in a temporary.
    fn builtin_member_call(
        &mut self,
        builtin: BuiltinFunction,
        base: ExpressionId,
        arguments: Vec<ExpressionId>,
        span: Span,
    ) -> ExpressionId {
        let result = match builtin {
            BuiltinFunction::StringCharacter => Type::Char,
            _ => Type::Int,
        };
        let borrow = self.borrow(false, BorrowOrigin::Receiver, base);
        let mut parameters = vec![(Some(THIS.to_string()), self.ty(borrow))];
        let mut written = vec![Argument::Written(borrow)];
        for argument in arguments {
            parameters.push((None, self.ty(argument)));
            written.push(Argument::Written(argument));
        }
        let call = Call {
            callee: Callee::Builtin(builtin),
            evaluation_order: (0..written.len()).collect(),
            escaping: vec![false; parameters.len()],
            arguments: written,
            parameters,
            type_arguments: Vec::new(),
            has_receiver: true,
            has_context: false,
            is_compose: false,
        };
        self.add(ExpressionKind::Call(call), result, span)
    }

    /// A call of the list member `builtin` (`size` or `indices`) on the list `base` of elements
    /// of type `element`, with a shared receiver borrow of it (D6 of `collections`). A list that
    /// is not a place is borrowed in a temporary.
    fn list_member_read(
        &mut self,
        builtin: BuiltinFunction,
        element: Type,
        base: ExpressionId,
        span: Span,
    ) -> ExpressionId {
        let result = builtin
            .list_member_signature()
            .expect("a list member")
            .result;
        let borrow = self.borrow(false, BorrowOrigin::Receiver, base);
        let call = Call {
            callee: Callee::Builtin(builtin),
            arguments: vec![Argument::Written(borrow)],
            parameters: builtin.list_member_parameters().expect("a list member"),
            escaping: vec![false],
            type_arguments: vec![element],
            evaluation_order: vec![0],
            has_receiver: true,
            has_context: false,
            is_compose: false,
        };
        self.add(ExpressionKind::Call(call), result, span)
    }

    /// Types a call of the built-in `toString()` of an `Int`, `Double`, `Bool` or `Char`,
    /// `receiver.toString()` or `receiver?.toString()` (D6 of `runtime-strings-and-drops`): the
    /// receiver is copied out of its references, and the result is a new `String`. It takes no
    /// arguments (`E4004`).
    fn infer_to_string_call(
        &mut self,
        receiver: ExpressionId,
        is_safe_nav: bool,
        type_arguments: &[prism_ast::types::Type],
        args: &[CallArg],
        span: Span,
    ) -> ExpressionId {
        self.reject_type_arguments("'toString'", type_arguments);
        if !args.is_empty() {
            let mut children = vec![receiver];
            for arg in args {
                children.push(self.infer_expr(&arg.value));
            }
            self.errors.push(TypeError::CallSignatureMismatch {
                message: format!("'toString()' takes no arguments, found {}", args.len()),
                span,
            });
            return self.error_node(children, span);
        }
        let nullable = clone_target(&self.ty(receiver), is_safe_nav).1;
        let (joined, base) = if nullable {
            self.open_chain(receiver)
        } else {
            (None, receiver)
        };
        let place = self.peel_indirection_nodes(base);
        let copy = self.copy(place);
        let copy_ty = self.ty(copy);
        let call = Call {
            callee: Callee::Builtin(BuiltinFunction::ToString),
            arguments: vec![Argument::Written(copy)],
            parameters: vec![(Some(THIS.to_string()), copy_ty)],
            escaping: vec![false],
            type_arguments: Vec::new(),
            evaluation_order: vec![0],
            has_receiver: true,
            has_context: false,
            is_compose: false,
        };
        let call = self.add(ExpressionKind::Call(call), Type::String, span);
        if nullable {
            self.close_chain(joined, call, SafeNavigationOrigin::Written, span)
        } else {
            call
        }
    }

    fn infer_member_access(
        &mut self,
        target: &Expr,
        member: &str,
        is_safe_nav: bool,
        span: Span,
    ) -> ExpressionId {
        if let Some((node, _)) = self.type_name_member(target, member, span) {
            return node;
        }
        let target_id = self.infer_read(target);
        let target_ty = self.ty(target_id);
        if !self.has_member(&target_ty, member) && self.extension_named(&target_ty, member) {
            // An extension is not a value: only a call reaches it.
            self.errors.push(TypeError::UnknownField {
                target_type: peel_indirection(&target_ty).clone(),
                field_name: member.to_string(),
                span,
                help: Some(format!(
                    "'{}' is an extension function, not a field: call it with '{}{}{}()'",
                    member,
                    target,
                    if is_safe_nav { "?." } else { "." },
                    member
                )),
            });
            return self.error_node(vec![target_id], span);
        }
        self.build_member_access(target_id, member, is_safe_nav, span)
            .0
    }

    /// Builds `member` accessed on the node `target`, reporting unknown members like
    /// [`Self::member_type`]: a field of the struct reached through every `&`, `&mut` and
    /// `Heap<T>` layer, each an explicit dereference, or a variant of the enum reached; inside a
    /// chain for `?.` on a nullable, which joins a chain `target` is (D5 and D7 of
    /// `typed-tree`). Returns the node and, for a field, whether it is declared `var`.
    fn build_member_access(
        &mut self,
        target: ExpressionId,
        member: &str,
        is_safe_nav: bool,
        span: Span,
    ) -> (ExpressionId, Option<bool>) {
        let target_ty = self.ty(target);
        let (member_ty, is_var) = self.member_type(&target_ty, member, is_safe_nav, span);
        if member_ty == Type::Error {
            return (self.error_node(vec![target], span), is_var);
        }
        // The navigation `member_type` made: through the references to a nullable for `?.`.
        let navigated = match peel_references(&target_ty) {
            nullable @ Type::Nullable(_) if is_safe_nav => nullable,
            _ => &target_ty,
        };
        let (base_ty, through_nullable) = match navigated {
            Type::Nullable(inner) => (inner.as_ref(), true),
            other => (other, false),
        };
        let reached = peel_indirection(base_ty).clone();
        if let (Type::Enum { .. }, Type::Fn { .. }) = (&reached, &member_ty) {
            // A payload variant read through a value is its constructor; the value is
            // evaluated and discarded.
            let variant = self.variant_node(&reached, member, member_ty, span);
            return (self.discard_then(target, variant), is_var);
        }
        let (joined, base) = if through_nullable {
            self.open_chain(target)
        } else {
            (None, target)
        };
        let base = self.peel_indirection_nodes(base);
        let body = match &reached {
            Type::Struct { id, arguments } => {
                let fields = self
                    .types
                    .instantiated_fields(id, arguments)
                    .unwrap_or_default();
                let index = fields
                    .iter()
                    .position(|(name, _, _)| name == member)
                    .expect("member_type found the field");
                let field_ty = fields[index].2.clone();
                let base = self.as_place(base);
                let kind = ExpressionKind::Field {
                    base,
                    index,
                    name: member.to_string(),
                };
                self.add(kind, field_ty, span)
            }
            Type::String => {
                self.builtin_member_call(BuiltinFunction::StringLength, base, Vec::new(), span)
            }
            Type::List(element) => {
                let builtin = if member == LIST_SIZE {
                    BuiltinFunction::ListSize
                } else {
                    BuiltinFunction::ListIndices
                };
                self.list_member_read(builtin, (**element).clone(), base, span)
            }
            _ => {
                let variant = self.variant_node(&reached, member, reached.clone(), span);
                self.discard_then(base, variant)
            }
        };
        let node = if through_nullable {
            self.close_chain(joined, body, SafeNavigationOrigin::Written, span)
        } else {
            body
        };
        (node, is_var)
    }

    /// The branch `id` of an `if` without `else` or of a `when` statement, whose value is
    /// discarded: a block among them holds its last expression as a statement, with no tail, as
    /// a block checked against `Unit` does (D9 of `typed-tree`). Only the node changes; the
    /// branch was checked as before.
    fn discard_branch(&mut self, id: ExpressionId) -> ExpressionId {
        if let ExpressionKind::Block {
            statements,
            tail: Some(tail),
            close_brace,
        } = self.builder.kind(id).clone()
        {
            let span = self.builder.span_of(tail);
            let statement = self
                .builder
                .add_statement(TreeStatementKind::Expression { value: tail }, span);
            let mut statements = statements;
            statements.push(statement);
            let expression = self.builder.expression_mut(id);
            expression.kind = ExpressionKind::Block {
                statements,
                tail: None,
                close_brace,
            };
            expression.ty = Type::Unit;
        }
        id
    }

    /// A block that evaluates `discarded` as a statement, then has the value `value`.
    fn discard_then(&mut self, discarded: ExpressionId, value: ExpressionId) -> ExpressionId {
        let span = self.builder.span_of(discarded);
        let statement = self
            .builder
            .add_statement(TreeStatementKind::Expression { value: discarded }, span);
        let (ty, value_span) = (self.ty(value), self.builder.span_of(value));
        let kind = ExpressionKind::Block {
            statements: vec![statement],
            tail: Some(value),
            close_brace: value_span,
        };
        self.add(kind, ty, value_span)
    }

    /// Whether the struct or enum behind `ty` has a field or variant named `name`.
    fn has_member(&self, ty: &Type, name: &str) -> bool {
        if self.has_field(ty, name) {
            return true;
        }
        let mut ty = ty;
        while let Type::Reference { inner, .. } | Type::Heap(inner) | Type::Nullable(inner) = ty {
            ty = inner;
        }
        match ty {
            Type::Enum { id, .. } => self.variant_index(id, name).is_some(),
            _ => false,
        }
    }

    /// Whether an extension named `name` is visible on the type behind `ty`, at any layer of
    /// references, `Heap<T>` and `?`.
    fn extension_named(&self, ty: &Type, name: &str) -> bool {
        let mut ty = ty;
        loop {
            if !matches!(self.find_extension(ty, name), ExtensionLookup::None) {
                return true;
            }
            match ty {
                Type::Reference { inner, .. } | Type::Heap(inner) | Type::Nullable(inner) => {
                    ty = inner
                }
                _ => return false,
            }
        }
    }

    /// Types `member` accessed on a value of type `target_ty`, reporting unknown members.
    ///
    /// Returns the member's type and, when the member is a struct field, whether that field is
    /// declared `var`.
    fn member_type(
        &mut self,
        target_ty: &Type,
        member: &str,
        is_safe_nav: bool,
        span: Span,
    ) -> (Type, Option<bool>) {
        if target_ty == &Type::Error {
            return (Type::Error, None);
        }
        // Safe navigation reads through references to a nullable, as through `(&T)?` (D8 of
        // `reference-access`).
        let target_ty = match peel_references(target_ty) {
            nullable @ Type::Nullable(_) if is_safe_nav => nullable,
            _ => target_ty,
        };

        let (base_ty, is_nullable) = match target_ty {
            Type::Nullable(inner) => {
                if is_safe_nav {
                    (inner.as_ref(), true)
                } else {
                    self.errors.push(TypeError::UnknownField {
                        target_type: target_ty.clone(),
                        field_name: member.to_string(),
                        span,
                        help: None,
                    });
                    return (Type::Error, None);
                }
            }
            other => (other, false),
        };
        let wrap = |ty: Type| if is_nullable { ty.nullable() } else { ty };

        let peeled_ty = peel_indirection(base_ty).clone();
        if self.reject_unimported_members(&peeled_ty, span) {
            return (Type::Error, None);
        }
        match &peeled_ty {
            Type::Struct { id, arguments } if self.members_visible(id) => {
                // Fields have their declared types with the type arguments substituted.
                let field = self.types.field_index(id, member).and_then(|index| {
                    let is_var = self.types.struct_fields(id)?[index].1;
                    Some((is_var, self.types.instantiated_field(id, arguments, index)?))
                });
                if let Some((is_var, f_ty)) = field {
                    (wrap(f_ty), Some(is_var))
                } else {
                    self.errors.push(TypeError::UnknownField {
                        target_type: peeled_ty.clone(),
                        field_name: member.to_string(),
                        span,
                        help: None,
                    });
                    (Type::Error, None)
                }
            }
            Type::Enum { id, arguments } if self.members_visible(id) => {
                // Through the enum's name (`Option.Some`), the enum is applied to its own
                // parameters, which the use solves (D8 of `generics`).
                let arguments = if arguments.is_empty() {
                    parameter_types(self.types.parameters(id))
                } else {
                    arguments.clone()
                };
                let variant = self
                    .types
                    .variant_index(id, member)
                    .and_then(|index| self.types.instantiated_variant(id, &arguments, index));
                if let Some(payloads) = &variant {
                    let enum_ty = Type::Enum {
                        id: id.clone(),
                        arguments,
                    };
                    if payloads.is_empty() {
                        (wrap(enum_ty), None)
                    } else {
                        let ctor = Type::function(false, payloads.clone(), enum_ty);
                        (ctor, None)
                    }
                } else {
                    self.errors.push(TypeError::UnknownField {
                        target_type: peeled_ty.clone(),
                        field_name: member.to_string(),
                        span,
                        help: None,
                    });
                    (Type::Error, None)
                }
            }
            // The built-in `length`, which cannot be assigned: strings are immutable.
            Type::String
                if member == STRING_LENGTH && self.has_intrinsic_property(&peeled_ty, member) =>
            {
                (wrap(Type::Int), Some(false))
            }
            // The intrinsic `size` and `indices` of a list, which are not assignable (D6 of
            // `collections`).
            Type::List(_)
                if member == LIST_SIZE && self.has_intrinsic_property(&peeled_ty, member) =>
            {
                (wrap(Type::Int), Some(false))
            }
            Type::List(_)
                if member == LIST_INDICES && self.has_intrinsic_property(&peeled_ty, member) =>
            {
                (
                    wrap(Type::plain_struct(NominalId::builtin_range())),
                    Some(false),
                )
            }
            other => {
                let help = matches!(other, Type::Parameter(_)).then(|| {
                    format!(
                        "'{}' is a type parameter, which has no members until traits exist",
                        other
                    )
                });
                self.errors.push(TypeError::UnknownField {
                    target_type: other.clone(),
                    field_name: member.to_string(),
                    span,
                    help,
                });
                (Type::Error, None)
            }
        }
    }

    /// Infers `target[index]` (D7 of `collections`), returning its node and whether the element
    /// may be written. The target is read in place when it is a place and consumed into a
    /// temporary otherwise, then read through its `&`, `&mut` and `Heap<T>` layers; the index
    /// is an `Int`. An element of a list is the place `Index { base, index }`, writable when
    /// every layer above it is. A string's character is computed, not stored: `s[i]` is a call
    /// of the built-in `get` with a shared receiver borrow of the string (D5 of
    /// `runtime-strings-and-drops`), never writable. Any other target is `E4002`.
    fn infer_index(&mut self, target: &Expr, index: &Expr, span: Span) -> (ExpressionId, bool) {
        let (target_id, target_writable) = if self.is_place(target) {
            self.infer_place(target)
        } else {
            (self.with_consuming(true, |c| c.infer_expr(target)), true)
        };
        let index_id = self.with_origin(Origin::Value, |c| c.check_expr(index, &Type::Int));
        let target_ty = self.ty(target_id);
        let base = self.peel_indirection_nodes(target_id);
        match self.ty(base) {
            Type::List(element) => {
                let base = self.as_place(base);
                let writable = contents_writable(&target_ty, target_writable);
                let kind = ExpressionKind::Index {
                    base,
                    index: index_id,
                };
                (self.add(kind, *element, span), writable)
            }
            Type::String => {
                let call = self.builtin_member_call(
                    BuiltinFunction::StringCharacter,
                    base,
                    vec![index_id],
                    span,
                );
                (call, false)
            }
            Type::Error => (self.error_node(vec![base, index_id], span), false),
            other => {
                self.errors.push(TypeError::TypeMismatch {
                    expected: Type::String,
                    found: other,
                    span,
                    help: Some("only a 'List' or a 'String' can be indexed".to_string()),
                });
                (self.error_node(vec![base, index_id], span), false)
            }
        }
    }

    /// Infers a place expression (a binding or a field path such as `o.user.name`), returning
    /// its node together with whether the place itself may be written.
    ///
    /// A binding is writable when it is `var`. A field is writable when it is `var` and the value
    /// holding it is writable, where a `&mut` hop grants and a `&` hop denies write access
    /// regardless of the binding that holds the reference. Any other expression is a temporary,
    /// which is writable unless it is reached through a shared reference.
    fn infer_place(&mut self, expr: &Expr) -> (ExpressionId, bool) {
        if let Some(field) = self.implicit_this_field(expr) {
            return self.infer_place(&field);
        }
        match &expr.kind {
            // `this` is a `val`: a `mut fun` writes through it, never to it.
            ExprKind::This => (self.infer_expr(expr), false),
            ExprKind::Ident(name) => {
                let id = self.infer_expr(expr);
                let writable = matches!(
                    self.env.lookup(name).map(|sym| &sym.kind),
                    Some(SymbolKind::Variable {
                        mutability: Mutability::Var,
                        ..
                    })
                );
                (id, writable)
            }
            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav,
            } => {
                if let Some((node, _)) = self.type_name_member(target, member, expr.span) {
                    return (node, true);
                }
                let (base, base_writable) = self.infer_place(target);
                let base_ty = self.ty(base);
                let (node, is_var) =
                    self.build_member_access(base, member, *is_safe_nav, expr.span);
                // An enum variant is a fresh temporary rather than a field.
                let writable = match is_var {
                    Some(is_var) => is_var && contents_writable(&base_ty, base_writable),
                    None => true,
                };
                (node, writable)
            }
            ExprKind::Index { target, index } => self.infer_index(target, index, expr.span),
            // Any other root is a temporary, so what its branches and `?:` operands give is
            // consumed (the `ownership` spec, "Consuming Positions").
            _ => (self.infer_read(expr), true),
        }
    }
}

/// The parameters of a callee as a call records them: each with its name, when the callee is a
/// declared function, and its type.
fn parameter_list(names: &[String], params: &[Type]) -> Vec<(Option<String>, Type)> {
    params
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            let name = names.get(index).filter(|name| !name.is_empty()).cloned();
            (name, ty.clone())
        })
        .collect()
}

/// The type of `left op right` for the arithmetic operator `op`, when it applies: `Int` with
/// `Int`, `Float` with `Float`, and `String` with `String` for `+`. There is no implicit
/// widening.
fn arithmetic_result(op: BinaryOp, left: &Type, right: &Type) -> Option<Type> {
    match (left, right) {
        (Type::Int, Type::Int) => Some(Type::Int),
        (Type::Float, Type::Float) => Some(Type::Float),
        (Type::String, Type::String) if op == BinaryOp::Add => Some(Type::String),
        _ => None,
    }
}

/// The help of an `OperatorMismatch` for the compound operator `compound` (`+=`) of `op`: the
/// operand types it accepts (D7 of `compound-assignment`).
fn compound_operand_help(op: BinaryOp, compound: &str) -> String {
    if op == BinaryOp::Add {
        format!(
            "'{}' needs a target and a value that are both 'Int', both 'Float' or both 'String'",
            compound
        )
    } else {
        format!(
            "'{}' needs a target and a value that are both 'Int' or both 'Float'",
            compound
        )
    }
}

/// An ordering operator as written in the source.
fn operator_text(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Lt => "<",
        BinaryOp::LtEq => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::GtEq => ">=",
        BinaryOp::Eq => "==",
        BinaryOp::NotEq => "!=",
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        _ => unreachable!(
            "only arithmetic, equality and ordering operators are reported by their text"
        ),
    }
}

/// The typed-tree operator of an arithmetic or comparison operator.
fn binary_operator(op: BinaryOp) -> BinaryOperator {
    match op {
        BinaryOp::Add => BinaryOperator::Add,
        BinaryOp::Sub => BinaryOperator::Subtract,
        BinaryOp::Mul => BinaryOperator::Multiply,
        BinaryOp::Div => BinaryOperator::Divide,
        BinaryOp::Mod => BinaryOperator::Remainder,
        BinaryOp::Eq => BinaryOperator::Equal,
        BinaryOp::NotEq => BinaryOperator::NotEqual,
        BinaryOp::Lt => BinaryOperator::Less,
        BinaryOp::LtEq => BinaryOperator::LessOrEqual,
        BinaryOp::Gt => BinaryOperator::Greater,
        BinaryOp::GtEq => BinaryOperator::GreaterOrEqual,
        BinaryOp::And
        | BinaryOp::Or
        | BinaryOp::Elvis
        | BinaryOp::Range
        | BinaryOp::HalfOpenRange => {
            unreachable!(
                "'{}' is not an arithmetic or comparison operator",
                op.as_str()
            )
        }
    }
}

/// The implicit borrow that passes an argument of type `found` to a reference parameter of type
/// `expected` (D1, D2 and D8 of `reference-access`): a place is borrowed where it lives and must
/// have exactly the referent's type, while any other argument is a temporary of a type
/// assignable to it. `&mut` is never borrowed implicitly.
fn implicit_borrow(found: &Type, expected: &Type, is_place: bool) -> Option<Adjustment> {
    if matches!(found, Type::Reference { .. }) {
        return None;
    }
    let shared = |ty: &Type| Type::Reference {
        mutable: false,
        inner: Box::new(ty.clone()),
    };
    match expected {
        Type::Reference {
            mutable: false,
            inner,
        } => {
            let fits = if is_place {
                shared(found).is_assignable_to(expected)
            } else {
                found.is_assignable_to(inner)
            };
            fits.then_some(if is_place {
                Adjustment::Borrow
            } else {
                Adjustment::BorrowTemporary
            })
        }
        // `(&T)?`: a `T?` place lends its contents; a temporary is converted, then borrowed.
        Type::Nullable(inner) => match inner.as_ref() {
            Type::Reference {
                mutable: false,
                inner: referent,
            } => {
                if is_place {
                    match found {
                        Type::Nullable(contents) => shared(contents)
                            .is_assignable_to(inner)
                            .then_some(Adjustment::BorrowContents),
                        other => shared(other)
                            .is_assignable_to(inner)
                            .then_some(Adjustment::Borrow),
                    }
                } else {
                    found
                        .is_assignable_to(&referent.clone().nullable())
                        .then_some(Adjustment::BorrowTemporary)
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// How a box's contents are borrowed to fit `expected`, a reference or optional reference, when
/// `found` holds a `Heap<T>` (D4 of `heap-boxes`): the steps through the references, boxes and at
/// most one nullable of `found` that reach a place of the referent's exact type, with at least
/// one box among them. An owned box (or nullable box) lends its contents only to a shared
/// reference parameter; a borrowed one converts anywhere, mutably only through `&mut` layers.
fn box_contents(found: &Type, expected: &Type, site: Site) -> Option<Adjustment> {
    let (target, nullable) = match expected {
        Type::Reference { .. } => (expected, false),
        Type::Nullable(inner) if matches!(**inner, Type::Reference { .. }) => {
            (inner.as_ref(), true)
        }
        _ => return None,
    };
    let Type::Reference { mutable, .. } = target else {
        return None;
    };
    let argument = matches!(site, Site::Argument { .. });
    let owned = !matches!(found, Type::Reference { .. });
    if owned && (*mutable || !argument) {
        return None;
    }
    let (mut ty, mut steps) = (found, 0);
    let (mut saw_box, mut entered_nullable, mut writable) = (false, false, true);
    loop {
        if saw_box && entered_nullable == nullable && (!*mutable || writable) {
            let reference = Type::Reference {
                mutable: *mutable,
                inner: Box::new(ty.clone()),
            };
            if reference.is_assignable_to(target) {
                return Some(Adjustment::BoxContents {
                    steps,
                    through_nullable: nullable,
                    mutable: *mutable,
                    origin: if argument {
                        BorrowOrigin::Argument
                    } else {
                        BorrowOrigin::Conversion
                    },
                });
            }
        }
        ty = match ty {
            Type::Reference { mutable, inner } => {
                writable &= *mutable;
                inner
            }
            Type::Heap(inner) => {
                saw_box = true;
                inner
            }
            Type::Nullable(inner) if nullable && !entered_nullable => {
                entered_nullable = true;
                inner
            }
            _ => return None,
        };
        steps += 1;
    }
}

/// The help of writing the contents of the box `reference` from a place that may not be
/// written (D5 of `heap-boxes`).
fn box_write_help(reference: &Expr, behind_reference: bool) -> String {
    match &reference.kind {
        _ if behind_reference => {
            "reach the box through a mutable borrow '&mut', 'var' bindings and 'var' fields"
                .to_string()
        }
        ExprKind::Ident(name) => format!(
            "declare '{}' with 'var', or move it into a 'var' binding, as in 'var m = {}'",
            name, name
        ),
        ExprKind::MemberAccess { member, .. } => format!(
            "declare the field '{}' with 'var', and reach it through 'var' bindings or '&mut' borrows",
            member
        ),
        _ => "write the contents through a 'var' binding holding the box".to_string(),
    }
}

/// The help of a box given for a box of other contents (D8 of `heap-boxes`): boxes are
/// invariant, so the contents must be converted when the box is built.
/// The help for a function value that would change representation when converted (D8 of
/// `closures`): a closure literal calling it, with one parameter per parameter of `expected`.
fn closure_call_help(expr: &Expr, expected: &Type) -> String {
    let arity = match expected {
        Type::Fn { params, .. } => params.len(),
        Type::Nullable(inner) => match inner.as_ref() {
            Type::Fn { params, .. } => params.len(),
            _ => 0,
        },
        _ => 0,
    };
    const NAMES: [&str; 6] = ["x", "y", "z", "a", "b", "c"];
    let names: Vec<String> = (0..arity)
        .map(|i| match NAMES.get(i) {
            Some(name) => name.to_string(),
            None => format!("p{}", i + 1),
        })
        .collect();
    let call = format!("{}({})", expr, names.join(", "));
    let closure = if names.is_empty() {
        format!("{{ {} }}", call)
    } else {
        format!("{{ {} -> {} }}", names.join(", "), call)
    };
    format!("wrap it in a closure that calls it: '{}'", closure)
}

fn invariant_box_help(found: &Type, expected: &Type) -> Option<String> {
    let expected = match expected {
        Type::Nullable(inner) => inner.as_ref(),
        other => other,
    };
    match (found, expected) {
        (Type::Heap(_), Type::Heap(contents)) => Some(format!(
            "'Heap' is invariant: build the box as a '{}' of the expected contents, as in 'Heap<{}>(...)'",
            expected, contents
        )),
        // D2 of `collections`.
        (Type::List(_), Type::List(element)) => Some(format!(
            "list types are invariant: build the list as a '{}' of the expected elements, as in 'listOf<{}>(...)'",
            expected, element
        )),
        _ => None,
    }
}

/// `ty` without its `Heap<T>` layers.
fn peel_boxes(mut ty: &Type) -> &Type {
    while let Type::Heap(inner) = ty {
        ty = inner;
    }
    ty
}

/// The borrow to suggest when `expr`, of type `found`, does not fit the reference `expected`
/// but its referent would: a written `&` outside arguments (D1 of `reference-access`), and a
/// written `&mut` everywhere, since a mutable borrow is never implicit.
fn borrow_help(expr: &Expr, found: &Type, expected: &Type, site: Site) -> Option<String> {
    let Type::Reference { mutable, inner } = expected else {
        return None;
    };
    // A box is borrowed at its contents too (D4 of `heap-boxes`).
    if matches!(found, Type::Reference { .. }) || !peel_boxes(found).is_assignable_to(inner) {
        return None;
    }
    match (mutable, site) {
        (true, _) => Some(format!("pass a mutable borrow: '&mut {}'", expr)),
        (false, Site::Value) => Some(format!("borrow it: '&{}'", expr)),
        (false, Site::Argument { .. }) => None,
    }
}

/// Whether a parameter of type `ty` takes a reference: `&T`, `&mut T`, `(&T)?` or `(&mut T)?`.
fn is_reference_parameter(ty: &Type) -> bool {
    match ty {
        Type::Nullable(inner) => matches!(**inner, Type::Reference { .. }),
        other => matches!(other, Type::Reference { .. }),
    }
}

/// The type a reference parameter refers to: `T` for `&T` or `&mut T`, and `T?` for `(&T)?`.
/// The type an argument temporary of type `found` is converted to before it is lent to a
/// parameter of type `expected` (`&T` or `(&T)?`): `T`, or `T?` for a nullable temporary lent
/// as an optional borrow, which lends its payload.
fn temporary_referent(found: &Type, expected: &Type) -> Type {
    match (found, expected) {
        (Type::Nullable(_), _) | (_, Type::Reference { .. }) => {
            referent_of_reference_parameter(expected)
        }
        (_, Type::Nullable(inner)) => referent_of_reference_parameter(inner),
        _ => found.clone(),
    }
}

/// Whether `ty` is the type of `null`, `Nothing?`.
fn is_null_type(ty: &Type) -> bool {
    matches!(ty, Type::Nullable(inner) if **inner == Type::Nothing)
}

fn referent_of_reference_parameter(ty: &Type) -> Type {
    match ty {
        Type::Nullable(inner) => referent_of_reference_parameter(inner).nullable(),
        Type::Reference { inner, .. } => inner.as_ref().clone(),
        other => other.clone(),
    }
}

/// The optional borrow `(&T)?` (or `(&mut T)?`) that a borrowed nullable `&(T?)` (or
/// `&mut (T?)`) can be read as, or `None` for any other type.
fn nullable_view(ty: &Type) -> Option<Type> {
    match ty {
        Type::Reference { mutable, inner } => match inner.as_ref() {
            Type::Nullable(contents) => Some(
                Type::Reference {
                    mutable: *mutable,
                    inner: contents.clone(),
                }
                .nullable(),
            ),
            _ => None,
        },
        _ => None,
    }
}

/// The type the built-in `clone()` copies when called on a receiver of type `receiver_ty`, and
/// whether the call is a safe call on a nullable: the receiver behind its references, or for
/// `?.` on a nullable, its payload behind references.
fn clone_target(receiver_ty: &Type, is_safe_nav: bool) -> (Type, bool) {
    match peel_references(receiver_ty) {
        Type::Nullable(inner) if is_safe_nav => (peel_references(inner).clone(), true),
        other => (other.clone(), false),
    }
}

/// The name of the built-in `length` member of `String` (D6 of `runtime-strings-and-drops`).
const STRING_LENGTH: &str = "length";

/// The names of the built-in `size` and `indices` of a list, read without parentheses (D6 of
/// `collections`).
const LIST_SIZE: &str = "size";
const LIST_INDICES: &str = "indices";

/// The name of the built-in `toString()` of the scalar types.
const TO_STRING: &str = "toString";

/// The name of the built-in `unbox()` of `Heap<T>` (D6 of `heap-boxes`).
const UNBOX: &str = "unbox";

/// The help of an `E4026` diagnostic for the template part `part` of type `ty` (D8 of
/// `string-templates`), with `following`, the text after the part.
fn template_part_help(
    part: &Expr,
    ty: &Type,
    reason: &NoTextFormReason,
    context: TextFormUse,
) -> Vec<String> {
    let operand = template_part_operand(part);
    let mut help = Vec::new();
    let following = match context {
        TextFormUse::Template { following } => following,
        TextFormUse::Printed => "",
    };
    // How the fix is written: inside the template, or as the printed argument.
    let written = |text: String| match context {
        TextFormUse::Template { .. } => format!("${{{}}}", text),
        TextFormUse::Printed => text,
    };
    match reason {
        NoTextFormReason::Nullable => {
            help.push(format!(
                "give a fallback for null: '{}'",
                written(format!("{} ?: ...", operand))
            ));
        }
        NoTextFormReason::NoToString => {
            if matches!(ty, Type::Struct { .. } | Type::Enum { .. }) {
                help.push(format!(
                    "declare 'fun {}.toString(): String' to give it one",
                    ty
                ));
                let mut member = following.strip_prefix('.').unwrap_or_default().chars();
                if following.starts_with('.')
                    && member
                        .next()
                        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                {
                    let name: String = following[1..]
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    help.push(format!(
                        "to insert a member, write '${{{}.{}}}'",
                        operand, name
                    ));
                }
            }
        }
        NoTextFormReason::ToStringReceiver { .. } => {
            help.push(format!(
                "call it explicitly: '{}'",
                written(format!("{}.toString()", operand))
            ));
        }
        NoTextFormReason::ToStringReturns(_) | NoTextFormReason::ToStringNeedsArguments => {}
    }
    help
}

/// Where a value's text form is needed, for the help of `E4026`.
#[derive(Debug, Clone, Copy)]
enum TextFormUse<'a> {
    /// A string template part, followed by the text `following`.
    Template { following: &'a str },
    /// An argument of `print` or `println`.
    Printed,
}

/// The template part `part` as the operand of `?:` or `.` in a help: as written when it binds
/// at least as tightly as both, and parenthesized otherwise, so that following the help keeps
/// the part whole (`${(if (c) a else b) ?: ...}`, not `${if (c) a else b ?: ...}`).
fn template_part_operand(part: &Expr) -> String {
    match &part.kind {
        // A binary or unary expression is written parenthesized already.
        ExprKind::Literal(_)
        | ExprKind::Ident(_)
        | ExprKind::This
        | ExprKind::Call { .. }
        | ExprKind::MemberAccess { .. }
        | ExprKind::Index { .. }
        | ExprKind::Template(_)
        | ExprKind::Binary { .. }
        | ExprKind::Unary { .. } => part.to_string(),
        _ => format!("({})", part),
    }
}

/// Whether values of `ty` have the built-in `toString()`: `Int`, `Double`, `Bool` and `Char`.
fn has_builtin_to_string(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::Float | Type::Bool | Type::Char)
}

/// The binding `expr` names: a variable for an identifier, the receiver for `this`.
fn binding_name(expr: &Expr) -> Option<&str> {
    match &expr.kind {
        ExprKind::Ident(name) => Some(name),
        ExprKind::This => Some(THIS),
        _ => None,
    }
}

/// `this.member`, the member access an implicit `this` stands for (D5 of `extension-methods`).
fn this_member(member: &str, span: Span) -> Expr {
    Expr::new(
        ExprKind::MemberAccess {
            target: Box::new(Expr::new(ExprKind::This, span)),
            member: member.to_string(),
            is_safe_nav: false,
        },
        span,
    )
}

/// The signature a closure literal is checked against: its compose flag, whether it has a
/// receiver, its parameter types, their `escaping` flags and its return type.
type ClosureSignature = (bool, bool, Vec<Type>, Vec<bool>, Type);

/// The function type a closure argument is checked against, for a parameter of declared type
/// `declared`: a function type, possibly nullable.
fn closure_signature(declared: &Type) -> Option<ClosureSignature> {
    match declared {
        Type::Fn {
            is_compose,
            receiver,
            params,
            escaping,
            ret,
        } => Some((
            *is_compose,
            *receiver,
            params.clone(),
            escaping.clone(),
            (**ret).clone(),
        )),
        Type::Nullable(inner) => closure_signature(inner),
        _ => None,
    }
}

/// The help of the operator `op` applied to a value of a type parameter (D5 of `generics`):
/// no bound gives a type parameter equality, orderings or arithmetic until traits exist.
fn type_parameter_operator_help(op: BinaryOp, left: &Type, right: &Type) -> Option<String> {
    if !(left.mentions_parameter_outside_functions()
        || right.mentions_parameter_outside_functions())
    {
        return None;
    }
    let what = match op {
        BinaryOp::Eq | BinaryOp::NotEq => "equality, neither '==' nor '!='",
        BinaryOp::Lt | BinaryOp::LtEq | BinaryOp::Gt | BinaryOp::GtEq => "orderings",
        _ => "arithmetic operators",
    };
    Some(format!(
        "type parameters have no {}, with or without a bound, until traits exist",
        what
    ))
}

/// Whether `name` is a built-in type name the lowering fixes, which takes no type arguments.
fn is_primitive_name(name: &str) -> bool {
    matches!(
        name,
        "Int" | "Float" | "Double" | "Bool" | "Char" | "String" | "Unit" | "Nothing"
    )
}

/// The semantic type parameters of a declaration's written ones, by position.
/// How many structs and enums deep [`TypeChecker::check_finite_sizes`] expands a type: only a
/// generic type growing its own arguments (`struct W<T> { val w: W<Pair<T, T>>? }`) gets there.
const MAXIMUM_EXPANSION_DEPTH: usize = 64;

/// The largest type, in nodes, [`TypeChecker::check_finite_sizes`] expands: a type growing its
/// own arguments can double at each step, so the depth alone does not bound the work.
const MAXIMUM_EXPANDED_TYPE_SIZE: usize = 256;

/// The number of nodes of `ty`.
fn type_size(ty: &Type) -> usize {
    1 + match ty {
        Type::Reference { inner, .. }
        | Type::Nullable(inner)
        | Type::Heap(inner)
        | Type::List(inner) => type_size(inner),
        Type::Struct { arguments, .. } | Type::Enum { arguments, .. } => {
            arguments.iter().map(type_size).sum()
        }
        Type::Fn { params, ret, .. } => {
            params.iter().map(type_size).sum::<usize>() + type_size(ret)
        }
        _ => 0,
    }
}

fn type_parameters_of(parameters: &[prism_ast::TypeParameter]) -> Vec<TypeParameter> {
    parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| TypeParameter {
            index: index as u32,
            name: parameter.name.clone(),
            bound: parameter.bound,
        })
        .collect()
}

/// The type parameters of a declaration as types, the arguments of its own generic type
/// (`Box<T>` inside `struct Box<T>`).
fn parameter_types(parameters: &[TypeParameter]) -> Vec<Type> {
    parameters.iter().map(TypeParameter::as_type).collect()
}

/// Whether `expr` is a leaf whose move [`TypeChecker::check_move_source`] checks: a name, a field
/// access (safe or not) or an index.
fn is_move_leaf(expr: &Expr) -> bool {
    matches!(
        expr.kind,
        ExprKind::Ident(_) | ExprKind::MemberAccess { .. } | ExprKind::Index { .. }
    )
}

/// Whether the value behind a place of type `ty` may be mutated, given whether the place itself
/// is writable. The first reference layer decides (`&mut` grants, `&` denies), and every further
/// reference layer must also be `&mut`. `Heap<T>` owns its contents, so it is transparent.
fn contents_writable(mut ty: &Type, place_writable: bool) -> bool {
    let mut writable = place_writable;
    let mut behind_reference = false;
    loop {
        match ty {
            Type::Reference { mutable, inner } => {
                writable = if behind_reference {
                    writable && *mutable
                } else {
                    *mutable
                };
                behind_reference = true;
                ty = inner;
            }
            Type::Heap(inner) => ty = inner,
            Type::Nullable(inner) => ty = inner,
            _ => return writable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_abi::{AbiType, PrimKind};

    /// Declares a struct with the given fields in `checker`'s module, returning its type.
    fn declare_struct(
        checker: &mut TypeChecker,
        name: &str,
        fields: Vec<(String, bool, Type)>,
    ) -> Type {
        let id = NominalId::new(checker.module_path.clone(), name);
        checker.types.replace_definition(
            id.clone(),
            TypeDef::Struct {
                parameters: Vec::new(),
                fields,
            },
        );
        checker
            .env
            .define(Symbol::struct_decl(
                id.clone(),
                Visibility::Public,
                None,
                Span::dummy(),
            ))
            .unwrap();
        Type::plain_struct(id)
    }

    /// Declares an enum with the given variants in `checker`'s module, returning its type.
    fn declare_enum(
        checker: &mut TypeChecker,
        name: &str,
        variants: Vec<(String, Vec<Type>)>,
    ) -> Type {
        let id = NominalId::new(checker.module_path.clone(), name);
        checker.types.replace_definition(
            id.clone(),
            TypeDef::Enum {
                parameters: Vec::new(),
                variants,
            },
        );
        checker
            .env
            .define(Symbol::enum_decl(
                id.clone(),
                Visibility::Public,
                None,
                Span::dummy(),
            ))
            .unwrap();
        Type::plain_enum(id)
    }

    /// Infers `e` in `checker`, returning its type.
    fn infer(checker: &mut TypeChecker, e: &Expr) -> Type {
        let id = checker.infer_expr(e);
        checker.type_of(id).clone()
    }

    /// Checks `e` against `expected` in `checker`, returning the type of its node.
    fn check(checker: &mut TypeChecker, e: &Expr, expected: &Type) -> Type {
        let id = checker.check_expr(e, expected);
        checker.type_of(id).clone()
    }

    fn statement(kind: StatementKind) -> Statement {
        Statement::new(kind, Span::dummy())
    }

    // Helper to wrap expression in Expr
    fn value_pattern(value: Expr) -> WhenPattern {
        let span = value.span;
        WhenPattern::Pattern(prism_ast::Pattern::new(PatternKind::Value(value), span))
    }

    fn expr(kind: ExprKind) -> Expr {
        Expr::new(kind, Span::dummy())
    }

    #[test]
    fn test_strict_numeric_operations_reject_widening() {
        let mut checker = TypeChecker::new(None, "<test>");
        let plus = expr(ExprKind::Binary {
            op: BinaryOp::Add,
            left: Box::new(expr(ExprKind::Literal(Literal::Int(1)))),
            right: Box::new(expr(ExprKind::Literal(Literal::Float(2.0)))),
        });

        let res = infer(&mut checker, &plus);
        assert_eq!(res, Type::Error);
        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4002");
    }

    #[test]
    fn test_char_literal_and_indexing() {
        let mut checker = TypeChecker::new(None, "<test>");
        let char_expr = expr(ExprKind::Literal(Literal::Char('z')));
        assert_eq!(infer(&mut checker, &char_expr), Type::Char);

        let index_expr = expr(ExprKind::Index {
            target: Box::new(expr(ExprKind::Literal(Literal::String(
                "hello".to_string(),
            )))),
            index: Box::new(expr(ExprKind::Literal(Literal::Int(0)))),
        });
        assert_eq!(infer(&mut checker, &index_expr), Type::Char);
    }

    #[test]
    fn test_val_reassignment_rejected_e4003() {
        let mut checker = TypeChecker::new(None, "<test>");
        let init = statement(StatementKind::Local {
            is_var: false,
            name: "x".to_string(),
            ty: Some(prism_ast::types::Type::named("Int", Span::dummy())),
            initializer: expr(ExprKind::Literal(Literal::Int(42))),
        });
        checker.check_statement(&init);

        let reassign = statement(StatementKind::Assign {
            target: expr(ExprKind::Ident("x".to_string())),
            value: expr(ExprKind::Literal(Literal::Int(100))),
        });
        checker.check_statement(&reassign);

        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4003");
    }

    #[test]
    fn test_error_sentinel_suppresses_cascade() {
        let mut checker = TypeChecker::new(None, "<test>");
        // undefined + 1
        let err_expr = expr(ExprKind::Binary {
            op: BinaryOp::Add,
            left: Box::new(expr(ExprKind::Ident("not_found".to_string()))),
            right: Box::new(expr(ExprKind::Literal(Literal::Int(1)))),
        });
        let res = infer(&mut checker, &err_expr);
        assert_eq!(res, Type::Error);
        // Only 1 error (E4001: unresolved symbol), no cascade E4002 error
        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4001");
    }

    #[test]
    fn test_compose_context_enforcement_e4008() {
        let mut checker = TypeChecker::new(None, "<test>");
        let compose_fn_sym = Symbol::function(
            "MyView",
            Type::function(true, Vec::new(), Type::Unit),
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(compose_fn_sym).unwrap();

        // Calling MyView in non-compose context
        let call = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("MyView".to_string()))),
            type_arguments: Vec::new(),
            args: Vec::new(),
        });
        infer(&mut checker, &call);

        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4008");
    }

    #[test]
    fn test_when_expression_exhaustiveness_e4009() {
        let mut checker = TypeChecker::new(None, "<test>");
        let when_no_else = expr(ExprKind::When {
            target: Some(Box::new(expr(ExprKind::Literal(Literal::Int(1))))),
            branches: vec![WhenBranch {
                patterns: vec![value_pattern(expr(ExprKind::Literal(Literal::Int(1))))],
                guard: None,
                body: expr(ExprKind::Literal(Literal::String("one".to_string()))),
                span: Span::dummy(),
            }],
        });
        infer(&mut checker, &when_no_else);

        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4009");
    }

    #[test]
    fn test_block_unit_and_trailing_expr() {
        let mut checker = TypeChecker::new(None, "<test>");
        let block_empty = expr(ExprKind::Block(Block {
            statements: Vec::new(),
            tail: None,
            span: Span::dummy(),
            close_brace: Span::dummy(),
        }));
        assert_eq!(infer(&mut checker, &block_empty), Type::Unit);

        let block_trailing = expr(ExprKind::Block(Block {
            statements: vec![statement(StatementKind::Local {
                is_var: false,
                name: "a".to_string(),
                ty: None,
                initializer: expr(ExprKind::Literal(Literal::Int(10))),
            })],
            tail: Some(Box::new(expr(ExprKind::Ident("a".to_string())))),
            span: Span::dummy(),
            close_brace: Span::dummy(),
        }));
        assert_eq!(infer(&mut checker, &block_trailing), Type::Int);
    }

    #[test]
    fn test_call_signature_mismatch_e4004() {
        let mut checker = TypeChecker::new(None, "<test>");
        let fn_sym = Symbol::function(
            "add",
            Type::function(false, vec![Type::Int, Type::Int], Type::Int),
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(fn_sym).unwrap();

        // Wrong arity
        let call_wrong_arity = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("add".to_string()))),
            type_arguments: Vec::new(),
            args: vec![CallArg {
                name: None,
                value: expr(ExprKind::Literal(Literal::Int(1))),
                span: Span::dummy(),
                is_trailing_lambda: false,
            }],
        });
        infer(&mut checker, &call_wrong_arity);
        assert_eq!(checker.errors.len(), 1);
        assert_eq!(checker.errors[0].code(), "E4004");

        // Wrong argument type
        let call_wrong_type = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("add".to_string()))),
            type_arguments: Vec::new(),
            args: vec![
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::String("bad".to_string()))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
            ],
        });
        infer(&mut checker, &call_wrong_type);
        assert_eq!(checker.errors.len(), 2);
        assert_eq!(checker.errors[1].code(), "E4004");
    }

    #[test]
    fn test_closure_checking_against_fn_type() {
        let mut checker = TypeChecker::new(None, "<test>");
        let closure = expr(ExprKind::Closure {
            params: vec![prism_ast::ClosureParameter {
                name: "n".to_string(),
                span: Span::dummy(),
            }],
            arrow: Some(Span::dummy()),
            body: Block {
                statements: Vec::new(),
                tail: Some(Box::new(expr(ExprKind::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(expr(ExprKind::Ident("n".to_string()))),
                    right: Box::new(expr(ExprKind::Literal(Literal::Int(1)))),
                }))),
                span: Span::dummy(),
                close_brace: Span::dummy(),
            },
        });

        let expected_fn = Type::function(false, vec![Type::Int], Type::Int);

        let checked = check(&mut checker, &closure, &expected_fn);
        assert_eq!(checked, expected_fn);
        assert!(checker.errors.is_empty());
    }

    #[test]
    fn test_when_diverging_branch_with_nothing() {
        let mut checker = TypeChecker::new(None, "<test>");
        checker.in_function = true;
        checker.current_expected_return = Some(Type::Int);
        let when_expr = expr(ExprKind::When {
            target: Some(Box::new(expr(ExprKind::Literal(Literal::Int(0))))),
            branches: vec![
                WhenBranch {
                    patterns: vec![value_pattern(expr(ExprKind::Literal(Literal::Int(0))))],
                    guard: None,
                    body: expr(ExprKind::Return(Some(Box::new(expr(ExprKind::Literal(
                        Literal::Int(99),
                    )))))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![WhenPattern::Else],
                    guard: None,
                    body: expr(ExprKind::Literal(Literal::Int(42))),
                    span: Span::dummy(),
                },
            ],
        });

        // The 'return' branch has Nothing, so unified type is Int
        let ty = infer(&mut checker, &when_expr);
        assert_eq!(ty, Type::Int);
        assert!(checker.errors.is_empty());
    }

    #[test]
    fn test_struct_instantiation_and_field_access() {
        let mut checker = TypeChecker::new(None, "<test>");
        declare_struct(
            &mut checker,
            "Point",
            vec![
                ("x".to_string(), false, Type::Int),
                ("y".to_string(), true, Type::Int),
            ],
        );

        // Valid positional instantiation
        let call_pos = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            type_arguments: Vec::new(),
            args: vec![
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(10))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(20))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
            ],
        });
        let res_pos = infer(&mut checker, &call_pos);
        assert!(matches!(res_pos, Type::Struct { ref id, .. } if id.name == "Point"));

        // Valid named field instantiation
        let call_named = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            type_arguments: Vec::new(),
            args: vec![
                CallArg {
                    name: Some("x".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(10))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
                CallArg {
                    name: Some("y".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(20))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
            ],
        });
        let res_named = infer(&mut checker, &call_named);
        assert!(matches!(res_named, Type::Struct { ref id, .. } if id.name == "Point"));

        // Field access: p.x -> Int
        let field_access = expr(ExprKind::MemberAccess {
            target: Box::new(call_pos),
            member: "x".to_string(),
            is_safe_nav: false,
        });
        assert_eq!(infer(&mut checker, &field_access), Type::Int);

        // Unknown field access: p.z -> E4006
        let unknown_field = expr(ExprKind::MemberAccess {
            target: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            member: "z".to_string(),
            is_safe_nav: false,
        });
        assert_eq!(infer(&mut checker, &unknown_field), Type::Error);
        assert!(checker.errors.iter().any(|e| e.code() == "E4006"));
    }

    #[test]
    fn test_struct_field_mutability_validation_e4003() {
        let mut checker = TypeChecker::new(None, "<test>");
        declare_struct(
            &mut checker,
            "User",
            vec![
                ("id".to_string(), false, Type::Int),
                ("age".to_string(), true, Type::Int),
            ],
        );

        // val u = User(1, 20)
        let init_val = statement(StatementKind::Local {
            is_var: false,
            name: "u".to_string(),
            ty: None,
            initializer: expr(ExprKind::Call {
                callee: Box::new(expr(ExprKind::Ident("User".to_string()))),
                type_arguments: Vec::new(),
                args: vec![
                    CallArg {
                        name: None,
                        value: expr(ExprKind::Literal(Literal::Int(1))),
                        span: Span::dummy(),
                        is_trailing_lambda: false,
                    },
                    CallArg {
                        name: None,
                        value: expr(ExprKind::Literal(Literal::Int(20))),
                        span: Span::dummy(),
                        is_trailing_lambda: false,
                    },
                ],
            }),
        });
        checker.check_statement(&init_val);

        // u.id = 2 -> E4003 (immutable field)
        let assign_val_field = statement(StatementKind::Assign {
            target: expr(ExprKind::MemberAccess {
                target: Box::new(expr(ExprKind::Ident("u".to_string()))),
                member: "id".to_string(),
                is_safe_nav: false,
            }),
            value: expr(ExprKind::Literal(Literal::Int(2))),
        });
        checker.check_statement(&assign_val_field);
        assert!(checker.errors.iter().any(|e| e.code() == "E4003"));

        // u.age = 21 -> E4003 (immutable binding 'u')
        let assign_val_binding = statement(StatementKind::Assign {
            target: expr(ExprKind::MemberAccess {
                target: Box::new(expr(ExprKind::Ident("u".to_string()))),
                member: "age".to_string(),
                is_safe_nav: false,
            }),
            value: expr(ExprKind::Literal(Literal::Int(21))),
        });
        checker.check_statement(&assign_val_binding);
        assert_eq!(
            checker
                .errors
                .iter()
                .filter(|e| e.code() == "E4003")
                .count(),
            2
        );

        // var u2 = User(2, 30)
        let init_var = statement(StatementKind::Local {
            is_var: true,
            name: "u2".to_string(),
            ty: None,
            initializer: expr(ExprKind::Call {
                callee: Box::new(expr(ExprKind::Ident("User".to_string()))),
                type_arguments: Vec::new(),
                args: vec![
                    CallArg {
                        name: None,
                        value: expr(ExprKind::Literal(Literal::Int(2))),
                        span: Span::dummy(),
                        is_trailing_lambda: false,
                    },
                    CallArg {
                        name: None,
                        value: expr(ExprKind::Literal(Literal::Int(30))),
                        span: Span::dummy(),
                        is_trailing_lambda: false,
                    },
                ],
            }),
        });
        checker.check_statement(&init_var);

        // u2.age = 31 -> OK (mutable field on mutable var binding)
        let initial_err_count = checker.errors.len();
        let assign_var = statement(StatementKind::Assign {
            target: expr(ExprKind::MemberAccess {
                target: Box::new(expr(ExprKind::Ident("u2".to_string()))),
                member: "age".to_string(),
                is_safe_nav: false,
            }),
            value: expr(ExprKind::Literal(Literal::Int(31))),
        });
        checker.check_statement(&assign_var);
        assert_eq!(checker.errors.len(), initial_err_count);
    }

    #[test]
    fn test_enum_variant_constructor_typing() {
        let mut checker = TypeChecker::new(None, "<test>");
        declare_enum(
            &mut checker,
            "Option",
            vec![
                ("None".to_string(), Vec::new()),
                ("Some".to_string(), vec![Type::Int]),
            ],
        );

        // Qualified unit variant: Option.None -> Option
        let none_access = expr(ExprKind::MemberAccess {
            target: Box::new(expr(ExprKind::Ident("Option".to_string()))),
            member: "None".to_string(),
            is_safe_nav: false,
        });
        let res_none = infer(&mut checker, &none_access);
        assert!(matches!(res_none, Type::Enum { ref id, .. } if id.name == "Option"));

        // Qualified variant constructor call: Option.Some(42) -> Option
        let some_call = expr(ExprKind::Call {
            type_arguments: Vec::new(),
            callee: Box::new(expr(ExprKind::MemberAccess {
                target: Box::new(expr(ExprKind::Ident("Option".to_string()))),
                member: "Some".to_string(),
                is_safe_nav: false,
            })),
            args: vec![CallArg {
                name: None,
                value: expr(ExprKind::Literal(Literal::Int(42))),
                span: Span::dummy(),
                is_trailing_lambda: false,
            }],
        });
        let res_some = infer(&mut checker, &some_call);
        assert!(matches!(res_some, Type::Enum { ref id, .. } if id.name == "Option"));

        // Option.Some("bad") -> E4004 (argument type mismatch)
        let some_bad = expr(ExprKind::Call {
            type_arguments: Vec::new(),
            callee: Box::new(expr(ExprKind::MemberAccess {
                target: Box::new(expr(ExprKind::Ident("Option".to_string()))),
                member: "Some".to_string(),
                is_safe_nav: false,
            })),
            args: vec![CallArg {
                name: None,
                value: expr(ExprKind::Literal(Literal::String("bad".to_string()))),
                span: Span::dummy(),
                is_trailing_lambda: false,
            }],
        });
        infer(&mut checker, &some_bad);
        assert!(checker.errors.iter().any(|e| e.code() == "E4004"));
    }

    #[test]
    fn test_reference_and_borrow_typing() {
        let mut checker = TypeChecker::new(None, "<test>");
        let init_val = statement(StatementKind::Local {
            is_var: false,
            name: "x".to_string(),
            ty: None,
            initializer: expr(ExprKind::Literal(Literal::Int(42))),
        });
        checker.check_statement(&init_val);

        // Shared borrow: &x -> &Int
        let shared_borrow = expr(ExprKind::Unary {
            op: UnaryOp::Borrow,
            expr: Box::new(expr(ExprKind::Ident("x".to_string()))),
        });
        assert_eq!(
            infer(&mut checker, &shared_borrow),
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Int),
            }
        );

        // Mutable borrow on val: &mut x -> E4003
        let mut_borrow_val = expr(ExprKind::Unary {
            op: UnaryOp::BorrowMut,
            expr: Box::new(expr(ExprKind::Ident("x".to_string()))),
        });
        infer(&mut checker, &mut_borrow_val);
        assert!(checker.errors.iter().any(|e| e.code() == "E4003"));

        // Mutable borrow on var: &mut y -> &mut Int
        let init_var = statement(StatementKind::Local {
            is_var: true,
            name: "y".to_string(),
            ty: None,
            initializer: expr(ExprKind::Literal(Literal::Int(100))),
        });
        checker.check_statement(&init_var);
        let mut_borrow_var = expr(ExprKind::Unary {
            op: UnaryOp::BorrowMut,
            expr: Box::new(expr(ExprKind::Ident("y".to_string()))),
        });
        assert_eq!(
            infer(&mut checker, &mut_borrow_var),
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Int),
            }
        );
    }

    #[test]
    fn test_nullability_safe_nav_and_elvis() {
        let mut checker = TypeChecker::new(None, "<test>");
        let point_ty = declare_struct(
            &mut checker,
            "Point",
            vec![("x".to_string(), false, Type::Int)],
        );
        let opt_point_ty = Type::Nullable(Box::new(point_ty));
        let init_opt = statement(StatementKind::Local {
            is_var: false,
            name: "opt".to_string(),
            ty: None,
            initializer: expr(ExprKind::Literal(Literal::Null)),
        });
        checker.check_statement(&init_opt);
        checker.env.update_type("opt", opt_point_ty);

        // Safe nav: opt?.x -> Int?
        let safe_nav = expr(ExprKind::MemberAccess {
            target: Box::new(expr(ExprKind::Ident("opt".to_string()))),
            member: "x".to_string(),
            is_safe_nav: true,
        });
        assert_eq!(
            infer(&mut checker, &safe_nav),
            Type::Nullable(Box::new(Type::Int))
        );

        // Elvis: opt_int ?: 0 -> Int
        let elvis = expr(ExprKind::Binary {
            op: BinaryOp::Elvis,
            left: Box::new(expr(ExprKind::Literal(Literal::Null))),
            right: Box::new(expr(ExprKind::Literal(Literal::Int(0)))),
        });
        assert_eq!(infer(&mut checker, &elvis), Type::Int);
    }

    #[test]
    fn test_heap_indirection_field_access() {
        let mut checker = TypeChecker::new(None, "<test>");
        let point_ty = Type::Heap(Box::new(declare_struct(
            &mut checker,
            "Point",
            vec![("x".to_string(), false, Type::Int)],
        )));
        checker
            .env
            .define(Symbol::variable(
                "h",
                point_ty,
                Mutability::Val,
                Span::dummy(),
            ))
            .unwrap();

        // Transparent dereference on Heap<Point>: h.x -> Int
        let field_access = expr(ExprKind::MemberAccess {
            target: Box::new(expr(ExprKind::Ident("h".to_string()))),
            member: "x".to_string(),
            is_safe_nav: false,
        });
        assert_eq!(infer(&mut checker, &field_access), Type::Int);
    }

    #[test]
    fn test_cross_module_abi_ingestion_and_package_visibility() {
        // ABI defined in package 'core.math'
        let abi = FileAbi {
            package: Some("core.math".to_string()),
            module: "core/math/math".to_string(),
            type_definitions: Vec::new(),
            symbols: vec![
                ExportedSymbol::Function {
                    visibility: Visibility::Public,
                    kind: prism_abi::FunctionKind::Prism,
                    is_compose: false,
                    receiver: None,
                    context: None,
                    name: "add".to_string(),
                    type_parameters: Vec::new(),
                    params: vec![
                        prism_abi::AbiParameter {
                            name: "a".to_string(),
                            ty: AbiType::Prim(PrimKind::Int),
                            escaping: false,
                            default: None,
                        },
                        prism_abi::AbiParameter {
                            name: "b".to_string(),
                            ty: AbiType::Prim(PrimKind::Int),
                            escaping: false,
                            default: None,
                        },
                    ],
                    return_type: AbiType::Prim(PrimKind::Int),
                },
                ExportedSymbol::Function {
                    visibility: Visibility::Package,
                    kind: prism_abi::FunctionKind::Prism,
                    is_compose: false,
                    receiver: None,
                    context: None,
                    name: "internal_calc".to_string(),
                    type_parameters: Vec::new(),
                    params: Vec::new(),
                    return_type: AbiType::Prim(PrimKind::Int),
                },
            ],
            abi_hash: prism_abi::ContentHash([1; 32]),
        };

        // Case 1: Ingested from external package 'app.ui'
        let mut ext_checker = TypeChecker::new(Some("app.ui".to_string()), "<test>");
        ext_checker.ingest_file_abi(&abi);

        // Calling public 'add' -> OK
        let call_pub = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("add".to_string()))),
            type_arguments: Vec::new(),
            args: vec![
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
            ],
        });
        assert_eq!(infer(&mut ext_checker, &call_pub), Type::Int);
        assert!(ext_checker.errors.is_empty());

        // Calling package-visible 'internal_calc' from external package -> E4007
        let call_pkg = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("internal_calc".to_string()))),
            type_arguments: Vec::new(),
            args: Vec::new(),
        });
        infer(&mut ext_checker, &call_pkg);
        assert!(ext_checker.errors.iter().any(|e| e.code() == "E4007"));

        // Case 2: Ingested from identical package 'core.math'
        let mut same_checker = TypeChecker::new(Some("core.math".to_string()), "<test>");
        same_checker.ingest_file_abi(&abi);
        assert_eq!(infer(&mut same_checker, &call_pkg), Type::Int);
        assert!(same_checker.errors.is_empty());
    }

    #[test]
    fn test_default_param_omission_and_named_args() {
        let mut checker = TypeChecker::new(None, "<test>");
        let fn_ty = Type::function(
            false,
            vec![Type::String, Type::Int, Type::String],
            Type::String,
        );
        let sym = Symbol::function_with_defaults(
            "greet",
            fn_ty,
            vec!["msg".to_string(), "count".to_string(), "prefix".to_string()],
            vec![false, true, true],
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(sym).unwrap();

        // 1. Omit default parameters: greet("hello") -> OK
        let call_one = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("greet".to_string()))),
            type_arguments: Vec::new(),
            args: vec![CallArg {
                name: None,
                value: expr(ExprKind::Literal(Literal::String("hello".to_string()))),
                span: Span::dummy(),
                is_trailing_lambda: false,
            }],
        });
        assert_eq!(infer(&mut checker, &call_one), Type::String);
        assert!(checker.errors.is_empty());

        // 2. Named arguments: greet(msg = "hi", prefix = ">>") -> OK
        let call_named = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("greet".to_string()))),
            type_arguments: Vec::new(),
            args: vec![
                CallArg {
                    name: Some("msg".to_string()),
                    value: expr(ExprKind::Literal(Literal::String("hi".to_string()))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
                CallArg {
                    name: Some("prefix".to_string()),
                    value: expr(ExprKind::Literal(Literal::String(">>".to_string()))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
            ],
        });
        assert_eq!(infer(&mut checker, &call_named), Type::String);
        assert!(checker.errors.is_empty());

        // 3. Duplicate named argument -> E4004
        let call_dup = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("greet".to_string()))),
            type_arguments: Vec::new(),
            args: vec![
                CallArg {
                    name: Some("msg".to_string()),
                    value: expr(ExprKind::Literal(Literal::String("a".to_string()))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
                CallArg {
                    name: Some("msg".to_string()),
                    value: expr(ExprKind::Literal(Literal::String("b".to_string()))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
            ],
        });
        infer(&mut checker, &call_dup);
        assert!(checker.errors.iter().any(|e| e.code() == "E4004"));
        checker.errors.clear();

        // 4. Missing required parameter -> E4004
        let call_missing = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("greet".to_string()))),
            type_arguments: Vec::new(),
            args: vec![CallArg {
                name: Some("count".to_string()),
                value: expr(ExprKind::Literal(Literal::Int(2))),
                span: Span::dummy(),
                is_trailing_lambda: false,
            }],
        });
        infer(&mut checker, &call_missing);
        assert!(checker.errors.iter().any(|e| e.code() == "E4004"));
    }

    #[test]
    fn test_struct_named_args_and_validation() {
        let mut checker = TypeChecker::new(None, "<test>");
        declare_struct(
            &mut checker,
            "Point",
            vec![
                ("x".to_string(), false, Type::Int),
                ("y".to_string(), false, Type::Int),
            ],
        );

        // Valid named: Point(x = 1, y = 2) -> OK
        let call_ok = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            type_arguments: Vec::new(),
            args: vec![
                CallArg {
                    name: Some("x".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
                CallArg {
                    name: Some("y".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
            ],
        });
        infer(&mut checker, &call_ok);
        assert!(checker.errors.is_empty());

        // Duplicate field: Point(x = 1, x = 2) -> E4004
        let call_dup = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            type_arguments: Vec::new(),
            args: vec![
                CallArg {
                    name: Some("x".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
                CallArg {
                    name: Some("x".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                    is_trailing_lambda: false,
                },
            ],
        });
        infer(&mut checker, &call_dup);
        assert!(checker.errors.iter().any(|e| e.code() == "E4004"));
        checker.errors.clear();

        // Missing field: Point(x = 1) -> E4004
        let call_missing = expr(ExprKind::Call {
            callee: Box::new(expr(ExprKind::Ident("Point".to_string()))),
            type_arguments: Vec::new(),
            args: vec![CallArg {
                name: Some("x".to_string()),
                value: expr(ExprKind::Literal(Literal::Int(1))),
                span: Span::dummy(),
                is_trailing_lambda: false,
            }],
        });
        infer(&mut checker, &call_missing);
        assert!(checker.errors.iter().any(|e| e.code() == "E4004"));
    }

    #[test]
    fn test_exhaustive_enum_when_without_else() {
        let mut checker = TypeChecker::new(None, "<test>");
        let color_ty = declare_enum(
            &mut checker,
            "Color",
            vec![
                ("Red".to_string(), Vec::new()),
                ("Green".to_string(), Vec::new()),
                ("Blue".to_string(), Vec::new()),
            ],
        );
        checker
            .env
            .define(Symbol::variable(
                "c",
                color_ty.clone(),
                Mutability::Val,
                Span::dummy(),
            ))
            .unwrap();
        checker
            .env
            .define(Symbol::top_level_variable(
                "Red",
                color_ty.clone(),
                Mutability::Val,
                Visibility::Public,
                None,
                Span::dummy(),
            ))
            .unwrap();
        checker
            .env
            .define(Symbol::top_level_variable(
                "Green",
                color_ty.clone(),
                Mutability::Val,
                Visibility::Public,
                None,
                Span::dummy(),
            ))
            .unwrap();
        checker
            .env
            .define(Symbol::top_level_variable(
                "Blue",
                color_ty.clone(),
                Mutability::Val,
                Visibility::Public,
                None,
                Span::dummy(),
            ))
            .unwrap();

        // Exhaustive: covers Red, Green, Blue without else -> OK
        let when_exhaustive = expr(ExprKind::When {
            target: Some(Box::new(expr(ExprKind::Ident("c".to_string())))),
            branches: vec![
                WhenBranch {
                    patterns: vec![value_pattern(expr(ExprKind::Ident("Red".to_string())))],
                    guard: None,
                    body: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![value_pattern(expr(ExprKind::Ident("Green".to_string())))],
                    guard: None,
                    body: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![value_pattern(expr(ExprKind::Ident("Blue".to_string())))],
                    guard: None,
                    body: expr(ExprKind::Literal(Literal::Int(3))),
                    span: Span::dummy(),
                },
            ],
        });
        let ty = infer(&mut checker, &when_exhaustive);
        assert_eq!(ty, Type::Int);
        assert!(checker.errors.is_empty());

        // Incomplete: covers only Red, Green without else -> E4009
        let when_incomplete = expr(ExprKind::When {
            target: Some(Box::new(expr(ExprKind::Ident("c".to_string())))),
            branches: vec![
                WhenBranch {
                    patterns: vec![value_pattern(expr(ExprKind::Ident("Red".to_string())))],
                    guard: None,
                    body: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![value_pattern(expr(ExprKind::Ident("Green".to_string())))],
                    guard: None,
                    body: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                },
            ],
        });
        infer(&mut checker, &when_incomplete);
        assert!(checker.errors.iter().any(|e| e.code() == "E4009"));
    }

    #[test]
    fn test_top_level_return_rejected() {
        let mut checker = TypeChecker::new(None, "<test>");
        let ret_expr = expr(ExprKind::Return(Some(Box::new(expr(ExprKind::Literal(
            Literal::Int(42),
        ))))));
        infer(&mut checker, &ret_expr);
        assert!(checker.errors.iter().any(|e| e.code() == "E4005"));
    }

    #[test]
    fn test_compose_isolation_across_closure() {
        let mut checker = TypeChecker::new(None, "<test>");
        let comp_fn = Symbol::function(
            "MyView",
            Type::function(true, Vec::new(), Type::Unit),
            Visibility::Public,
            None,
            Span::dummy(),
        );
        checker.env.define(comp_fn).unwrap();

        // Enclosing context is composable
        checker.push_compose_context(true);

        // A standard (non-composable) closure: { MyView() }
        let closure = expr(ExprKind::Closure {
            params: Vec::new(),
            arrow: None,
            body: Block {
                statements: Vec::new(),
                tail: Some(Box::new(expr(ExprKind::Call {
                    callee: Box::new(expr(ExprKind::Ident("MyView".to_string()))),
                    type_arguments: Vec::new(),
                    args: Vec::new(),
                }))),
                span: Span::dummy(),
                close_brace: Span::dummy(),
            },
        });

        // Inferring the closure should not leak the outer compose context -> emits E4008!
        infer(&mut checker, &closure);
        assert!(checker.errors.iter().any(|e| e.code() == "E4008"));
    }

    #[test]
    fn test_duplicate_struct_fields_and_enum_variants_rejected() {
        let mut checker = TypeChecker::new(None, "<test>");
        let source_file = SourceFile {
            package: None,
            imports: Vec::new(),
            declarations: vec![
                Declaration::Struct(prism_ast::StructDecl {
                    visibility: Visibility::Public,
                    name: "DupPoint".to_string(),
                    name_span: Span::dummy(),
                    type_parameters: Vec::new(),
                    marker: None,
                    intrinsic: None,
                    fields: vec![
                        prism_ast::StructField {
                            name: "x".to_string(),
                            is_var: false,
                            ty: prism_ast::Type::named("Int", Span::dummy()),
                            span: Span::dummy(),
                        },
                        prism_ast::StructField {
                            name: "x".to_string(),
                            is_var: false,
                            ty: prism_ast::Type::named("Int", Span::dummy()),
                            span: Span::dummy(),
                        },
                    ],
                    span: Span::dummy(),
                }),
                Declaration::Enum(prism_ast::EnumDecl {
                    visibility: Visibility::Public,
                    name: "DupEnum".to_string(),
                    name_span: Span::dummy(),
                    type_parameters: Vec::new(),
                    marker: None,
                    variants: vec![
                        prism_ast::EnumVariant {
                            name: "Val".to_string(),
                            fields: Vec::new(),
                            span: Span::dummy(),
                        },
                        prism_ast::EnumVariant {
                            name: "Val".to_string(),
                            fields: Vec::new(),
                            span: Span::dummy(),
                        },
                    ],
                    span: Span::dummy(),
                }),
            ],
            span: Span::dummy(),
        };

        checker.register_module_declarations(&source_file);
        assert_eq!(
            checker
                .errors
                .iter()
                .filter(|e| e.code() == "E4010")
                .count(),
            3
        );
    }

    #[test]
    fn test_indexing_indirection_peeling_and_heap_rejection() {
        let mut checker = TypeChecker::new(None, "<test>");
        // Borrowed string: &String
        let ref_str_ty = Type::Reference {
            mutable: false,
            inner: Box::new(Type::String),
        };
        checker
            .env
            .define(Symbol::variable(
                "s",
                ref_str_ty,
                Mutability::Val,
                Span::dummy(),
            ))
            .unwrap();

        let index_expr = expr(ExprKind::Index {
            target: Box::new(expr(ExprKind::Ident("s".to_string()))),
            index: Box::new(expr(ExprKind::Literal(Literal::Int(0)))),
        });
        assert_eq!(infer(&mut checker, &index_expr), Type::Char);
        assert!(checker.errors.is_empty());

        // Non-collection Heap: Heap<Int> -> cannot index -> E4002
        let heap_int_ty = Type::Heap(Box::new(Type::Int));
        checker
            .env
            .define(Symbol::variable(
                "h",
                heap_int_ty,
                Mutability::Val,
                Span::dummy(),
            ))
            .unwrap();
        let bad_index = expr(ExprKind::Index {
            target: Box::new(expr(ExprKind::Ident("h".to_string()))),
            index: Box::new(expr(ExprKind::Literal(Literal::Int(0)))),
        });
        infer(&mut checker, &bad_index);
        assert!(checker.errors.iter().any(|e| e.code() == "E4002"));
    }

    /// Parses and fully checks a Prism source file, returning the accumulated errors.
    fn check_source(src: &str) -> Vec<TypeError> {
        let file = prism_parser::parse_source(src).expect("source must parse");
        let mut checker =
            TypeChecker::new(file.package.as_ref().map(|p| p.package_string()), "<test>");
        checker.check_module(&file);
        checker.errors
    }

    fn error_codes(errors: &[TypeError]) -> Vec<&'static str> {
        errors.iter().map(|e| e.code()).collect()
    }

    const USER_STRUCT: &str = "struct User {\n    val id: Int\n    var name: String\n}\n";

    #[test]
    fn test_field_access_on_annotated_param_by_value() {
        let src = format!(
            "{}fun byValue(user: User): String = user.name.clone()\n",
            USER_STRUCT
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn test_field_access_through_shared_reference() {
        let src = format!(
            "{}fun byRef(user: &User): String = user.name.clone()\nfun idOf(user: &User): Int = user.id\n",
            USER_STRUCT
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn test_field_assignment_through_mutable_reference() {
        let ok = format!(
            "{}fun byMut(user: &mut User, n: String): Unit {{\n    user.name = n\n}}\n",
            USER_STRUCT
        );
        assert!(check_source(&ok).is_empty());

        // Assigning to a `val` field through `&mut` is still an immutability violation.
        let val_field = format!(
            "{}fun setId(user: &mut User): Unit {{\n    user.id = 1\n}}\n",
            USER_STRUCT
        );
        assert_eq!(error_codes(&check_source(&val_field)), vec!["E4003"]);

        // Mutable borrow of a `val` field resolves the struct and is rejected.
        let borrow_val = format!(
            "{}fun borrowId(user: &mut User): Unit {{\n    val r = &mut user.id\n}}\n",
            USER_STRUCT
        );
        assert_eq!(error_codes(&check_source(&borrow_val)), vec!["E4003"]);

        // Assigning a mismatched value type to a resolved field is rejected.
        let mismatch = format!(
            "{}fun setName(user: &mut User): Unit {{\n    user.name = 42\n}}\n",
            USER_STRUCT
        );
        assert_eq!(error_codes(&check_source(&mismatch)), vec!["E4002"]);
    }

    #[test]
    fn test_safe_navigation_on_annotated_nullable_struct() {
        let ok = format!(
            "{}fun nameOf(user: User?): String? = user?.name?.clone()\n",
            USER_STRUCT
        );
        assert!(check_source(&ok).is_empty());

        // `user?.name` is `String?`, so it is not assignable to the `String` return type.
        let not_null = format!(
            "{}fun nameOf(user: User?): String = user?.name?.clone()\n",
            USER_STRUCT
        );
        assert_eq!(error_codes(&check_source(&not_null)), vec!["E4005"]);
    }

    #[test]
    fn test_field_access_on_annotated_local_val() {
        let src = "struct Point {\n    val x: Int\n    val y: Int\n}\nfun main(): Int {\n    val p: Point = Point(1, 2)\n    return p.x + p.y\n}\n";
        assert!(check_source(src).is_empty());
    }

    #[test]
    fn test_chained_field_access_through_struct_typed_field() {
        // `Order` references `User` before it is declared (forward reference in Pass 1).
        let src = format!(
            "struct Order {{\n    val user: User\n}}\n{}fun buyer(order: &Order): String = order.user.name.clone()\n",
            USER_STRUCT
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn test_enum_typed_param_when_exhaustiveness() {
        let enum_decl = "enum Status {\n    Active,\n    Inactive\n}\n";
        let exhaustive = format!(
            "{}fun describe(s: Status): Int = when (s) {{\n    Active -> 1,\n    Inactive -> 0\n}}\n",
            enum_decl
        );
        assert!(check_source(&exhaustive).is_empty());

        let partial = format!(
            "{}fun describe(s: Status): Int = when (s) {{\n    Active -> 1\n}}\n",
            enum_decl
        );
        assert_eq!(error_codes(&check_source(&partial)), vec!["E4009"]);
    }

    #[test]
    fn test_unknown_nominal_type_stays_opaque() {
        // The annotation itself is accepted; member access on the opaque type is E4006.
        assert!(check_source("fun id(x: Missing): Missing = x\n").is_empty());
        assert_eq!(
            error_codes(&check_source("fun size(x: Missing): Int = x.size\n")),
            vec!["E4006"]
        );

        // Unknown fields on a resolved struct are still rejected.
        let src = format!("{}fun bad(user: User): Int = user.age\n", USER_STRUCT);
        assert_eq!(error_codes(&check_source(&src)), vec!["E4006"]);
    }

    #[test]
    fn test_field_access_on_struct_imported_from_abi() {
        let models = prism_parser::parse_source(
            "package demo.models\npublic struct Address {\n    val city: String\n}\npublic struct User {\n    var name: String\n    val email: String?\n    val home: Address\n}\n",
        )
        .expect("models must parse");
        let abi = FileAbi::extract(&models, "demo/models/models", &prism_abi::no_imports)
            .expect("models ABI must extract");

        let app = prism_parser::parse_source(
            "package demo.app\nfun nameOf(user: &User): String = user.name.clone()\nfun rename(user: &mut User, n: String): Unit {\n    user.name = n\n}\nfun emailOf(user: &User): String? = user.email.clone()\nfun cityOf(user: User): String = user.home.city.clone()\n",
        )
        .expect("app must parse");
        let mut checker = TypeChecker::new(Some("demo.app".to_string()), "<test>");
        checker.ingest_file_abi(&abi);
        checker.check_module(&app);
        assert!(checker.errors.is_empty(), "{:?}", checker.errors);
    }

    #[test]
    fn test_exported_function_missing_return_type_e4011() {
        for src in [
            "public fun greet(name: String) {\n}\n",
            "package fun helper(x: Int) {\n}\n",
        ] {
            assert_eq!(error_codes(&check_source(src)), vec!["E4011"], "{}", src);
        }
        // Private functions may still omit the return type.
        assert!(check_source("fun local(x: Int) {\n}\n").is_empty());
    }

    /// A checker that has registered `src`, for querying the `Copy` and `Clone` predicates.
    fn registered(src: &str) -> TypeChecker {
        let file = prism_parser::parse_source(src).expect("source must parse");
        let mut checker = TypeChecker::new(None, "<test>");
        checker.check_module(&file);
        checker
    }

    #[test]
    fn copy_and_clone_tables_cover_every_kind_of_type() {
        let checker = registered(
            "struct P : Copy { val x: Int }\nstruct C : Clone { val s: String }\nstruct M { val x: Int }\nenum E : Copy { A }\n",
        );
        let nominal = |name: &str| Type::plain_struct(NominalId::new("<test>", name));
        let reference = |mutable, inner: Type| Type::Reference {
            mutable,
            inner: Box::new(inner),
        };
        let nullable = |inner: Type| Type::Nullable(Box::new(inner));
        let heap = |inner: Type| Type::Heap(Box::new(inner));
        let function = Type::function(false, Vec::new(), Type::Unit);
        // (type, is Copy, is Clone)
        let table = [
            (Type::Int, true, true),
            (Type::Float, true, true),
            (Type::Bool, true, true),
            (Type::Char, true, true),
            (Type::Unit, true, true),
            (Type::Nothing, true, true),
            (Type::plain_struct(NominalId::builtin_range()), true, true),
            (reference(false, Type::String), true, true),
            (reference(true, Type::Int), false, false),
            (Type::String, false, true),
            (nullable(Type::Int), true, true),
            (nullable(Type::String), false, true),
            (nullable(function.clone()), false, false),
            (heap(Type::Int), false, true),
            (heap(function.clone()), false, false),
            (function, false, false),
            (
                Type::Parameter(prism_typed_tree::TypeParameter {
                    index: 0,
                    name: "T".into(),
                    bound: None,
                }),
                false,
                false,
            ),
            (Type::Opaque("UInt64".into()), false, false),
            (Type::Error, true, true),
            (nominal("P"), true, true),
            (nominal("C"), false, true),
            (nominal("M"), false, false),
            (Type::plain_enum(NominalId::new("<test>", "E")), true, true),
        ];
        for (ty, copy, clone) in table {
            assert_eq!(checker.is_copy(&ty), copy, "is_copy({ty})");
            assert_eq!(checker.is_clone(&ty), clone, "is_clone({ty})");
        }
    }

    #[test]
    fn marker_of_a_type_known_only_through_a_reference() {
        // `Money` is defined in `shop/models`, which this file does not import; only the field
        // type recorded in `shop/api`'s ABI says that it is `Copy`.
        let models = prism_parser::parse_source(
            "package shop\npublic struct Money : Copy { val cents: Int }\npublic struct Note { val text: String }\n",
        )
        .unwrap();
        let exported = prism_abi::declared_types(&models, true);
        let bind = |name: &str| {
            exported
                .get(name)
                .map(|header| (header.clone(), "shop/models".to_string()))
        };
        let api = prism_parser::parse_source(
            "package shop\nimport models\npublic struct Order { val price: Money, val note: Note }\n",
        )
        .unwrap();
        let mut checker = TypeChecker::new(None, "app");
        checker.ingest_file_abi(&FileAbi::extract(&api, "shop/api", &bind).unwrap());
        let money = Type::plain_struct(NominalId::new("shop/models", "Money"));
        let note = Type::plain_struct(NominalId::new("shop/models", "Note"));
        assert!(checker
            .type_def(&NominalId::new("shop/models", "Money"))
            .is_none());
        assert!(checker.is_copy(&money));
        assert!(!checker.is_copy(&note));
        assert!(!checker.is_clone(&note));
    }

    #[test]
    fn marker_check_reports_each_offending_field_once() {
        let errors = check_source(
            "struct User : Copy { val id: Int, val name: String, val tags: Heap<Int> }\nenum Event : Copy { Click(Int, Int), Text(String) }\n",
        );
        let messages: Vec<String> = errors.iter().map(TypeError::message).collect();
        assert_eq!(error_codes(&errors), vec!["E4016"; 3], "{messages:?}");
        assert!(messages[0].contains("field 'name'") && messages[0].contains("'String'"));
        assert!(messages[1].contains("field 'tags'"));
        assert!(messages[2].contains("payload 1 of variant 'Text'"));
    }

    #[test]
    fn generic_declarations_record_their_type_parameters_and_marker_parameters() {
        let file = prism_parser::parse_source(
            "struct Pair<A, B> : Copy { val a: A, val b: Int }\nenum Option<T> { Some(T), None }\nfun <T : Clone> keep(x: T): T = x\nfun f(p: Pair<Int, String>): Int = p.b\n",
        )
        .expect("source must parse");
        let mut checker = TypeChecker::new(None, "main");
        let typed = checker.check_module(&file);
        assert!(checker.errors.is_empty(), "{:?}", checker.errors);
        let pair = NominalId::new("main", "Pair");
        let names: Vec<&str> = checker
            .types
            .parameters(&pair)
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, vec!["A", "B"]);
        // Only `A` occurs in a field, so only `A` conditions the marker.
        assert_eq!(checker.types.marker_parameters(&pair), &[0]);
        let fields = checker.types.struct_fields(&pair).unwrap();
        assert!(matches!(&fields[0].2, Type::Parameter(p) if p.name == "A" && p.index == 0));
        let option = NominalId::new("main", "Option");
        assert_eq!(checker.types.parameters(&option).len(), 1);
        let keep = checker.env.lookup("keep").expect("keep is declared");
        let bounds: Vec<Option<TypeMarker>> =
            keep.type_parameters().iter().map(|p| p.bound).collect();
        assert_eq!(bounds, vec![Some(TypeMarker::Clone)]);
        let item = typed.items.iter().find_map(|item| match item {
            Item::Function(f) if f.path.name == "keep" => Some(f),
            _ => None,
        });
        assert_eq!(item.expect("keep is an item").type_parameters.len(), 1);
    }

    #[test]
    fn annotations_of_generic_types_keep_their_arguments_and_bind_parameters_first() {
        let file = prism_parser::parse_source(
            "struct T { val n: Int }\nstruct Box<T>(val v: T)\nfun <T> f(b: Box<T>, t: T) {}\n",
        )
        .expect("source must parse");
        let mut checker = TypeChecker::new(None, "main");
        checker.check_module(&file);
        assert!(checker.errors.is_empty(), "{:?}", checker.errors);
        let Some(Type::Fn { params, .. }) = checker.env.lookup("f").map(|s| s.ty()) else {
            panic!("f is a function");
        };
        let parameter = Type::Parameter(TypeParameter {
            index: 0,
            name: "T".to_string(),
            bound: None,
        });
        assert_eq!(
            params,
            vec![
                Type::Struct {
                    id: NominalId::new("main", "Box"),
                    arguments: vec![parameter.clone()],
                },
                parameter,
            ]
        );
        assert_eq!(
            error_codes(&check_source("struct Box<T>(val v: T)\nfun f(b: Box) {}\nfun g(b: Box<&Int>) {}\nfun <T> h(x: T<Int>) {}\n")),
            vec!["E4020", "E4022", "E4020"]
        );
    }
}
