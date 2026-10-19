//! The typed tree of one file: its declarations, and one body per function and top-level `val`.
//!
//! The tree's invariants, checked by [`TypedFile::validate`]:
//!
//! - **Places** (D5 of `typed-tree`). [`ExpressionKind::Local`], [`ExpressionKind::TopLevelValue`],
//!   [`ExpressionKind::Field`], [`ExpressionKind::Deref`], [`ExpressionKind::Index`],
//!   [`ExpressionKind::NonNullPayload`] and [`ExpressionKind::Temporary`] are places. A place
//!   whose value is consumed is wrapped in [`ExpressionKind::Move`] or [`ExpressionKind::Copy`];
//!   a place anywhere else is read in place. The operand of a move, copy, borrow, field or index,
//!   and an assignment's target, is always a place.
//! - **Slot types** (D6). In a tree without type errors, every child's type equals the type its
//!   position expects (an argument its parameter, a value its place, a branch its `if`, a tail
//!   its block); where a value fits only by subtyping, a [`ExpressionKind::Coerce`] node of the
//!   expected type wraps it.
//! - **Chains** (D7). [`ExpressionKind::NonNullPayload`] appears only inside the chain of a
//!   [`ExpressionKind::SafeNavigation`], which it leaves with `null` when its operand is `null`.
//! - **Error nodes** (D12). A tree without type errors has no [`ExpressionKind::Error`] node and
//!   no node or binding of the error type.

use crate::arena::{Arena, BodyId, ExpressionId, LocalId, StatementId};
use crate::types::{NominalId, Type, TypeParameter, TypeTable};
use prism_ast::{Literal, ReceiverMode, Span, TypeMarker, Visibility};
use std::fmt;

/// The typed tree of one file.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedFile {
    /// The file's module path (`app/main`).
    pub module: String,
    /// The file's package (`app`), `None` at the workspace root.
    pub package: Option<String>,
    /// The file's declarations, in source order.
    pub items: Vec<Item>,
    /// The bodies the items point at, in checking order.
    pub bodies: Arena<BodyId, Body>,
    /// Every struct and enum the file knows, with its marker and, when known, its fields or
    /// variants (D4 of `mir-lowering`), so later stages need neither the checker nor the ABIs.
    pub types: TypeTable,
    /// The `external` functions the file declares or imports (D5 of `standard-library`): a
    /// function reference to one of them is lowered to an adapter calling it by symbol.
    pub externals: Vec<DeclarationPath>,
}

/// A declaration of the file.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Function(FunctionItem),
    Struct(StructItem),
    Enum(EnumItem),
    Value(ValueItem),
}

/// A function or extension function.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionItem {
    pub path: DeclarationPath,
    pub visibility: Visibility,
    pub is_compose: bool,
    /// The type parameters of a generic function, in order, with their bounds; the types of
    /// its signature and body mention them.
    pub type_parameters: Vec<TypeParameter>,
    /// The receiver of an extension: its mode and its type (`User` for `fun User.greet()`).
    pub receiver: Option<(ReceiverMode, Type)>,
    /// The context parameter: its name and type (`scope` and `RowScope` for
    /// `context(scope: RowScope)`), passed as a shared borrow before the receiver (D9 of
    /// `receiver-lambdas`).
    pub context: Option<Box<(String, Type)>>,
    /// The declared parameters, without the context and the receiver.
    pub parameters: Vec<(String, Type)>,
    /// For each declared parameter, what its default value reads of its generator's inputs
    /// (D26 of `ui-toolkit`); [`prism_ast::DefaultInputs::ALL`] for one without a default.
    pub default_inputs: Vec<prism_ast::DefaultInputs>,
    /// The declared or inferred return type.
    pub return_type: Type,
    pub body: BodyId,
    pub span: Span,
}

impl FunctionItem {
    /// How many parameters precede the declared ones: the context and the receiver.
    pub fn leading_parameters(&self) -> usize {
        usize::from(self.context.is_some()) + usize::from(self.receiver.is_some())
    }
}

/// A struct declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct StructItem {
    pub id: NominalId,
    pub visibility: Visibility,
    /// The type parameters of a generic struct, which its field types mention.
    pub type_parameters: Vec<TypeParameter>,
    pub marker: Option<TypeMarker>,
    /// The indices of the type parameters that condition the marker (D6 of `generics`).
    pub marker_parameters: Vec<u32>,
    /// The fields as `(name, is_var, type)`, in declaration order: a field is named by its index.
    pub fields: Vec<(String, bool, Type)>,
    pub span: Span,
}

/// An enum declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumItem {
    pub id: NominalId,
    pub visibility: Visibility,
    /// The type parameters of a generic enum, which its payload types mention.
    pub type_parameters: Vec<TypeParameter>,
    pub marker: Option<TypeMarker>,
    /// The indices of the type parameters that condition the marker (D6 of `generics`).
    pub marker_parameters: Vec<u32>,
    /// The variants as `(name, payload types)`, in declaration order: a variant is named by its
    /// index.
    pub variants: Vec<(String, Vec<Type>)>,
    pub span: Span,
}

/// A top-level `val`.
#[derive(Debug, Clone, PartialEq)]
pub struct ValueItem {
    pub path: DeclarationPath,
    pub visibility: Visibility,
    /// The declared or inferred type.
    pub ty: Type,
    pub body: BodyId,
    pub span: Span,
}

/// The identity of a top-level function, extension or `val`: its defining module, the receiver
/// type for an extension, its name (D4), and the context type of an extension with a context
/// parameter (D9 of `receiver-lambdas`). It is the ABI's identity for an exported symbol, unique
/// because Prism has no overloading and one extension per receiver type, name and context.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeclarationPath {
    pub module: String,
    pub receiver: Option<Type>,
    pub name: String,
    /// The context type of an extension with a context parameter; `None` for every other
    /// declaration, a plain function with a context included, which is not overloaded by it.
    pub context: Option<Box<Type>>,
}

impl DeclarationPath {
    /// The path of a plain declaration `name` of `module`.
    pub fn new(module: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            module: module.into(),
            receiver: None,
            name: name.into(),
            context: None,
        }
    }

    /// The path of the extension `name` on `receiver`, declared in `module`.
    pub fn extension(module: impl Into<String>, receiver: Type, name: impl Into<String>) -> Self {
        Self {
            module: module.into(),
            receiver: Some(receiver),
            name: name.into(),
            context: None,
        }
    }

    /// The path with the context type `context`, which only an extension's path records.
    pub fn with_context(mut self, context: Option<Type>) -> Self {
        if self.receiver.is_some() {
            self.context = context.map(Box::new);
        }
        self
    }

    /// `#context(<type>)` after the path of an extension with a context, `render` writing the
    /// type; empty for any other path.
    pub fn context_suffix(&self, render: impl Fn(&Type) -> String) -> String {
        match &self.context {
            Some(context) => format!("#context({})", render(context)),
            None => String::new(),
        }
    }
}

impl fmt::Display for DeclarationPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.receiver {
            Some(receiver) => write!(
                f,
                "{}::{}.{}{}",
                self.module,
                receiver,
                self.name,
                self.context_suffix(Type::to_string)
            ),
            None => write!(f, "{}::{}", self.module, self.name),
        }
    }
}

