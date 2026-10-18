//! Main TypeChecker implementation: Pass 1 declaration registration and Pass 2 body checking.
//!
//! Pass 2 builds the file's typed tree while it checks (D1 of `typed-tree`): every `infer_*` and
//! `check_*` function builds the node of the expression it checks and returns its id, and the
//! point that decides something (a coercion, a move, a borrow, a receiver pass) records it.

use crate::builder::{BodyBuilder, FileBuilder};
use crate::error::TypeError;
use crate::inference::Unknowns;
use crate::scope::{is_type_symbol, ImportLookup, Mutability, ScopeStack, Symbol, SymbolKind};
use crate::types::{
    type_from_abi, type_parameters_from_abi, NominalId, Type, TypeDef, TypeParameter,
    TypeRelations, BUILTIN_MODULE,
};
use prism_abi::{AbiType, AbiTypeMembers, ExportedSymbol, FileAbi};
use prism_ast::decl::{Declaration, FunctionBody, FunctionDecl, ReceiverMode, TopLevelVarDecl};
use prism_ast::expr::{
    BinaryOp, CallArg, Expr, ExprKind, Literal, UnaryOp, WhenBranch, WhenPattern,
};
use prism_ast::module::SourceFile;
use prism_ast::statement::{Block, Statement, StatementKind};
use prism_ast::{Span, TypeMarker, Visibility};
use prism_typed_tree::{
    Argument, BinaryOperator, BorrowOrigin, BuiltinFunction, Call, Callee, CoercionKind,
    ConstructField, ConstructKind, DeclarationPath, EnumItem, ExpressionId, ExpressionKind,
    FunctionItem, Item, Local, LocalKind, LogicalOperator, Pattern, SafeNavigationOrigin,
    StatementId, StatementKind as TreeStatementKind, StructItem, TypeTable, TypedFile,
    UnaryOperator, ValueItem,
};
use std::collections::{HashMap, HashSet};

/// The help of a type argument that cannot be inferred (`E4013`, D8 of `generics`).
const TYPE_ARGUMENTS_HELP: &str =
    "write the type arguments, as in 'name<...>(...)', or annotate the expected type";

/// The name the receiver of an extension function is bound to in its body. It is a keyword, so no
/// user declaration can use it.
const THIS: &str = "this";

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
            .chain(params.iter().map(|(_, ty, _)| ty))
            .chain(std::iter::once(return_type))
            .collect(),
        ExportedSymbol::Struct { fields, .. } => fields.iter().map(|(_, _, ty)| ty).collect(),
        ExportedSymbol::Enum { variants, .. } => variants.iter().flat_map(|(_, p)| p).collect(),
        ExportedSymbol::Var { ty, .. } => vec![ty],
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
    /// A `Copy` value read out of `layers` references, inside a `?` when `through_nullable`
    /// (`(&Int)?` used as an `Int?`).
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

/// The outcome of [`TypeChecker::coerce`].
#[derive(Debug, Clone, PartialEq)]
enum Coercion {
    /// The value fits after the adjustment.
    Fits(Adjustment),
    /// The value fits only by moving `ty` out of a borrow, which is not `Copy` (`E4017`).
    /// `nullable` is true when the borrow is inside a `?`.
    MovesOutOfBorrow { ty: Type, nullable: bool },
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
/// exported by a direct import. Extensions live in their own tables, keyed by receiver type and
/// name, apart from the name scopes, so `fun greet()` and `fun User.greet()` coexist.
#[derive(Debug, Clone)]
struct Extension {
    mode: ReceiverMode,
    /// The declared receiver type, which may mention the extension's type parameters
    /// (`Box<T>` in `fun <T> Box<T>.get()`).
    receiver: Type,
    /// The defining module.
    module: String,
    /// The function's signature, parameter names, defaults and visibility. Its name is the
    /// method's name.
    symbol: Symbol,
}

/// What extensions are keyed by: the type constructor of their receiver, without type arguments
/// (D10 of `generics`), so `fun Box<Int>.sum()` and `fun <T> Box<T>.sum()` share a key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ReceiverKey {
    /// A struct or enum, by identity.
    Nominal(NominalId),
    /// Any other receiver type, as it is.
    Other(Type),
}