/// The body of a function or of a top-level `val`: its bindings, expressions and statements.
/// A closure's parameters and locals live in the body of its enclosing declaration (D11).
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    /// The declaration the body belongs to.
    pub owner: DeclarationPath,
    pub locals: Arena<LocalId, Local>,
    pub expressions: Arena<ExpressionId, Expression>,
    pub statements: Arena<StatementId, Statement>,
    /// The function's parameters, `this` first for an extension. Empty for a `val`.
    pub parameters: Vec<LocalId>,
    /// One default value per declared parameter (without `this`), evaluated at the call. It may
    /// read `this` and the earlier parameters.
    pub defaults: Vec<Option<ExpressionId>>,
    /// The function's body or the `val`'s initializer; `None` for a function without a body.
    pub value: Option<ExpressionId>,
}

impl Body {
    /// An empty body owned by `owner`.
    pub fn new(owner: DeclarationPath) -> Self {
        Self {
            owner,
            locals: Arena::new(),
            expressions: Arena::new(),
            statements: Arena::new(),
            parameters: Vec::new(),
            defaults: Vec::new(),
            value: None,
        }
    }

    /// The type of expression `id`.
    pub fn type_of(&self, id: ExpressionId) -> &Type {
        &self.expressions[id].ty
    }
}

/// A binding of a body. Two declarations with the same name are distinct bindings.
#[derive(Debug, Clone, PartialEq)]
pub struct Local {
    pub name: String,
    pub ty: Type,
    /// Whether the binding is a `var`.
    pub mutable: bool,
    pub kind: LocalKind,
    /// Whether a parameter or closure parameter is `escaping`: its function value holds no
    /// borrow and may be kept (D4 of `escaping-closures`). Always false for other bindings.
    pub escaping: bool,
    pub span: Span,
}

/// What declares a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    /// The `this` of an extension.
    Receiver,
    /// A function parameter.
    Parameter,
    /// A local `val` or `var`.
    Let,
    /// The variable of a `for` loop.
    ForVariable,
    /// A closure parameter.
    ClosureParameter,
    /// The implicit parameter `it` of a closure without a header, declared at its `{` (D5 of
    /// `implicit-it`).
    ImplicitClosureParameter,
    /// The `this` of a lambda with a receiver, its first parameter, declared at its `{` (D3 of
    /// `receiver-lambdas`).
    ClosureReceiver,
    /// A binding of a pattern (`val r` in `Shape.Circle(val r)`) or a destructuring
    /// declaration, whose type its [`BindingMode`] gives (D14 of `payload-patterns`).
    PatternBinding,
}

/// An expression node: its kind, its type and the span of the source it stands for.
#[derive(Debug, Clone, PartialEq)]
pub struct Expression {
    pub kind: ExpressionKind,
    pub ty: Type,
    pub span: Span,
}

/// What an expression node is.
#[derive(Debug, Clone, PartialEq)]
pub enum ExpressionKind {
    /// A literal value.
    Literal(Literal),

    // Places (D5).
    /// A binding of the body.
    Local(LocalId),
    /// A top-level `val` of the file or of a direct import.
    TopLevelValue(DeclarationPath),
    /// Field `index` (named `name`) of the struct value `base`, a place.
    Field {
        base: ExpressionId,
        index: usize,
        name: String,
    },
    /// The referent of `pointer`, a `&T`, `&mut T` or `Heap<T>`.
    Deref { pointer: ExpressionId },
    /// The element `index` of the place `base`, a `List<T>` (D7 of `collections`). A character
    /// of a `String` is computed, not stored, so `s[i]` is a call of
    /// [`BuiltinFunction::StringCharacter`] instead (D5 of `runtime-strings-and-drops`).
    Index {
        base: ExpressionId,
        index: ExpressionId,
    },
    /// The payload of `nullable`, which leaves the innermost enclosing chain with `null` when
    /// `nullable` is `null` (D7). A place when `nullable` is one.
    NonNullPayload { nullable: ExpressionId },
    /// `value` evaluated into storage that lives until the end of the enclosing statement.
    Temporary { value: ExpressionId },
    /// The value of the state cell `handle` names, a `State<T>` or `MutableState<T>` read
    /// out of its place (D17 of `state-handles`): the target of an assignment of `s.value`,
    /// or, in a composable context, a place read in place, borrowed or projected. The node's
    /// type is the value's.
    StatePlace { handle: ExpressionId },

    // Uses of places.
    /// The value of the place, moved out of it.
    Move { place: ExpressionId },
    /// The value of the place, copied (its type is `Copy`).
    Copy { place: ExpressionId },
    /// A borrow of the place.
    Borrow {
        mutable: bool,
        origin: BorrowOrigin,
        place: ExpressionId,
    },
    /// Evaluates `chain`, whose [`ExpressionKind::NonNullPayload`] nodes may leave it with
    /// `null`. Its type is the chain's type made nullable (D7).
    SafeNavigation {
        chain: ExpressionId,
        origin: SafeNavigationOrigin,
    },
    /// `value` converted to this node's type by assignability (D6).
    Coerce {
        value: ExpressionId,
        kind: CoercionKind,
    },

    // Values.
    /// A top-level function read as a value.
    FunctionReference(DeclarationPath),
    /// A variant with a payload, read as its constructor function.
    VariantConstructor {
        enum_id: NominalId,
        index: usize,
        name: String,
    },
    /// A call (D8).
    Call(Call),
    /// A struct or variant construction, its fields in declaration order (D8).
    Construct {
        kind: ConstructKind,
        fields: Vec<ConstructField>,
        /// The field indices of the written values, in source order.
        evaluation_order: Vec<usize>,
    },
    /// `start..end` or `start..<end`, of the built-in `Range` type.
    Range {
        start: ExpressionId,
        end: ExpressionId,
        end_inclusive: bool,
    },
    /// An arithmetic or comparison operator over operands read in place.
    Binary {
        operator: BinaryOperator,
        left: ExpressionId,
        right: ExpressionId,
    },
    /// `-` or `!` over an operand read in place.
    Unary {
        operator: UnaryOperator,
        operand: ExpressionId,
    },
    /// `&&` or `||`, which evaluates `right` only when needed.
    Logical {
        operator: LogicalOperator,
        left: ExpressionId,
        right: ExpressionId,
    },
    /// `nullable ?: fallback`, which evaluates `fallback` only when `nullable` is `null`.
    Elvis {
        nullable: ExpressionId,
        fallback: ExpressionId,
    },
    /// A string template, of type `String`: its text runs and the values of its parts, in source
    /// order (D9 of `string-templates`). A value part is a `String`, `Int`, `Double`, `Bool` or
    /// `Char` read in place, as an operator's operand is: a place is not wrapped in a move or a
    /// copy, and a computed value (a `toString()` call among them) is a temporary.
    Template { parts: Vec<TemplatePart> },

    // Control flow (D9).
    /// A block: its statements, and its tail, which is the block's value. A block whose value is
    /// discarded has no tail. `close_brace` is the span of its closing `}` (the whole block's
    /// for an unbraced or synthesized block), where the MIR of a block that falls through drops
    /// its locals (D15 of `borrowck-exclusivity`).
    Block {
        statements: Vec<StatementId>,
        tail: Option<ExpressionId>,
        close_brace: Span,
    },
    If {
        condition: ExpressionId,
        then_branch: ExpressionId,
        else_branch: Option<ExpressionId>,
    },
    /// A `when`, its subject read in place.
    When {
        subject: Option<ExpressionId>,
        branches: Vec<WhenBranch>,
        /// Whether the branches cover every value of the subject.
        exhaustive: bool,
    },
    /// `return`, of type `Nothing`.
    Return { value: Option<ExpressionId> },
    /// `break`, naming the loop it leaves; `None` only in a tree with type errors.
    Break { target: Option<StatementId> },
    /// `continue`, naming the loop it continues; `None` only in a tree with type errors.
    Continue { target: Option<StatementId> },
    /// A closure (D11). Its parameters and locals are bindings of the enclosing body.
    Closure {
        parameters: Vec<LocalId>,
        body: ExpressionId,
        is_compose: bool,
        /// The bindings of enclosing scopes its body uses, each once, in first-use order, with
        /// how the closure takes each (D2 of `escaping-closures`).
        captures: Vec<Capture>,
        /// Why the closure owns some of its captures, when it does: the first position, in
        /// source order, that makes its value escape, or the store that keeps it outside the
        /// block of a captured binding.
        ownership_reason: Option<OwnershipReason>,
    },

    /// Where checking failed: of the error type, keeping the nodes that were checked (D12).
    Error { children: Vec<ExpressionId> },
}

impl ExpressionKind {
    /// Whether the node is a place (D5). An error node counts as one, so a tree with type errors
    /// reports no follow-on violation.
    pub fn is_place(&self) -> bool {
        matches!(
            self,
            ExpressionKind::Local(_)
                | ExpressionKind::TopLevelValue(_)
                | ExpressionKind::Field { .. }
                | ExpressionKind::Deref { .. }
                | ExpressionKind::Index { .. }
                | ExpressionKind::NonNullPayload { .. }
                | ExpressionKind::Temporary { .. }
                | ExpressionKind::StatePlace { .. }
                | ExpressionKind::Error { .. }
        )
    }

    /// The expression children of the node, with its statements (a block's) pushed onto
    /// `statements`. A closure's child is its body.
    pub fn children(&self, statements: &mut Vec<StatementId>) -> Vec<ExpressionId> {
        match self {
            ExpressionKind::Literal(_)
            | ExpressionKind::Local(_)
            | ExpressionKind::TopLevelValue(_)
            | ExpressionKind::FunctionReference(_)
            | ExpressionKind::VariantConstructor { .. }
            | ExpressionKind::Break { .. }
            | ExpressionKind::Continue { .. } => Vec::new(),
            ExpressionKind::Field { base, .. } => vec![*base],
            ExpressionKind::Deref { pointer } => vec![*pointer],
            ExpressionKind::Index { base, index } => vec![*base, *index],
            ExpressionKind::NonNullPayload { nullable } => vec![*nullable],
            ExpressionKind::Temporary { value }
            | ExpressionKind::StatePlace { handle: value }
            | ExpressionKind::Coerce { value, .. }
            | ExpressionKind::Move { place: value }
            | ExpressionKind::Copy { place: value }
            | ExpressionKind::Borrow { place: value, .. }
            | ExpressionKind::SafeNavigation { chain: value, .. }
            | ExpressionKind::Unary { operand: value, .. } => vec![*value],
            ExpressionKind::Call(call) => {
                let callee = match call.callee {
                    Callee::Value(callee) => Some(callee),
                    _ => None,
                };
                callee
                    .into_iter()
                    .chain(call.arguments.iter().filter_map(|a| match a {
                        Argument::Written(value) => Some(*value),
                        Argument::Default(_) => None,
                    }))
                    .collect()
            }
            ExpressionKind::Construct { fields, .. } => fields.iter().map(|f| f.value).collect(),
            ExpressionKind::Range { start, end, .. } => vec![*start, *end],
            ExpressionKind::Binary { left, right, .. }
            | ExpressionKind::Logical { left, right, .. } => {
                vec![*left, *right]
            }
            ExpressionKind::Elvis { nullable, fallback } => vec![*nullable, *fallback],
            ExpressionKind::Template { parts } => parts
                .iter()
                .filter_map(|part| match part {
                    TemplatePart::Value(value) => Some(*value),
                    TemplatePart::Text(_) => None,
                })
                .collect(),
            ExpressionKind::Block {
                statements: s,
                tail,
                ..
            } => {
                statements.extend(s.iter().copied());
                tail.iter().copied().collect()
            }
            ExpressionKind::If {
                condition,
                then_branch,
                else_branch,
            } => [Some(*condition), Some(*then_branch), *else_branch]
                .into_iter()
                .flatten()
                .collect(),
            ExpressionKind::When {
                subject, branches, ..
            } => {
                let mut children: Vec<ExpressionId> = subject.iter().copied().collect();
                for branch in branches {
                    for pattern in &branch.patterns {
                        pattern.expressions(&mut children);
                    }
                    children.extend(branch.guard);
                    children.push(branch.body);
                }
                children
            }
            ExpressionKind::Return { value } => value.iter().copied().collect(),
            ExpressionKind::Closure { body, .. } => vec![*body],
            ExpressionKind::Error { children } => children.clone(),
        }
    }
}

/// A part of a [`ExpressionKind::Template`].
#[derive(Debug, Clone, PartialEq)]
pub enum TemplatePart {
    /// A run of text, its escapes decoded.
    Text(String),
    /// A value inserted as its text.
    Value(ExpressionId),
}

/// A normalized call (D8).
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub callee: Callee,
    /// One argument per parameter, in parameter order: the context first when `has_context`,
    /// then the receiver when `has_receiver`.
    pub arguments: Vec<Argument>,
    /// The parameters as the callee declares them, in the same order: the name (`None` for a
    /// function-typed value) and the declared type, which mentions a generic callee's type
    /// parameters. An extension's receiver is `this`, of the type it is passed as (`&T`, `&mut T`
    /// or `T`). The type of argument `i` is [`Call::parameter_type`].
    pub parameters: Vec<(Option<String>, Type)>,
    /// Whether each parameter is `escaping`, in the same order: the callee may keep what is
    /// passed for it (D3 of `escaping-closures`). The receiver of an extension never is.
    pub escaping: Vec<bool>,
    /// The type arguments of a generic callee, written or inferred, in the order of its type
    /// parameters; empty for any other callee. Inside a generic body they may mention the
    /// body's own type parameters (D9 of `generics`).
    pub type_arguments: Vec<Type>,
    /// The parameter indices of the written arguments, in source order, the receiver first.
    /// Defaults are evaluated after them.
    pub evaluation_order: Vec<usize>,
    /// Whether the first argument is an extension's receiver, the second one after a context.
    pub has_receiver: bool,
    /// Whether the first argument is the context argument of a callee with a context parameter
    /// (D9 of `receiver-lambdas`).
    pub has_context: bool,
    /// Whether the callee is a `compose fun` or composable function value.
    pub is_compose: bool,
}