impl ReceiverKey {
    fn of(ty: &Type) -> Self {
        match ty.nominal_id() {
            Some(id) => ReceiverKey::Nominal(id.clone()),
            None => ReceiverKey::Other(ty.clone()),
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
    /// `consuming fun` through references: a `Copy` receiver is copied out.
    CopyOut,
}

/// A layer looked through to reach an extension's receiver type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Reference { mutable: bool },
    Heap,
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
    /// An extension function, by receiver type constructor and name.
    Extension { receiver: ReceiverKey, name: String },
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
    locals: Vec<HashMap<String, Symbol>>,
    in_compose_stack: Vec<bool>,
    current_expected_return: Option<Type>,
    in_function: bool,
    origin: Origin,
    consuming: bool,
    /// The tree of the body being checked, with its open closures and loops.
    builder: BodyBuilder,
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
    own_extensions: HashMap<(ReceiverKey, String), Extension>,
    /// Extension functions exported by direct imports, by receiver type and name, with one
    /// candidate per exporting module.
    imported_extensions: HashMap<(ReceiverKey, String), Vec<(String, Extension)>>,
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
    /// Expected return type of the currently enclosing function.
    pub current_expected_return: Option<Type>,
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
            imported_extensions: HashMap::new(),
            direct_imports: HashSet::new(),
            seen_nominals: HashMap::new(),
            exported_nominals: HashSet::new(),
            default_inputs: None,
            type_parameter_scope: Vec::new(),
            generic_callee: false,
            errors: Vec::new(),
            in_compose_stack: vec![false],
            current_expected_return: None,
            unannotated_private_fns: Vec::new(),
            in_function: false,
            builder: BodyBuilder::new(DeclarationPath::new("", "<none>")),
            file: FileBuilder::default(),
            variant_owners: HashMap::new(),
            ambiguous_variants: HashSet::new(),
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
                let kind = ExpressionKind::Coerce {
                    value: id,
                    kind: CoercionKind::Subtype,
                };
                self.add(kind, expected.clone(), span)
            }
        }
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
                params,
                return_type,
            } => Type::Fn {
                is_compose: *is_compose,
                params: params.iter().map(|p| self.lower_type(p)).collect(),
                ret: Box::new(self.lower_type(return_type)),
            },
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
            "Int", "Float", "Double", "Bool", "Char", "String", "Unit", "Nothing", "Heap",
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
            Err(TypeError::UnresolvedSymbol { .. }) if output_builtin_named(name).is_some() => {
                // An output built-in can only be called (D15 of `codegen-core`).
                self.errors.push(TypeError::BuiltinFunctionAsValue {
                    name: name.to_string(),
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
            AbiType::Nullable(inner) | AbiType::Heap(inner) | AbiType::Ref { inner, .. } => {
                self.record_nominals(inner)
            }
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
                params,
                return_type,
                ..
            } => {
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
        let payload_free = self
            .enum_variants(&id)?
            .iter()
            .any(|(name, payloads)| name == &member && payloads.is_empty());
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
        let index = self
            .enum_variants(&enum_id)
            .and_then(|variants| variants.iter().position(|(name, _)| name == member));
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
            let id = NominalId::new(definition.module.clone(), definition.name.clone());
            self.types.insert_marker(
                id.clone(),
                definition.marker,
                definition.marker_parameters.clone(),
            );
            let parameters = type_parameters_from_abi(&definition.type_parameters);
            let convert = |ty: &AbiType| type_from_abi(ty, &parameters);
            let type_definition = match &definition.members {
                AbiTypeMembers::Fields(fields) => TypeDef::Struct {
                    parameters: parameters.clone(),
                    fields: fields
                        .iter()
                        .map(|(name, is_var, ty)| (name.clone(), *is_var, convert(ty)))
                        .collect(),
                },
                AbiTypeMembers::Variants(variants) => TypeDef::Enum {
                    parameters: parameters.clone(),
                    variants: variants
                        .iter()
                        .map(|(name, payloads)| {
                            (name.clone(), payloads.iter().map(convert).collect())
                        })
                        .collect(),
                },
            };
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
                    is_compose,
                    receiver,
                    name,
                    type_parameters,
                    params,
                    return_type,
                    ..
                } => {
                    let type_parameters = type_parameters_from_abi(type_parameters);
                    let param_types = params
                        .iter()
                        .map(|(_, ty, _)| type_from_abi(ty, &type_parameters))
                        .collect();
                    let param_names = params.iter().map(|(n, _, _)| n.clone()).collect();
                    let has_default = params.iter().map(|(_, _, d)| d.is_some()).collect();
                    let fn_ty = Type::Fn {
                        is_compose: *is_compose,
                        params: param_types,
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
                    match receiver {
                        // An extension is reached through its receiver, never by its bare name.
                        Some(receiver) => {
                            let receiver_ty = type_from_abi(&receiver.ty, &type_parameters);
                            let key = (ReceiverKey::of(&receiver_ty), name.clone());
                            let candidates = self.imported_extensions.entry(key).or_default();
                            if !candidates.iter().any(|(m, _)| m == &abi.module) {
                                let extension = Extension {
                                    mode: receiver.mode,
                                    receiver: receiver_ty,
                                    module: abi.module.clone(),
                                    symbol,
                                };
                                candidates.push((abi.module.clone(), extension));
                            }
                        }
                        None => self.env.define_import(&abi.module, symbol),
                    }
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
                                Type::Fn {
                                    is_compose: false,
                                    params: f_types.clone(),
                                    ret: Box::new(enum_ty.clone()),
                                },
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
                                Type::Fn {
                                    is_compose: false,
                                    params: f_types.clone(),
                                    ret: Box::new(enum_ty.clone()),
                                },
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

        for decl in &source_file.declarations {
            if let Declaration::Function(f) = decl {
                if f.receiver.is_some() {
                    self.register_extension_decl(f);
                }
            }
        }
    }

    /// Defines a top-level declaration in module scope. A declaration with the same name as an
    /// imported symbol shadows the import: lookups check module scope before the import layer.
    fn define_top_level(&mut self, symbol: Symbol) {
        if let Err(err) = self.env.define(symbol) {
            self.errors.push(err);
        }
    }

    fn register_function_decl(&mut self, f: &FunctionDecl) {
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
        for annot in f.params.iter().map(|p| &p.ty).chain(&f.return_type) {
            self.check_annotation_names(annot);
        }
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
            params,
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
    }

    /// Registers the extension function `f` in the file's extension table (D7 of
    /// `extension-methods`). Its receiver type follows the rules of annotations. Two extensions
    /// with the same receiver and name, an extension named like a field of its receiver struct,
    /// an extension named `clone` on a `Clone` type, and one named like a built-in member
    /// (`toString` on `Int`, `Double`, `Bool` or `Char`, `length` on `String`) are rejected with
    /// `E4010`: all but the first could never be called, because members take precedence.
    fn register_extension_decl(&mut self, f: &FunctionDecl) {
        let receiver = f.receiver.as_ref().expect("an extension has a receiver");
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
        let builtin_clash = (f.name == TO_STRING && has_builtin_to_string(&receiver_ty))
            || (f.name == STRING_LENGTH && receiver_ty == Type::String);
        if self.own_extensions.contains_key(&key) {
            // Extensions are keyed by the receiver's type constructor (D10 of `generics`), so
            // `Box<Int>.size` and `Box<T>.size` are one name.
            let constructor = match &key.0 {
                ReceiverKey::Nominal(id) => id.name.clone(),
                ReceiverKey::Other(ty) => ty.to_string(),
            };
            self.errors.push(TypeError::DuplicateSymbol {
                name: format!("{}.{}", constructor, f.name),
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
        } else if builtin_clash {
            self.errors.push(TypeError::UncallableExtension {
                message: format!(
                    "extension '{}.{}' can never be called: the built-in member '{}' of '{}' always takes precedence",
                    receiver_ty, f.name, f.name, receiver_ty
                ),
                span: f.span,
            });
        }
        let extension = Extension {
            mode: receiver.mode,
            receiver: receiver_ty,
            module: self.module_path.clone(),
            symbol,
        };
        self.own_extensions.insert(key, extension);
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
        self.finish_file(source_file)
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
            }
        }
        TypedFile {
            module: self.module_path.clone(),
            package: self.current_package.clone(),
            items,
            bodies: file.bodies,
            types: self.types.clone(),
        }
    }

    /// Starts building the body of the declaration `owner`, returning the builder of the body
    /// being built when it started, which [`Self::finish_body`] restores.
    fn start_body(&mut self, owner: DeclarationPath) -> BodyBuilder {
        std::mem::replace(&mut self.builder, BodyBuilder::new(owner))
    }

    /// Finishes the body being built, restoring `previous`, and records it with `item`, which
    /// gets the body's id.
    fn finish_body(
        &mut self,
        previous: BodyBuilder,
        span: Span,
        item: impl FnOnce(prism_typed_tree::BodyId) -> Item,
    ) {
        let built = std::mem::replace(&mut self.builder, previous);
        let body = self.file.bodies.alloc(built.finish());
        self.file.items.entry(span).or_insert_with(|| item(body));
    }

    /// Checks a top-level variable's initializer. An unannotated private variable takes the
    /// initializer's type.
    fn check_top_level_var(&mut self, v: &TopLevelVarDecl) {
        let previous = self.start_body(DeclarationPath::new(self.module_path.clone(), &v.name));
        let initializer = self.with_consuming(true, |c| c.infer_expr(&v.initializer));
        let inferred = self.ty(initializer);
        let (ty, value) = if let Some(annot) = &v.ty {
            let expected = self.lower_type(annot);
            let value = if !inferred.is_assignable_to(&expected) {
                self.errors.push(TypeError::TypeMismatch {
                    expected: expected.clone(),
                    found: inferred.clone(),
                    span: v.span,
                    help: None,
                });
                self.wrap_error(initializer)
            } else {
                self.coerce_to(initializer, &expected)
            };
            (expected, value)
        } else {
            if v.visibility == Visibility::Private {
                self.env.update_type(&v.name, inferred.clone());
            }
            (inferred, initializer)
        };
        self.builder.body_mut().value = Some(value);
        let path = DeclarationPath::new(self.module_path.clone(), &v.name);
        self.finish_body(previous, v.span, |body| {
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
                    InferenceKey::Extension { receiver, name } => match receiver {
                        ReceiverKey::Nominal(id) => format!("{}.{}", id.name, name),
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
        let path = match &receiver_ty {
            Some(receiver_ty) => {
                DeclarationPath::extension(self.module_path.clone(), receiver_ty.clone(), &f.name)
            }
            None => DeclarationPath::new(self.module_path.clone(), &f.name),
        };
        let previous = self.start_body(path.clone());
        let prev_in_func = self.in_function;
        self.in_function = true;
        self.env.enter_scope();
        self.push_compose_context(f.is_compose);

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
            self.define_local(sym);
        }

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
            let default = p.default_value.as_ref().map(|default| {
                self.with_consuming(true, |c| c.check_argument(default, &p_ty, Origin::Value))
            });
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
                            params: f.params.iter().map(|p| self.lower_type(&p.ty)).collect(),
                            ret: Box::new(expr_ty),
                        };
                        match &receiver_ty {
                            Some(receiver_ty) => {
                                let key = (ReceiverKey::of(receiver_ty), f.name.clone());
                                if let Some(extension) = self.own_extensions.get_mut(&key) {
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
        let receiver = f
            .receiver
            .as_ref()
            .zip(receiver_ty)
            .map(|(receiver, ty)| (receiver.mode, ty));
        let type_parameters = self.type_parameter_scope.clone();
        self.finish_body(previous, f.span, |body| {
            Item::Function(FunctionItem {
                path,
                visibility: f.visibility,
                is_compose: f.is_compose,
                type_parameters,
                receiver,
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

    /// Infers an operator operand or a condition. Operators read through references (D1 of
    /// `typechecker-soundness`), so every `&` and `&mut` layer is peeled: with `a: &Int`,
    /// `a + 1` is an `Int`. `Heap<T>` is not peeled. Each peeled layer is an explicit
    /// dereference, and the operand is read in place (D5 of `typed-tree`).
    fn infer_operand(&mut self, expr: &Expr) -> ExpressionId {
        let id = self.with_origin(Origin::Value, |c| c.infer_expr(expr));
        self.peel_reference_nodes(id)
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

        // Special handling for closures checked against expected function type
        if let ExprKind::Closure { params, body } = &expr.kind {
            if let Type::Fn {
                is_compose,
                params: exp_params,
                ret: exp_ret,
            } = expected
            {
                return self.check_closure(
                    params,
                    body,
                    *is_compose,
                    exp_params,
                    Some(exp_ret),
                    expr.span,
                );
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
    /// `exp_params` and, when known, return type `exp_ret`; without one the body's type is the
    /// closure's return type.
    fn check_closure(
        &mut self,
        params: &[prism_ast::ClosureParameter],
        body: &Block,
        is_compose: bool,
        exp_params: &[Type],
        exp_ret: Option<&Type>,
        span: Span,
    ) -> ExpressionId {
        if params.len() != exp_params.len() {
            self.errors.push(TypeError::CallSignatureMismatch {
                message: format!(
                    "closure expected {} parameters, but has {}",
                    exp_params.len(),
                    params.len()
                ),
                span,
            });
            return self.error_node(Vec::new(), span);
        }

        let prev = self.enter_closure(is_compose, exp_ret.cloned());
        let mut parameters = Vec::new();
        for (parameter, param_ty) in params.iter().zip(exp_params.iter()) {
            let sym = self.bind_local(
                &parameter.name,
                param_ty.clone(),
                Mutability::Val,
                LocalKind::ClosureParameter,
                parameter.span,
            );
            parameters.push(Self::local_of(&sym).expect("a bound local"));
            self.define_local(sym);
        }

        // The body's value is what the closure returns.
        let body_id = self.with_origin(Origin::Return, |c| {
            c.with_consuming(true, |c| c.check_block(body, exp_ret))
        });
        let captures = self.exit_closure(prev);
        let ty = Type::Fn {
            is_compose,
            params: exp_params.to_vec(),
            ret: Box::new(self.ty(body_id)),
        };
        let kind = ExpressionKind::Closure {
            parameters,
            body: body_id,
            is_compose,
            captures,
        };
        self.add(kind, ty, span)
    }

    /// Enters the body of a closure: a scope, a compose context, a fresh set of open loops (a
    /// jump cannot leave the closure for a loop around it) and the expected return type. The
    /// builder records the closure's captures (D11 of `typed-tree`). Returns what
    /// [`Self::exit_closure`] restores.
    fn enter_closure(
        &mut self,
        is_compose: bool,
        expected_return: Option<Type>,
    ) -> (bool, Vec<StatementId>, Option<Type>) {
        self.env.enter_scope();
        self.push_compose_context(is_compose);
        let prev_in_func = std::mem::replace(&mut self.in_function, true);
        let prev_loops = std::mem::take(&mut self.builder.loops);
        let prev_ret = std::mem::replace(&mut self.current_expected_return, expected_return);
        self.builder.open_closure();
        (prev_in_func, prev_loops, prev_ret)
    }

    /// Leaves a closure body entered by [`Self::enter_closure`], returning its captures.
    fn exit_closure(
        &mut self,
        (prev_in_func, prev_loops, prev_ret): (bool, Vec<StatementId>, Option<Type>),
    ) -> Vec<prism_typed_tree::Capture> {
        let captures = self.builder.close_closure();
        self.builder.loops = prev_loops;
        self.current_expected_return = prev_ret;
        self.in_function = prev_in_func;
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

            ExprKind::Index { target, index } => {
                let index_id = self.check_expr(index, &Type::Int);
                // A string's character is computed, not stored: `s[i]` is a call of the
                // built-in `get` with a shared receiver borrow of the string, read in place
                // when it is a place and consumed into a temporary otherwise, as the receiver of
                // `clone()` is (D5 of `runtime-strings-and-drops`).
                let target_id = if self.is_place(target) {
                    self.infer_place(target).0
                } else {
                    self.with_consuming(true, |c| c.infer_expr(target))
                };
                let base = self.peel_indirection_nodes(target_id);
                match self.ty(base) {
                    Type::String => self.builtin_member_call(
                        BuiltinFunction::StringCharacter,
                        base,
                        vec![index_id],
                        expr.span,
                    ),
                    Type::Error => self.error_node(vec![index_id, base], expr.span),
                    other => {
                        self.errors.push(TypeError::TypeMismatch {
                            expected: Type::String,
                            found: other,
                            span: expr.span,
                            help: None,
                        });
                        self.error_node(vec![index_id, base], expr.span)
                    }
                }
            }

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

            ExprKind::Closure { params, body } => {
                let prev = self.enter_closure(false, None);

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

                let ty = Type::Fn {
                    is_compose: false,
                    params: param_types,
                    ret: Box::new(self.ty(body_id)),
                };
                let kind = ExpressionKind::Closure {
                    parameters,
                    body: body_id,
                    is_compose: false,
                    captures,
                };
                self.add(kind, ty, expr.span)
            }
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
                } else {
                    // Strict typing: no implicit widening (e.g. Int + Float rejected)
                    let help = type_parameter_operator_help(&left_ty, &right_ty);
                    self.errors.push(TypeError::TypeMismatch {
                        expected: left_ty,
                        found: right_ty,
                        span,
                        help,
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
                    let help = type_parameter_operator_help(&left_ty, &right_ty);
                    self.errors.push(TypeError::TypeMismatch {
                        expected: left_ty,
                        found: right_ty,
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
                    let help = type_parameter_operator_help(&left_ty, &right_ty).or_else(|| {
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
                let node = self.with_origin(Origin::Value, |c| c.infer_expr(target));
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

        let value_id = self.with_consuming(true, |c| c.check_expr(value, &target_ty));
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
                    let help = type_parameter_operator_help(&target_ty, &value_ty)
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

    /// Types the referent written by `*reference = ...` (D5 of `reference-access`): `reference`
    /// is read in place and must be a reference, and every reference layer written through must
    /// be `&mut`. Returns the referent's place (a dereference), its type and whether the write is
    /// allowed; a refusal is reported once, at the first layer that refuses it.
    fn write_through(&mut self, reference: &Expr, span: Span) -> (ExpressionId, Type, bool) {
        let (node, ty, writable) = match &reference.kind {
            ExprKind::Unary {
                op: UnaryOp::Deref,
                expr: inner,
            } => self.write_through(inner, span),
            _ => {
                let id = self.infer_expr(reference);
                (id, self.ty(id), true)
            }
        };
        match ty {
            Type::Error => (node, Type::Error, false),
            Type::Reference { mutable, inner } => {
                if writable && !mutable {
                    self.errors.push(TypeError::ImmutabilityViolation {
                        message: format!(
                            "cannot write through '{}': it is a shared '&' borrow, which cannot be written through",
                            reference
                        ),
                        span,
                        help: Some(
                            "take a mutable borrow '&mut' where the reference is created"
                                .to_string(),
                        ),
                    });
                }
                let referent = self.deref(node);
                (referent, *inner, writable && mutable)
            }
            // A refusal at an inner layer was reported there.
            other if !writable => (self.wrap_error(node), other, false),
            other => {
                self.errors.push(TypeError::NotAReference {
                    ty: other,
                    span: reference.span,
                });
                (self.wrap_error(node), Type::Error, false)
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
                let range =
                    self.check_operand(iterable, &Type::plain_struct(NominalId::builtin_range()));
                // The loop variable sits in a scope level above the body, so the body may
                // shadow it, as a function body may shadow a parameter.
                self.env.enter_scope();
                let sym = self.bind_local(
                    variable,
                    Type::Int,
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
                        range,
                        body,
                    },
                );
                id
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
        // `Enum`; the subject is read in place through its dereferences (D9 of `typed-tree`).
        let subject = target.map(|t| {
            let id = self.infer_expr(t);
            let id = self.peel_reference_nodes(id);
            let ty = self.ty(id);
            // Matching on variants needs the enum's definition, which needs its module.
            if self.reject_unimported_members(&ty, t.span) {
                self.wrap_error(id)
            } else {
                id
            }
        });
        let subject_ty = subject.map(|id| self.ty(id));
        let subject_variants = match &subject_ty {
            Some(Type::Enum { id, .. }) => self
                .enum_variants(id)
                .map(|variants| (id.clone(), variants)),
            _ => None,
        };
        let mut has_else = false;
        let mut unified_ty: Option<Type> = None;
        let mut matched_variants = HashSet::new();
        // The literals a `Bool` or `Bool?` subject is matched against, which cover it like the
        // variants of an enum: `true`, `false` and, for `Bool?`, `null`.
        let mut matched_literals = HashSet::new();
        let mut tree_branches = Vec::new();

        for branch in branches {
            let mut patterns = Vec::new();
            for pattern in &branch.patterns {
                match pattern {
                    WhenPattern::Else => {
                        has_else = true;
                        patterns.push(Pattern::Else);
                    }
                    WhenPattern::Expr(p_expr) => {
                        let covered = subject_variants.as_ref().and_then(|(id, variants)| {
                            self.unit_variant_pattern(p_expr, id, variants)
                                .map(|name| (id.clone(), variants, name))
                        });
                        if let Some((enum_id, variants, v_name)) = covered {
                            let index = variants
                                .iter()
                                .position(|(name, _)| name == &v_name)
                                .expect("a covered variant is a variant");
                            patterns.push(Pattern::Variant {
                                enum_id,
                                index,
                                name: v_name.clone(),
                            });
                            matched_variants.insert(v_name);
                        } else if let Some(s_ty) = &subject_ty {
                            if let ExprKind::Literal(literal @ (Literal::Bool(_) | Literal::Null)) =
                                &p_expr.kind
                            {
                                matched_literals.insert(match literal {
                                    Literal::Bool(value) => Some(*value),
                                    _ => None,
                                });
                            }
                            patterns.push(Pattern::Value(self.check_expr(p_expr, s_ty)));
                        } else {
                            patterns
                                .push(Pattern::Condition(self.check_operand(p_expr, &Type::Bool)));
                        }
                    }
                    // Whether the end is included only matters at runtime.
                    WhenPattern::InRange {
                        start,
                        end,
                        end_inclusive,
                    } => {
                        if let Some(s_ty) = &subject_ty {
                            let start = self.check_expr(start, s_ty);
                            let end = self.check_expr(end, s_ty);
                            patterns.push(Pattern::Range {
                                start,
                                end,
                                end_inclusive: *end_inclusive,
                            });
                        } else {
                            // Without a subject, a pattern is a condition, which a range is
                            // not: its ends are checked as a range's, and the range rejected.
                            let start = self.check_operand(start, &Type::Int);
                            let end = self.check_operand(end, &Type::Int);
                            let span = self.builder.span_of(start).merge(self.builder.span_of(end));
                            self.report_mismatch(
                                &Type::Bool,
                                Type::plain_struct(NominalId::builtin_range()),
                                span,
                            );
                            let condition = self.error_node(vec![start, end], span);
                            patterns.push(Pattern::Condition(condition));
                        }
                    }
                }
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
            tree_branches.push(prism_typed_tree::WhenBranch {
                patterns,
                body,
                span: branch.span,
            });
        }

        self.origin = branch_origin;
        let covers_variants = subject_variants.as_ref().is_some_and(|(_, variants)| {
            !variants.is_empty()
                && variants
                    .iter()
                    .all(|(v_name, _)| matched_variants.contains(v_name))
        });
        // `true` and `false` cover a `Bool`, and with `null` a `Bool?`.
        let covers_booleans = match &subject_ty {
            Some(Type::Bool) => [Some(true), Some(false)]
                .iter()
                .all(|literal| matched_literals.contains(literal)),
            Some(Type::Nullable(inner)) if **inner == Type::Bool => [Some(true), Some(false), None]
                .iter()
                .all(|literal| matched_literals.contains(literal)),
            _ => false,
        };
        let covers_variants = covers_variants || covers_booleans;
        let build = |checker: &mut Self, branches, ty| {
            let kind = ExpressionKind::When {
                subject,
                branches,
                exhaustive: has_else || covers_variants,
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
        let is_exhaustive = has_else || subject_ty == Some(Type::Error) || covers_variants;

        if !is_exhaustive {
            self.errors.push(TypeError::NonExhaustiveWhen {
                message: "'when' expression in value position must have an 'else' branch or be exhaustive"
                    .to_string(),
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

    /// If `pattern` names a payload-free variant of the subject enum `enum_id`, returns that
    /// variant's name. Such a pattern covers the whole variant. A qualified path `Enum.Variant`
    /// counts only when `Enum` is the subject's enum, and a bare `Variant` counts unless a local
    /// binding shadows it. Values that merely equal a variant (`Shape.Circle(1)`, `h.Green`) never
    /// count.
    fn unit_variant_pattern(
        &self,
        pattern: &Expr,
        enum_id: &NominalId,
        variants: &[(String, Vec<Type>)],
    ) -> Option<String> {
        let v_name = match &pattern.kind {
            ExprKind::Ident(name) => {
                let shadowed =
                    self.env.lookup(name).is_some() && !self.env.resolves_at_module_level(name);
                if shadowed {
                    return None;
                }
                name
            }
            ExprKind::MemberAccess {
                target,
                member,
                is_safe_nav: false,
            } => match self.type_name_symbol(target).map(|sym| sym.kind) {
                Some(SymbolKind::Enum { id, .. }) if &id == enum_id => member,
                _ => return None,
            },
            _ => return None,
        };
        variants
            .iter()
            .any(|(name, payloads)| name == v_name && payloads.is_empty())
            .then(|| v_name.clone())
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
        // In an extension body, `name(...)` may call a field or another extension of `this`.
        if let Some(member) = self.implicit_this_callee(callee) {
            return self.infer_call_expr(&member, type_arguments, args, span, expected);
        }
        // `print(...)` and `println(...)` call the output built-ins unless a declaration of the
        // name is in scope (D15 of `codegen-core`).
        if let Some(builtin) = self.output_builtin(callee) {
            return self.infer_output_call(builtin, type_arguments, args, span);
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
                let id = self.infer_expr(callee);
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

    /// The output built-in a bare `print` or `println` callee names: only when no local,
    /// parameter, top-level declaration or import of that name is in scope, so the built-ins are
    /// found last (D15 of `codegen-core`).
    fn output_builtin(&self, callee: &Expr) -> Option<BuiltinFunction> {
        let ExprKind::Ident(name) = &callee.kind else {
            return None;
        };
        let builtin = output_builtin_named(name)?;
        let declared = self.env.lookup(name).is_some()
            || !matches!(self.env.lookup_import(name, |_| true), ImportLookup::None);
        (!declared).then_some(builtin)
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
            match self.check_printed_argument(name, &arg.value) {
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
            arguments,
            parameters,
            type_arguments: Vec::new(),
            has_receiver: false,
            is_compose: false,
        };
        self.add(ExpressionKind::Call(call), Type::Unit, span)
    }

    /// Checks the argument of an output built-in named `name`, adjusted as for a parameter of
    /// its type: a `Copy` scalar is passed by value, copied out of references, and a `String`
    /// is borrowed implicitly, as for a `&String` parameter, without being moved. Any other type
    /// is `E4004`; the error result is the node checked so far.
    fn check_printed_argument(
        &mut self,
        name: &str,
        expr: &Expr,
    ) -> Result<ExpressionId, ExpressionId> {
        let is_place = self.is_place(expr);
        let found = self.with_origin(Origin::Value, |c| {
            c.with_consuming(!is_place, |c| c.infer_expr(expr))
        });
        let found_ty = self.ty(found);
        let mut referent = &found_ty;
        while let Type::Reference { inner, .. } = referent {
            referent = inner;
        }
        let parameter = match referent {
            Type::Int | Type::Float | Type::Bool | Type::Char => referent.clone(),
            Type::String => Type::Reference {
                mutable: false,
                inner: Box::new(Type::String),
            },
            Type::Error => return Err(found),
            other => {
                self.errors.push(TypeError::UnprintableArgument {
                    function: name.to_string(),
                    ty: other.clone(),
                    span: expr.span,
                });
                return Err(found);
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
                params,
                ret,
            } => {
                // Composable context enforcement (first-class compose invariant, E4008)
                if is_compose && !self.is_in_compose() {
                    let callee_name = if let ExprKind::Ident(n) = &callee.kind {
                        n.clone()
                    } else {
                        "<closure>".to_string()
                    };
                    self.errors.push(TypeError::InvalidComposeContext {
                        callee: callee_name,
                        span,
                    });
                }

                let (param_names, has_default, type_parameters) =
                    self.callee_declaration(callee, callee_node);
                let callee_node = callee_node.expect("a function type comes from a node");
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
                            arguments,
                            evaluation_order,
                        }) => {
                            let parameters = parameter_list(&param_names, &params);
                            self.build_call(
                                callee_node,
                                arguments,
                                parameters,
                                Vec::new(),
                                evaluation_order,
                                is_compose,
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
                            arguments,
                            evaluation_order,
                        },
                        solution,
                    )) => {
                        // The call records the declared parameter types and the type
                        // arguments (D9 of `generics`).
                        let parameters = parameter_list(&param_names, &signature.params);
                        let ret = signature.ret.substitute(&solution);
                        self.build_call(
                            callee_node,
                            arguments,
                            parameters,
                            solution,
                            evaluation_order,
                            is_compose,
                            ret,
                            span,
                        )
                    }
                }
            }
            Type::Struct { id, .. } if is_constructor => {
                self.construct_struct(id, type_arguments, args, expected, span)
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
            self.errors.push(TypeError::CannotInferType {
                help: Some(TYPE_ARGUMENTS_HELP.to_string()),
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
        let (is_compose, params, ret) =
            closure_signature(declared).expect("deferred closures have a function type");
        let ExprKind::Closure {
            params: written,
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
        let node = self.check_closure(
            written,
            body,
            is_compose,
            &params,
            expected_return.as_ref(),
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
        type_arguments: Vec<Type>,
        evaluation_order: Vec<usize>,
        is_compose: bool,
        ret: Type,
        span: Span,
    ) -> ExpressionId {
        match self.builder.kind(callee_node).clone() {
            ExpressionKind::FunctionReference(path) => {
                let call = Call {
                    callee: Callee::Function(path),
                    arguments,
                    parameters,
                    type_arguments,
                    evaluation_order,
                    has_receiver: false,
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
                    type_arguments,
                    evaluation_order,
                    has_receiver: false,
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
        if !has_named {
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
                if let Some(arg_name) = &arg.name {
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
    /// struct, the built-in `clone()` of a `Clone` type, the file's own extension, the single
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
        if member == TO_STRING && has_builtin_to_string(&clone_target(&receiver_ty, is_safe_nav).0)
        {
            return self.infer_to_string_call(receiver, is_safe_nav, type_arguments, args, span);
        }

        // 3. An extension on the receiver's type, behind any `&`, `&mut` and `Heap<T>`.
        let (payload, through_nullable) = match peel_references(&receiver_ty) {
            Type::Nullable(inner) => (inner.as_ref().clone(), true),
            other => (other.clone(), false),
        };
        let mut layers = Vec::new();
        let mut reached = if through_nullable {
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
        let lookup = loop {
            match self.find_extension(&reached, member) {
                ExtensionLookup::None => {}
                found => break found,
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
                _ => break ExtensionLookup::None,
            }
        };
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
                let adjustment = self.pass_receiver(&receiver_site, &extension, member);
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
            // 4. Nothing matched.
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
        let place = self.deref_until(base, reached);
        let argument = match adjustment.pass {
            ReceiverPass::SharedBorrow => self.borrow(false, BorrowOrigin::Receiver, place),
            ReceiverPass::MutableBorrow => self.borrow(true, BorrowOrigin::Receiver, place),
            // A whole receiver is moved (or copied); a temporary is the consumed value itself.
            ReceiverPass::Move => self.consume(place),
            ReceiverPass::CopyOut => self.copy(place),
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
                    ret,
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
            self.errors.push(TypeError::InvalidComposeContext {
                callee: callee.to_string(),
                span,
            });
        }
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
                let path = DeclarationPath::extension(
                    extension.module.clone(),
                    extension.receiver.clone(),
                    member,
                );
                let call = Call {
                    callee: Callee::Function(path),
                    arguments: all_arguments,
                    parameters,
                    type_arguments,
                    evaluation_order: order,
                    has_receiver: true,
                    is_compose,
                };
                self.add(ExpressionKind::Call(call), ret, span)
            }
        }
    }

    /// The extension named `name` on receiver type `receiver` visible in this file: its own one,
    /// which shadows imported ones, else the single import exporting one.
    fn find_extension(&self, receiver: &Type, name: &str) -> ExtensionLookup {
        let key = (ReceiverKey::of(receiver), name.to_string());
        if let Some(extension) = self.own_extensions.get(&key) {
            return ExtensionLookup::Found {
                extension: Box::new(extension.clone()),
                own: true,
            };
        }
        match self.imported_extensions.get(&key).map(Vec::as_slice) {
            None | Some([]) => ExtensionLookup::None,
            Some([(_, extension)]) => ExtensionLookup::Found {
                extension: Box::new(extension.clone()),
                own: false,
            },
            Some(many) => ExtensionLookup::Ambiguous(many.iter().map(|(m, _)| m.clone()).collect()),
        }
    }

    /// Passes the receiver of a call of `extension` by its mode (D8 and D9 of
    /// `extension-methods`), reporting a `mut fun` receiver that is not a mutable place (`E4003`)
    /// and a `consuming fun` receiver that cannot be moved (`E4017`).
    fn pass_receiver(
        &mut self,
        receiver: &ReceiverSite,
        extension: &Extension,
        method: &str,
    ) -> ReceiverAdjustment {
        let expr = receiver.expr;
        let pass = match extension.mode {
            ReceiverMode::Shared => ReceiverPass::SharedBorrow,
            ReceiverMode::Mutable => {
                // A temporary may be mutated unless it is reached through a shared borrow.
                if !contents_writable(receiver.ty, receiver.writable) {
                    self.report_immutable_receiver(receiver, method);
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
                if !self.is_copy(&reached) {
                    let through_heap = receiver.layers.contains(&Layer::Heap);
                    let source = if through_heap {
                        format!(
                            "the contents of 'Heap' '{}' (moving out of a 'Heap' is not supported yet)",
                            expr
                        )
                    } else {
                        format!("borrow '{}'", expr)
                    };
                    let clone = (!through_heap && self.is_clone(&reached)).then(|| {
                        if receiver.through_nullable {
                            format!("{}?.clone()", expr)
                        } else {
                            format!("{}.clone()", expr)
                        }
                    });
                    self.errors.push(TypeError::CannotMoveOut {
                        source,
                        ty: reached,
                        borrow: None,
                        clone,
                        span: expr.span,
                    });
                }
                ReceiverPass::CopyOut
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

    /// Whether `expr` is an optional place: a chain of `?.` (or `.`) field accesses whose root is
    /// a place and which has at least one `?.` (`t?.lead` in `t?.lead?.greet()`). As the receiver
    /// of a call it is borrowed where it lives when every link is present (D10 of
    /// `extension-methods`).
    fn is_optional_place(&self, expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::MemberAccess {
                target,
                is_safe_nav,
                ..
            } => {
                self.type_name_symbol(target).is_none()
                    && ((*is_safe_nav && self.is_place(target)) || self.is_optional_place(target))
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

        if !has_named_args {
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
            if let Some(arg_name) = &arg.name {
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
        if nullable_view(found).is_some_and(|view| view.is_assignable_to(expected)) {
            return Coercion::Fits(Adjustment::NullableView);
        }
        self.copy_out(found, expected)
    }

    /// Copies a `Copy` value out of the references around `found` until it fits `expected`,
    /// directly or inside a `?` (`(&T)?` as a `T?`).
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
        while let Type::Reference { inner, .. } = referent {
            referent = inner;
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
                }
            };
        }
        Coercion::NoFit
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
        match self.coerce(&found, expected, site) {
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
            Coercion::Fits(adjustment) => self.apply_adjustment(found_id, adjustment, expected),
            Coercion::MovesOutOfBorrow { ty, nullable } => {
                let clone = self.is_clone(&ty).then(|| match nullable {
                    true => format!("{}?.clone()", expr),
                    false => format!("{}.clone()", expr),
                });
                self.errors.push(TypeError::CannotMoveOut {
                    source: format!("borrow '{}'", expr),
                    ty: if nullable { ty.nullable() } else { ty },
                    borrow: None,
                    clone,
                    span: expr.span,
                });
                self.wrap_error(found_id)
            }
            Coercion::NoFit => {
                if found == Type::Error {
                    return found_id;
                }
                let help = borrow_help(expr, &found, expected, site);
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
        let mut borrowed = false;
        let source = match &expr.kind {
            ExprKind::Ident(name) if self.is_default_input(name) => {
                borrowed = true;
                format!("parameter '{}', which a default value borrows", name)
            }
            ExprKind::This if self.is_default_input(THIS) => {
                borrowed = true;
                "'this', which a default value borrows".to_string()
            }
            ExprKind::Ident(name) if self.is_capture(name) => {
                borrowed = true;
                format!("captured variable '{}', which the closure borrows", name)
            }
            ExprKind::This if self.is_capture(THIS) => {
                borrowed = true;
                "'this', which the closure borrows".to_string()
            }
            ExprKind::Ident(name) if self.is_top_level_value(name) => {
                format!("top-level value '{}'", name)
            }
            // `Color.Red` names a variant: a fresh value, not a field.
            ExprKind::MemberAccess { target, .. } if self.type_name_symbol(target).is_some() => {
                return
            }
            ExprKind::MemberAccess { target, member, .. } => {
                format!("field '{}' of '{}'", member, target)
            }
            ExprKind::Index { target, .. } => format!("an element of '{}'", target),
            _ => return,
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
        self.errors.push(TypeError::CannotMoveOut {
            source,
            ty: ty.clone(),
            borrow: (!is_borrowed).then(|| format!("&{}", expr)),
            clone,
            span: expr.span,
        });
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
                Type::String => return name == STRING_LENGTH,
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
            type_arguments: Vec::new(),
            evaluation_order: vec![0],
            has_receiver: true,
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
            arguments: written,
            parameters,
            type_arguments: Vec::new(),
            has_receiver: true,
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
        let place = self.peel_reference_nodes(base);
        let copy = self.copy(place);
        let copy_ty = self.ty(copy);
        let call = Call {
            callee: Callee::Builtin(BuiltinFunction::ToString),
            arguments: vec![Argument::Written(copy)],
            parameters: vec![(Some(THIS.to_string()), copy_ty)],
            type_arguments: Vec::new(),
            evaluation_order: vec![0],
            has_receiver: true,
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
        let target_id = self.infer_expr(target);
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
            Type::Enum { id, .. } => self
                .enum_variants(id)
                .is_some_and(|variants| variants.iter().any(|(v, _)| v == name)),
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
            Type::Struct { id, arguments } if self.struct_fields(id).is_some() => {
                // Fields have their declared types with the type arguments substituted.
                let fields = self
                    .types
                    .instantiated_fields(id, arguments)
                    .unwrap_or_default();
                if let Some((_, is_var, f_ty)) = fields.iter().find(|(n, _, _)| n == member) {
                    (wrap(f_ty.clone()), Some(*is_var))
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
            Type::Enum { id, arguments } if self.enum_variants(id).is_some() => {
                // Through the enum's name (`Option.Some`), the enum is applied to its own
                // parameters, which the use solves (D8 of `generics`).
                let arguments = if arguments.is_empty() {
                    parameter_types(self.types.parameters(id))
                } else {
                    arguments.clone()
                };
                let variants = self
                    .types
                    .instantiated_variants(id, &arguments)
                    .unwrap_or_default();
                if let Some((_, payloads)) = variants.iter().find(|(v_name, _)| v_name == member) {
                    let enum_ty = Type::Enum {
                        id: id.clone(),
                        arguments,
                    };
                    if payloads.is_empty() {
                        (wrap(enum_ty), None)
                    } else {
                        let ctor = Type::Fn {
                            is_compose: false,
                            params: payloads.clone(),
                            ret: Box::new(enum_ty),
                        };
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
            Type::String if member == STRING_LENGTH => (wrap(Type::Int), Some(false)),
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
            _ => (self.infer_expr(expr), true),
        }
    }
}

/// The parameters of a callee as a call records them: each with its name, when the callee is a
/// declared function, and its type.
/// The output built-in called by the bare name `name`, if any (D15 of `codegen-core`).
fn output_builtin_named(name: &str) -> Option<BuiltinFunction> {
    match name {
        "print" => Some(BuiltinFunction::Print),
        "println" => Some(BuiltinFunction::PrintLine),
        _ => None,
    }
}

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
        _ => unreachable!("only the ordering operators are reported by their text"),
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

/// The borrow to suggest when `expr`, of type `found`, does not fit the reference `expected`
/// but its referent would: a written `&` outside arguments (D1 of `reference-access`), and a
/// written `&mut` everywhere, since a mutable borrow is never implicit.
fn borrow_help(expr: &Expr, found: &Type, expected: &Type, site: Site) -> Option<String> {
    let Type::Reference { mutable, inner } = expected else {
        return None;
    };
    if matches!(found, Type::Reference { .. }) || !found.is_assignable_to(inner) {
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

/// The name of the built-in `toString()` of the scalar types.
const TO_STRING: &str = "toString";

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

/// The function type a closure argument is checked against, for a parameter of declared type
/// `declared`: a function type, possibly nullable. Returns its compose flag, parameter types and
/// return type.
fn closure_signature(declared: &Type) -> Option<(bool, Vec<Type>, Type)> {
    match declared {
        Type::Fn {
            is_compose,
            params,
            ret,
        } => Some((*is_compose, params.clone(), (**ret).clone())),
        Type::Nullable(inner) => closure_signature(inner),
        _ => None,
    }
}

/// The help of an operator applied to a value of a type parameter (D5 of `generics`).
fn type_parameter_operator_help(left: &Type, right: &Type) -> Option<String> {
    (left.mentions_parameter_outside_functions() || right.mentions_parameter_outside_functions())
        .then(|| "type parameters have no '==' or operators until traits exist".to_string())
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
        Type::Reference { inner, .. } | Type::Nullable(inner) | Type::Heap(inner) => {
            type_size(inner)
        }
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
            Type::Fn {
                is_compose: true,
                params: Vec::new(),
                ret: Box::new(Type::Unit),
            },
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
                patterns: vec![WhenPattern::Expr(expr(ExprKind::Literal(Literal::Int(1))))],
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
            Type::Fn {
                is_compose: false,
                params: vec![Type::Int, Type::Int],
                ret: Box::new(Type::Int),
            },
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
                },
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::String("bad".to_string()))),
                    span: Span::dummy(),
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

        let expected_fn = Type::Fn {
            is_compose: false,
            params: vec![Type::Int],
            ret: Box::new(Type::Int),
        };

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
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Literal(Literal::Int(0))))],
                    body: expr(ExprKind::Return(Some(Box::new(expr(ExprKind::Literal(
                        Literal::Int(99),
                    )))))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![WhenPattern::Else],
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
                },
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(20))),
                    span: Span::dummy(),
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
                },
                CallArg {
                    name: Some("y".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(20))),
                    span: Span::dummy(),
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
                    },
                    CallArg {
                        name: None,
                        value: expr(ExprKind::Literal(Literal::Int(20))),
                        span: Span::dummy(),
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
                    },
                    CallArg {
                        name: None,
                        value: expr(ExprKind::Literal(Literal::Int(30))),
                        span: Span::dummy(),
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
                    is_compose: false,
                    receiver: None,
                    name: "add".to_string(),
                    type_parameters: Vec::new(),
                    params: vec![
                        ("a".to_string(), AbiType::Prim(PrimKind::Int), None),
                        ("b".to_string(), AbiType::Prim(PrimKind::Int), None),
                    ],
                    return_type: AbiType::Prim(PrimKind::Int),
                    generic_body_hash: None,
                },
                ExportedSymbol::Function {
                    visibility: Visibility::Package,
                    is_compose: false,
                    receiver: None,
                    name: "internal_calc".to_string(),
                    type_parameters: Vec::new(),
                    params: Vec::new(),
                    return_type: AbiType::Prim(PrimKind::Int),
                    generic_body_hash: None,
                },
            ],
            abi_hash: prism_abi::ContentHash([1; 32]),
            interface_hash: prism_abi::ContentHash([2; 32]),
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
                },
                CallArg {
                    name: None,
                    value: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
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
        let fn_ty = Type::Fn {
            is_compose: false,
            params: vec![Type::String, Type::Int, Type::String],
            ret: Box::new(Type::String),
        };
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
                },
                CallArg {
                    name: Some("prefix".to_string()),
                    value: expr(ExprKind::Literal(Literal::String(">>".to_string()))),
                    span: Span::dummy(),
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
                },
                CallArg {
                    name: Some("msg".to_string()),
                    value: expr(ExprKind::Literal(Literal::String("b".to_string()))),
                    span: Span::dummy(),
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
                },
                CallArg {
                    name: Some("y".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
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
                },
                CallArg {
                    name: Some("x".to_string()),
                    value: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
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
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Ident("Red".to_string())))],
                    body: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Ident(
                        "Green".to_string(),
                    )))],
                    body: expr(ExprKind::Literal(Literal::Int(2))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Ident("Blue".to_string())))],
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
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Ident("Red".to_string())))],
                    body: expr(ExprKind::Literal(Literal::Int(1))),
                    span: Span::dummy(),
                },
                WhenBranch {
                    patterns: vec![WhenPattern::Expr(expr(ExprKind::Ident(
                        "Green".to_string(),
                    )))],
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
            Type::Fn {
                is_compose: true,
                params: Vec::new(),
                ret: Box::new(Type::Unit),
            },
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
        let function = Type::Fn {
            is_compose: false,
            params: Vec::new(),
            ret: Box::new(Type::Unit),
        };
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