impl Call {
    /// The type of argument `index`: its declared parameter type with the call's type
    /// arguments substituted.
    pub fn parameter_type(&self, index: usize) -> Option<Type> {
        self.parameters
            .get(index)
            .map(|(_, ty)| ty.substitute(&self.type_arguments))
    }
}

/// What a call calls.
#[derive(Debug, Clone, PartialEq)]
pub enum Callee {
    /// A top-level function or extension, of the file or of a direct import.
    Function(DeclarationPath),
    /// An `external fun` of the bundled library: a runtime entry point called directly by the
    /// C symbol equal to its name, with no function-table slot (D5 of `standard-library`).
    External(DeclarationPath),
    /// A function the compiler provides.
    Builtin(BuiltinFunction),
    /// A function-typed value, read in place.
    Value(ExpressionId),
}

/// A function the compiler provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinFunction {
    /// `clone()` of a `Clone` type, called with a shared receiver borrow.
    Clone,
    /// `print(x)`: writes the text of its argument, an `Int`, `Double`, `Bool` or `Char` passed
    /// by value or a borrowed `String` (D15 of `codegen-core`).
    Print,
    /// `println(x)` or `println()`: writes the text of its argument, if any, then a newline.
    PrintLine,
    /// `s.length` of a `String`, called with a shared receiver borrow: its number of characters
    /// (D6 of `runtime-strings-and-drops`).
    StringLength,
    /// `s[i]` of a `String`, called with a shared receiver borrow and the `Int` index: the
    /// character at character index `i` (D5).
    StringCharacter,
    /// `x.toString()` of an `Int`, `Double`, `Bool` or `Char`, called with the receiver copied
    /// out of any reference: the text `print(x)` writes (D6).
    ToString,
    /// `Heap(value)`: allocates a box and moves the value into it (D2 of `heap-boxes`). Declared
    /// like `fun <T> Heap(value: T): Heap<T>`: its parameter is the type parameter `T` and its
    /// type argument the contents' type.
    HeapAllocate,
    /// `h.unbox()` of a `Heap<T>`, called with the box moved as its receiver: moves the contents
    /// out and frees the box (D6 of `heap-boxes`).
    Unbox,
    /// `listOf(e1, ..., en)`: a list of exactly its arguments, in order (D4 of `collections`).
    /// Declared like `fun <T> listOf(e1: T, ..., en: T): List<T>`, with as many parameters as the
    /// call has arguments; its type argument is the element type.
    ListOf,
    /// `xs.size` of a `List<T>`: its number of elements. Like every list member below, declared
    /// as an extension on `List<T>` ([`BuiltinFunction::list_member_signature`]) with the
    /// element type as its type argument (D6 of `collections`).
    ListSize,
    /// `xs.indices` of a `List<T>`: the `Range` `0..<xs.size`.
    ListIndices,
    /// `xs.isEmpty()`.
    ListIsEmpty,
    /// `xs.isNotEmpty()`.
    ListIsNotEmpty,
    /// `xs.contains(element)`: whether an element equals `element`.
    ListContains,
    /// `xs.indexOf(element)`: the first index of an element equal to `element`, or -1.
    ListIndexOf,
    /// `xs.add(element)`: appends `element`.
    ListAdd,
    /// `xs.insert(index, element)`: places `element` at `index`, moving the later elements up.
    ListInsert,
    /// `xs.removeAt(index)`: gives the element at `index` by move, moving the later ones down.
    ListRemoveAt,
    /// `xs.removeLast()`: gives the last element by move.
    ListRemoveLast,
    /// `xs.reverse()`: reverses the order of the elements in place.
    ListReverse,
    /// `xs.clear()`: drops every element, keeping the buffer.
    ListClear,
    /// `composition(content)` of `/prism/compose/runtime`: starts a composition of the
    /// composable `content`, composes it once and gives the owned `Composition` (D15 of
    /// `compose-transform`).
    Composition,
    /// `c.setContent(content)`: replaces a composition's content and recomposes its root.
    SetContent,
    /// `c.recompose()`: recomposes a composition's invalid scopes, giving whether any ran.
    Recompose,
    /// `c.dump()`: the text dump of a composition.
    DumpComposition,
    /// `remember(keys..., calculation)`, a composable: the kept value of its slot, or the
    /// calculation's result when the slot is new or a key changed (D12). Typed by a rule: up to
    /// three keys, each of a type parameter of its own bounded by `Copy`, before the
    /// calculation; its type arguments are the value's type, then the keys' types.
    Remember,
    /// `key(value) { content }`, a composable: composes `content` in a movable group whose
    /// data key is `value` (D13).
    Key,
    /// `currentRecomposeScope()`, a composable: the innermost restart scope being composed.
    CurrentRecomposeScope,
    /// `s.invalidate()` of a `RecomposeScope`.
    InvalidateScope,
    /// `mutableStateOf(value)`: creates a state cell holding `value`, moved in, and gives a
    /// `MutableState<T>` of it (D2 of `state-handles`). Its type argument is the value type.
    MutableStateOf,
    /// `s.value` of a `State<T>` or `MutableState<T>` whose `T` is `Copy`: a copy of the
    /// cell's value, called with the handle copied as its receiver (D5). Any other `s.value`
    /// is an [`ExpressionKind::StatePlace`].
    StateValue,
    /// `s.read(reader)`: calls `reader` with a shared borrow of the cell's value and gives its
    /// result (D6). Its type arguments are the value type and the result type.
    StateRead,
    /// `s.update(transform)`: moves the value out of the cell, calls `transform` with it and
    /// moves the result back in.
    StateUpdate,
    /// `s.edit(editor)`: calls `editor` with a mutable borrow of the cell's value.
    StateEdit,
    /// `SideEffect(effect)`, a composable: queues `effect` to run after the pass (D13).
    SideEffect,
    /// `Node(kind, update, content)` of `/prism/compose/ui`, a composable: a node group of
    /// `kind` whose node `update` receives, holding `content` (D3 of `ui-toolkit`).
    UiNode,
    /// `LeafNode(kind, update)`: a node group without content.
    UiLeafNode,
    /// `runApp(title, width, height, content)` of `/prism/compose/ui`: runs an app of the
    /// composable `content` until its window closes (D15 of `ui-toolkit`).
    RunApp,
    /// `n.dp` of an `Int`: the construction `Dp(n)` (D7 of `ui-toolkit`).
    DpOfInt,
    /// `n.sp` of an `Int`: the construction `TextUnit(n)`.
    TextUnitOfInt,
}

/// The declared signature of a built-in list member (D5 of `collections`), written with the
/// element type parameter [`BuiltinFunction::list_element_parameter`]: how it takes its
/// receiver, its parameters after the receiver and its result.
#[derive(Debug, Clone, PartialEq)]
pub struct ListMemberSignature {
    pub mode: ReceiverMode,
    pub parameters: Vec<(&'static str, Type)>,
    pub result: Type,
}

impl BuiltinFunction {
    /// The name the built-in is called by.
    pub fn name(self) -> &'static str {
        match self {
            BuiltinFunction::Clone => "clone",
            BuiltinFunction::Print => "print",
            BuiltinFunction::PrintLine => "println",
            BuiltinFunction::StringLength => "length",
            // The name Kotlin gives the `[]` operator.
            BuiltinFunction::StringCharacter => "get",
            BuiltinFunction::ToString => "toString",
            BuiltinFunction::HeapAllocate => "Heap",
            BuiltinFunction::Unbox => "unbox",
            BuiltinFunction::ListOf => "listOf",
            BuiltinFunction::ListSize => "size",
            BuiltinFunction::ListIndices => "indices",
            BuiltinFunction::ListIsEmpty => "isEmpty",
            BuiltinFunction::ListIsNotEmpty => "isNotEmpty",
            BuiltinFunction::ListContains => "contains",
            BuiltinFunction::ListIndexOf => "indexOf",
            BuiltinFunction::ListAdd => "add",
            BuiltinFunction::ListInsert => "insert",
            BuiltinFunction::ListRemoveAt => "removeAt",
            BuiltinFunction::ListRemoveLast => "removeLast",
            BuiltinFunction::ListReverse => "reverse",
            BuiltinFunction::ListClear => "clear",
            BuiltinFunction::Composition => "composition",
            BuiltinFunction::SetContent => "setContent",
            BuiltinFunction::Recompose => "recompose",
            BuiltinFunction::DumpComposition => "dump",
            BuiltinFunction::Remember => "remember",
            BuiltinFunction::Key => "key",
            BuiltinFunction::CurrentRecomposeScope => "currentRecomposeScope",
            BuiltinFunction::InvalidateScope => "invalidate",
            BuiltinFunction::MutableStateOf => "mutableStateOf",
            BuiltinFunction::StateValue => "value",
            BuiltinFunction::StateRead => "read",
            BuiltinFunction::StateUpdate => "update",
            BuiltinFunction::StateEdit => "edit",
            BuiltinFunction::SideEffect => "SideEffect",
            BuiltinFunction::UiNode => "Node",
            BuiltinFunction::UiLeafNode => "LeafNode",
            BuiltinFunction::RunApp => "runApp",
            BuiltinFunction::DpOfInt => "dp",
            BuiltinFunction::TextUnitOfInt => "sp",
        }
    }

    /// The declaration path of an intrinsic whose declaration gives default values, whose
    /// generators a call that omits an argument calls: `runApp` of `/prism/compose/ui` (D5 of
    /// `ui-toolkit`, R8 of `standard-library`).
    pub fn declaration_with_defaults(self) -> Option<crate::DeclarationPath> {
        match self {
            BuiltinFunction::RunApp => Some(crate::DeclarationPath::new(
                crate::types::COMPOSE_UI_MODULE,
                "runApp",
            )),
            _ => None,
        }
    }

    /// Whether the built-in is a state operation of `/prism/compose/runtime`, which MIR
    /// lowering turns into state operations (D17 of `state-handles`).
    pub fn is_state_operation(self) -> bool {
        matches!(
            self,
            BuiltinFunction::MutableStateOf
                | BuiltinFunction::StateValue
                | BuiltinFunction::StateRead
                | BuiltinFunction::StateUpdate
                | BuiltinFunction::StateEdit
        )
    }

    /// The built-in list members, in the order of D5 of `collections`.
    pub const LIST_MEMBERS: [BuiltinFunction; 12] = [
        BuiltinFunction::ListSize,
        BuiltinFunction::ListIndices,
        BuiltinFunction::ListIsEmpty,
        BuiltinFunction::ListIsNotEmpty,
        BuiltinFunction::ListContains,
        BuiltinFunction::ListIndexOf,
        BuiltinFunction::ListAdd,
        BuiltinFunction::ListInsert,
        BuiltinFunction::ListRemoveAt,
        BuiltinFunction::ListRemoveLast,
        BuiltinFunction::ListReverse,
        BuiltinFunction::ListClear,
    ];

    /// The type parameter `T` of the list built-ins' declarations, the element type of
    /// `List<T>` in `fun <T> listOf(...)` and `fun <T> List<T>.add(element: T)`.
    pub fn list_element_parameter() -> crate::types::TypeParameter {
        crate::types::TypeParameter {
            index: 0,
            name: "T".to_string(),
            bound: None,
        }
    }

    /// The declared signature of a list member, `None` for any other built-in (`listOf`
    /// included, whose parameters are as many as its call's arguments).
    pub fn list_member_signature(self) -> Option<ListMemberSignature> {
        let element = Self::list_element_parameter().as_type();
        let borrowed = Type::Reference {
            mutable: false,
            inner: Box::new(element.clone()),
        };
        let (mode, parameters, result) = match self {
            BuiltinFunction::ListSize => (ReceiverMode::Shared, vec![], Type::Int),
            BuiltinFunction::ListIndices => (
                ReceiverMode::Shared,
                vec![],
                Type::plain_struct(NominalId::builtin_range()),
            ),
            BuiltinFunction::ListIsEmpty | BuiltinFunction::ListIsNotEmpty => {
                (ReceiverMode::Shared, vec![], Type::Bool)
            }
            BuiltinFunction::ListContains => (
                ReceiverMode::Shared,
                vec![("element", borrowed)],
                Type::Bool,
            ),
            BuiltinFunction::ListIndexOf => {
                (ReceiverMode::Shared, vec![("element", borrowed)], Type::Int)
            }
            BuiltinFunction::ListAdd => (
                ReceiverMode::Mutable,
                vec![("element", element)],
                Type::Unit,
            ),
            BuiltinFunction::ListInsert => (
                ReceiverMode::Mutable,
                vec![("index", Type::Int), ("element", element)],
                Type::Unit,
            ),
            BuiltinFunction::ListRemoveAt => {
                (ReceiverMode::Mutable, vec![("index", Type::Int)], element)
            }
            BuiltinFunction::ListRemoveLast => (ReceiverMode::Mutable, vec![], element),
            BuiltinFunction::ListReverse | BuiltinFunction::ListClear => {
                (ReceiverMode::Mutable, vec![], Type::Unit)
            }
            _ => return None,
        };
        Some(ListMemberSignature {
            mode,
            parameters,
            result,
        })
    }

    /// The parameters a call of the list member `self` records, as [`Call::parameters`]: `this`
    /// first, of the type its receiver mode passes (`&List<T>` or `&mut List<T>`).
    pub fn list_member_parameters(self) -> Option<Vec<(Option<String>, Type)>> {
        let signature = self.list_member_signature()?;
        let list = Type::List(Box::new(Self::list_element_parameter().as_type()));
        let this = Type::Reference {
            mutable: signature.mode == ReceiverMode::Mutable,
            inner: Box::new(list),
        };
        Some(
            std::iter::once((Some("this".to_string()), this))
                .chain(
                    signature
                        .parameters
                        .into_iter()
                        .map(|(name, ty)| (Some(name.to_string()), ty)),
                )
                .collect(),
        )
    }

    /// Whether the built-in's first argument is a receiver: that of every built-in member
    /// (`clone`, `length`, `get`, `toString`, `unbox`, the list members and the members of
    /// `Composition`, `RecomposeScope` and the state handles), not the arguments of `print`,
    /// `println`, `Heap`, `listOf`, `composition`, `remember`, `key`, `currentRecomposeScope`,
    /// `mutableStateOf` and `SideEffect`.
    pub fn has_receiver(self) -> bool {
        !matches!(
            self,
            BuiltinFunction::Print
                | BuiltinFunction::PrintLine
                | BuiltinFunction::HeapAllocate
                | BuiltinFunction::ListOf
                | BuiltinFunction::Composition
                | BuiltinFunction::Remember
                | BuiltinFunction::Key
                | BuiltinFunction::CurrentRecomposeScope
                | BuiltinFunction::MutableStateOf
                | BuiltinFunction::SideEffect
                | BuiltinFunction::UiNode
                | BuiltinFunction::UiLeafNode
                | BuiltinFunction::RunApp
        )
    }

    /// Whether the built-in is one of `/prism/compose/runtime`'s, which the compose transform
    /// lowers to composer operations (D15 of `compose-transform`).
    pub fn is_compose_runtime(self) -> bool {
        matches!(
            self,
            BuiltinFunction::Composition
                | BuiltinFunction::SetContent
                | BuiltinFunction::Recompose
                | BuiltinFunction::DumpComposition
                | BuiltinFunction::Remember
                | BuiltinFunction::Key
                | BuiltinFunction::CurrentRecomposeScope
                | BuiltinFunction::InvalidateScope
                | BuiltinFunction::SideEffect
                | BuiltinFunction::UiNode
                | BuiltinFunction::UiLeafNode
                | BuiltinFunction::RunApp
        )
    }

    /// The type parameter of [`BuiltinFunction::HeapAllocate`]'s declaration, `T` in
    /// `fun <T> Heap(value: T): Heap<T>`.
    pub fn heap_contents_parameter() -> crate::types::TypeParameter {
        crate::types::TypeParameter {
            index: 0,
            name: "T".to_string(),
            bound: None,
        }
    }
}

/// One argument of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Argument {
    /// A written argument (or the receiver).
    Written(ExpressionId),
    /// An omitted parameter, which takes its default value: a call of its generator with
    /// borrows of the inputs the default reads (D26 of `ui-toolkit`).
    Default(prism_ast::DefaultInputs),
}

/// Why a borrow exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowOrigin {
    /// Written `&e` or `&mut e`.
    Written,
    /// Implied by a reference parameter.
    Argument,
    /// Implied by an extension's receiver mode. Taken after the other arguments are evaluated.
    Receiver,
    /// Made by a nullable conversion (`&(T?)` read as `(&T)?`).
    Conversion,
    /// The borrow of the list a `for` loop iterates, held for the whole loop (D9 of
    /// `collections`).
    Iteration,
}

/// How a `for` loop iterates its iterable (D10 of `collections`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForIteration {
    /// Over a `Range` held by value: the variable is each `Int` in turn.
    Range,
    /// Over a list, held as a borrow of origin [`BorrowOrigin::Iteration`], mutable when
    /// `mutable`: the variable is a `&T` (or `&mut T`) to each element in turn.
    List { mutable: bool },
}

/// How a [`ExpressionKind::Coerce`] converts its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoercionKind {
    /// `T` to `T?`: a representation change.
    WrapNullable,
    /// `Nothing` to any type: the value never exists.
    Diverge,
    /// Any other assignability step: `&mut T` to `&T`, `Nothing?` to `T?`, `T?` to `U?` and
    /// function-type variance. Boxes are invariant (D8 of `heap-boxes`), so no step converts
    /// one `Heap` type to another.
    Subtype,
}

/// What a [`ExpressionKind::SafeNavigation`] was written as or converts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafeNavigationOrigin {
    /// A written `?.`.
    Written,
    /// A borrowed nullable `&(T?)` read as an optional borrow `(&T)?`.
    NullableView,
    /// A `T?` place lent as an optional borrow `(&T)?`.
    BorrowContents,
    /// A `Copy` value read out of an optional borrow (`(&T)?` as `T?`).
    CopyOut,
}

/// What a construction constructs.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstructKind {
    Struct(NominalId),
    Variant {
        enum_id: NominalId,
        index: usize,
        name: String,
    },
}

/// One field of a construction, in declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstructField {
    /// The field's name, `None` for a variant payload.
    pub name: Option<String>,
    /// The field's declared type, which mentions a generic struct's or enum's type parameters;
    /// the value has it with the construction's type arguments substituted.
    pub ty: Type,
    pub value: ExpressionId,
}

/// One branch of a `when`.
#[derive(Debug, Clone, PartialEq)]
pub struct WhenBranch {
    pub patterns: Vec<Pattern>,
    /// The guard `if condition` after the patterns, which sees the branch's bindings (D9 of
    /// `payload-patterns`).
    pub guard: Option<ExpressionId>,
    pub body: ExpressionId,
    pub span: Span,
}

/// A resolved pattern (D14 of `payload-patterns`). The patterns of one branch share one binding
/// per name.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// A variant of the matched enum, with one sub-pattern per payload: none for a payload-free
    /// variant, all wildcards for a payload variant named alone. `arguments` are the enum's type
    /// arguments, from the matched value's type.
    Variant {
        enum_id: NominalId,
        index: usize,
        name: String,
        arguments: Vec<Type>,
        fields: Vec<Pattern>,
    },
    /// The matched struct, with one sub-pattern per field, in declaration order.
    Struct {
        id: NominalId,
        arguments: Vec<Type>,
        fields: Vec<Pattern>,
    },
    /// A nullable that is not `null`, whose payload matches the inner pattern: a constructor
    /// pattern of `T` matching a `T?`.
    NonNull(Box<Pattern>),
    /// A box whose contents match the inner pattern: a constructor, value or range pattern of
    /// `T` matching a `Heap<T>` (D9 of `heap-boxes`). Taking the contents apart unboxes the box.
    /// Also the referent of the borrow a `(&T)?` holds, matched by a constructor or value
    /// pattern of `T`, which binds by the borrow's mode and never takes anything apart.
    Deref(Box<Pattern>),
    /// `_`, or a part no pattern names.
    Wildcard,
    /// A binding of the matched part, by `mode`.
    Binding { local: LocalId, mode: BindingMode },
    /// A value compared with the matched part.
    Value(ExpressionId),
    /// `in start..end` or `in start..<end`.
    Range {
        start: ExpressionId,
        end: ExpressionId,
        end_inclusive: bool,
    },
    /// A condition of a `when` without subject.
    Condition(ExpressionId),
    /// `else`.
    Else,
}

impl Pattern {
    /// Pushes the expressions the pattern holds onto `out`, in source order.
    pub fn expressions(&self, out: &mut Vec<ExpressionId>) {
        match self {
            Pattern::Variant { fields, .. } | Pattern::Struct { fields, .. } => {
                for field in fields {
                    field.expressions(out);
                }
            }
            Pattern::NonNull(inner) | Pattern::Deref(inner) => inner.expressions(out),
            Pattern::Value(value) | Pattern::Condition(value) => out.push(*value),
            Pattern::Range { start, end, .. } => out.extend([*start, *end]),
            Pattern::Wildcard | Pattern::Binding { .. } | Pattern::Else => {}
        }
    }

    /// The bindings of the pattern, in source order, with their modes.
    pub fn bindings(&self) -> Vec<(LocalId, BindingMode)> {
        let mut out = Vec::new();
        self.collect_bindings(&mut out);
        out
    }

    fn collect_bindings(&self, out: &mut Vec<(LocalId, BindingMode)>) {
        match self {
            Pattern::Variant { fields, .. } | Pattern::Struct { fields, .. } => {
                for field in fields {
                    field.collect_bindings(out);
                }
            }
            Pattern::NonNull(inner) | Pattern::Deref(inner) => inner.collect_bindings(out),
            Pattern::Binding { local, mode } => out.push((*local, *mode)),
            _ => {}
        }
    }

    /// Whether a binding of the pattern moves its part, which takes the matched value apart
    /// (D6 of `payload-patterns`).
    pub fn moves(&self) -> bool {
        self.bindings()
            .iter()
            .any(|(_, mode)| *mode == BindingMode::Move)
    }
}

/// How a pattern binding holds its part (D5 of `payload-patterns`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingMode {
    /// A copy of a `Copy` part; the binding has the part's type.
    Copy,
    /// The part, moved out of an owned value taken apart; the binding has the part's type.
    Move,
    /// A shared borrow `&T` of the part.
    SharedBorrow,
    /// A mutable borrow `&mut T` of the part.
    MutableBorrow,
}

impl BindingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            BindingMode::Copy => "copy",
            BindingMode::Move => "move",
            BindingMode::SharedBorrow => "shared borrow",
            BindingMode::MutableBorrow => "mutable borrow",
        }
    }

    /// The type of a binding of a part of type `part` in this mode.
    pub fn binding_type(self, part: &Type) -> Type {
        match self {
            BindingMode::Copy | BindingMode::Move => part.clone(),
            BindingMode::SharedBorrow | BindingMode::MutableBorrow => Type::Reference {
                mutable: self == BindingMode::MutableBorrow,
                inner: Box::new(part.clone()),
            },
        }
    }
}

/// A path of a binding of an enclosing scope that a closure uses (`path-captures`): the binding,
/// then the fields and dereferences in `projections` (empty for the whole binding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub local: LocalId,
    /// The fields and dereferences of the captured path, from the binding outwards: the
    /// binding's own first dereference when it is a `val` reference, then fields and box
    /// dereferences (D1 of `path-captures`).
    pub projections: Vec<CaptureProjection>,
    /// The first use of any path the capture covers.
    pub span: Span,
    /// How the closure takes the path when it is built.
    pub mode: CaptureMode,
    /// For an owned capture of a whole binding that the closure uses only through paths it
    /// cannot copy, the first such path and why (D3, D6 of `path-captures`).
    pub widened: Option<Widening>,
}

/// One step of a captured path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CaptureProjection {
    /// Field `index` of a struct.
    Field(usize),
    /// The referent of a reference or the contents of a box.
    Deref,
}

/// Why an owning closure took a whole binding for a path it reads (D3 of `path-captures`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widening {
    /// The first path the closure could not copy.
    pub projections: Vec<CaptureProjection>,
    pub reason: WideningReason,
}

/// Why an owning closure cannot copy a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WideningReason {
    /// The path's type is not `Copy`.
    NotCopy,
    /// The path's binding is a `var`.
    VariableBinding,
    /// The path dereferences a `&mut` reference, whose referent may be replaced.
    MutableReference,
    /// The path reads a `var` field through a reference.
    VariableField,
}

impl WideningReason {
    pub fn as_str(self) -> &'static str {
        match self {
            WideningReason::NotCopy => "not Copy",
            WideningReason::VariableBinding => "a var binding",
            WideningReason::MutableReference => "through a mutable reference",
            WideningReason::VariableField => "a var field through a reference",
        }
    }
}

impl Capture {
    /// The capture of the whole binding `local`.
    pub fn whole(local: LocalId, span: Span, mode: CaptureMode) -> Self {
        Self {
            local,
            projections: Vec::new(),
            span,
            mode,
            widened: None,
        }
    }

    /// The type of the captured path, for a binding of type `binding`: `None` when a step does
    /// not apply to the type it is taken of.
    pub fn path_type(&self, binding: &Type, types: &TypeTable) -> Option<Type> {
        path_type(binding, &self.projections, types)
    }

    /// The captured path as source writes it, for a binding named `name` of type `binding`.
    pub fn path_text(&self, name: &str, binding: &Type, types: &TypeTable) -> String {
        path_text(name, binding, &self.projections, types)
    }
}

/// The type of the path `projections` of a binding of type `binding`.
pub fn path_type(
    binding: &Type,
    projections: &[CaptureProjection],
    types: &TypeTable,
) -> Option<Type> {
    let mut ty = binding.clone();
    for projection in projections {
        ty = match (projection, ty) {
            (CaptureProjection::Field(index), Type::Struct { id, arguments }) => {
                types.instantiated_field(&id, &arguments, *index)?
            }
            (CaptureProjection::Deref, Type::Reference { inner, .. } | Type::Heap(inner)) => {
                *inner
            }
            _ => return None,
        };
    }
    Some(ty)
}

/// The path `projections` of a binding named `name` of type `binding` as source writes it: the
/// binding and its fields joined by `.`, the dereferences between them implied by the types, and
/// a `*` for each dereference after the last field (`p.x`, `todo.done`, `*r`).
pub fn path_text(
    name: &str,
    binding: &Type,
    projections: &[CaptureProjection],
    types: &TypeTable,
) -> String {
    let mut text = name.to_string();
    let mut trailing = 0;
    let mut ty = Some(binding.clone());
    for projection in projections {
        match projection {
            CaptureProjection::Deref => trailing += 1,
            CaptureProjection::Field(index) => {
                trailing = 0;
                let field = match &ty {
                    Some(Type::Struct { id, .. }) => types
                        .struct_fields(id)
                        .and_then(|fields| fields.get(*index))
                        .map(|(name, _, _)| name.clone()),
                    _ => None,
                };
                text.push('.');
                text.push_str(&field.unwrap_or_else(|| index.to_string()));
            }
        }
        ty = ty.and_then(|ty| path_type(&ty, std::slice::from_ref(projection), types));
    }
    format!("{}{}", "*".repeat(trailing), text)
}

/// How a closure takes a captured binding (D2 of `escaping-closures`). Whether a borrow is
/// shared or mutable is not decided here: the borrow checker infers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// The closure borrows the binding for as long as it is live.
    Borrowed,
    /// The closure owns the binding, moved into it where the closure is built.
    Moved,
    /// The closure owns a copy of the binding, whose type is `Copy`.
    Copied,
}

impl CaptureMode {
    /// Whether the closure owns what it captures.
    pub fn is_owned(self) -> bool {
        !matches!(self, CaptureMode::Borrowed)
    }
}

/// Why a closure owns some of its captures (D1 of `escaping-closures`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipReason {
    /// Where the closure's value escapes, or is stored outside a captured binding's block.
    pub span: Span,
    pub kind: OwnershipReasonKind,
}

/// The position that makes a closure own its captures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnershipReasonKind {
    /// The result of a function, closure or default value, or the operand of a `return`.
    Returned,
    /// The initializer of a top-level `val`.
    TopLevelValue,
    /// A field argument of a struct construction, or the value assigned to a field.
    StoredInField,
    /// A payload argument of an enum variant.
    StoredInPayload,
    /// The argument of `Heap(...)`.
    StoredInBox,
    /// The value written through a reference or a box.
    WrittenThroughReference,
    /// The value assigned to a variable that the enclosing closure captures.
    AssignedToCapturedVariable,
    /// An argument for a parameter whose type has a type parameter outside a function type.
    PassedToGenericParameter,
    /// An argument for an `escaping` parameter.
    PassedToEscapingParameter,
    /// A capture of a closure that owns its captures.
    CapturedByOwningClosure,
    /// A store into a binding declared outside the block of the captured binding `name`; the
    /// closure owns only the captures it outlives this way.
    StoredOutsideBlock { name: String },
    /// The value assigned to a state's `value` (D18 of `state-handles`), which the cell keeps.
    StoredInState,
}

impl OwnershipReason {
    /// The label of the reason in a diagnostic: "the closure owns its captures because it is
    /// returned here", or "the closure is stored here, in a variable that outlives 's'".
    pub fn label(&self) -> String {
        let position = match &self.kind {
            OwnershipReasonKind::StoredOutsideBlock { name } => {
                return format!(
                    "the closure is stored here, in a variable that outlives '{}'",
                    name
                )
            }
            OwnershipReasonKind::Returned => "returned here",
            OwnershipReasonKind::TopLevelValue => "stored in a top-level value here",
            OwnershipReasonKind::StoredInField => "stored in a field here",
            OwnershipReasonKind::StoredInPayload => "stored in an enum payload here",
            OwnershipReasonKind::StoredInBox => "stored in a box here",
            OwnershipReasonKind::StoredInState => "stored in a state here",
            OwnershipReasonKind::WrittenThroughReference => "written through a reference here",
            OwnershipReasonKind::AssignedToCapturedVariable => {
                "assigned to a captured variable here"
            }
            OwnershipReasonKind::PassedToGenericParameter => "passed to a generic parameter here",
            OwnershipReasonKind::PassedToEscapingParameter => {
                "passed to an escaping parameter here"
            }
            OwnershipReasonKind::CapturedByOwningClosure => {
                "captured here by a closure that owns its captures"
            }
        };
        format!("the closure owns its captures because it is {}", position)
    }

    /// The short form the typed-tree dump prints: "returned here", "stored outside 's'".
    pub fn short(&self) -> String {
        match &self.kind {
            OwnershipReasonKind::StoredOutsideBlock { name } => {
                format!("stored outside the block of '{}'", name)
            }
            _ => self
                .label()
                .trim_start_matches("the closure owns its captures because it is ")
                .to_string(),
        }
    }
}

/// An arithmetic or comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperator {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

impl BinaryOperator {
    pub fn as_str(self) -> &'static str {
        match self {
            BinaryOperator::Add => "+",
            BinaryOperator::Subtract => "-",
            BinaryOperator::Multiply => "*",
            BinaryOperator::Divide => "/",
            BinaryOperator::Remainder => "%",
            BinaryOperator::Equal => "==",
            BinaryOperator::NotEqual => "!=",
            BinaryOperator::Less => "<",
            BinaryOperator::LessOrEqual => "<=",
            BinaryOperator::Greater => ">",
            BinaryOperator::GreaterOrEqual => ">=",
        }
    }

    /// Whether the operator is arithmetic (`+`, `-`, `*`, `/`, `%`), whose result has its
    /// operands' type, rather than a comparison.
    pub fn is_arithmetic(self) -> bool {
        matches!(
            self,
            BinaryOperator::Add
                | BinaryOperator::Subtract
                | BinaryOperator::Multiply
                | BinaryOperator::Divide
                | BinaryOperator::Remainder
        )
    }
}

/// A unary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOperator {
    Negate,
    Not,
}

impl UnaryOperator {
    pub fn as_str(self) -> &'static str {
        match self {
            UnaryOperator::Negate => "-",
            UnaryOperator::Not => "!",
        }
    }
}

/// A short-circuiting operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOperator {
    And,
    Or,
}

impl LogicalOperator {
    pub fn as_str(self) -> &'static str {
        match self {
            LogicalOperator::And => "&&",
            LogicalOperator::Or => "||",
        }
    }
}

/// A statement node.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
}

/// What a statement is.
#[derive(Debug, Clone, PartialEq)]
pub enum StatementKind {
    /// A local `val` or `var`, initialized with a consumed value.
    Let {
        local: LocalId,
        initializer: ExpressionId,
    },
    /// An assignment to a place (D10): a binding, a field path, or a dereference for a write
    /// through a reference. The value is consumed.
    Assign {
        target: ExpressionId,
        value: ExpressionId,
    },
    /// A compound assignment `target op= value` (D4 of `compound-assignment`): the target is a
    /// place, read in place and then written with `target op value`; the value is an operand of
    /// the target's type, read in place when it is a place and never consumed. `operator` is
    /// arithmetic (`Add` to `Remainder`).
    CompoundAssign {
        operator: BinaryOperator,
        target: ExpressionId,
        value: ExpressionId,
    },
    /// An expression whose value is discarded.
    Expression { value: ExpressionId },
    While {
        condition: ExpressionId,
        body: ExpressionId,
    },
    /// The condition is evaluated in the body block's scope, so the body's locals live through
    /// it.
    DoWhile {
        body: ExpressionId,
        condition: ExpressionId,
    },
    /// `for (variable in iterable) body`, iterating as `iteration` says: a `Range` value, or a
    /// borrow of a list (D10 of `collections`).
    For {
        variable: LocalId,
        iterable: ExpressionId,
        iteration: ForIteration,
        body: ExpressionId,
    },
    /// A destructuring declaration `val (a, b) = value` (D14 of `payload-patterns`): `value` read
    /// in place, and a [`Pattern::Struct`] of its struct binding the named fields, its other
    /// fields wildcards.
    Destructure {
        pattern: Pattern,
        value: ExpressionId,
    },
}

impl StatementKind {
    /// The expression children of the statement, in evaluation order of the source.
    pub fn children(&self) -> Vec<ExpressionId> {
        match self {
            StatementKind::Let { initializer, .. } => vec![*initializer],
            StatementKind::Assign { target, value }
            | StatementKind::CompoundAssign { target, value, .. } => vec![*target, *value],
            StatementKind::Expression { value } => vec![*value],
            StatementKind::While { condition, body } => vec![*condition, *body],
            StatementKind::DoWhile { body, condition } => vec![*body, *condition],
            StatementKind::For { iterable, body, .. } => vec![*iterable, *body],
            StatementKind::Destructure { value, .. } => vec![*value],
        }
    }

    /// Whether the statement is a loop, which a `break` or `continue` can name.
    pub fn is_loop(&self) -> bool {
        matches!(
            self,
            StatementKind::While { .. } | StatementKind::DoWhile { .. } | StatementKind::For { .. }
        )
    }
}
