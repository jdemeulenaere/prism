//! Declarations in Prism AST.

use crate::expr::Expr;
use crate::span::Span;
use crate::statement::Block;
use crate::types::Type;
use std::fmt;

/// Declaration visibility modifier.
///
/// Single-file compilation unit invariant: default visibility is `private`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Visibility {
    /// Accessible only within the declaring source file (default when no keyword is provided).
    #[default]
    Private,
    /// Accessible to any file in the same package directory (`package`).
    Package,
    /// Accessible across any package via imports (`public`).
    Public,
}

impl Visibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Package => "package",
            Visibility::Public => "public",
        }
    }
}

impl fmt::Display for Visibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Package declaration at the top of a file (e.g. `package demo.math`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PackageDecl {
    pub path: Vec<String>,
    pub span: Span,
}

impl PackageDecl {
    pub fn package_string(&self) -> String {
        self.path.join(".")
    }
}

/// Import declaration (e.g. `import /core/fmt` or `import doo`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImportDecl {
    /// True if anchored to workspace root (`/`), e.g. `/core/fmt`.
    pub is_root: bool,
    /// The import path components, e.g. `["core", "fmt"]` or `["doo"]`.
    pub path: Vec<String>,
    pub span: Span,
}

impl ImportDecl {
    pub fn path_string(&self) -> String {
        if self.is_root {
            format!("/{}", self.path.join("/"))
        } else {
            self.path.join("/")
        }
    }
}

/// A function parameter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    /// Whether the parameter is written `escaping`: its function value may be kept by the callee.
    pub escaping: bool,
    pub default_value: Option<Expr>,
    pub span: Span,
}

/// A function body: either a block `{ ... }` or single expression `= expr`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FunctionBody {
    Block(Block),
    Expr(Expr),
}

/// How an extension function receives its receiver, written as a modifier before `fun`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReceiverMode {
    /// `fun T.name()`: the receiver is borrowed shared, and `this` is a `&T`.
    #[default]
    Shared,
    /// `mut fun T.name()`: the receiver is borrowed mutably, and `this` is a `&mut T`.
    Mutable,
    /// `consuming fun T.name()`: the receiver is moved in, and `this` is a `T`.
    Consuming,
}

impl ReceiverMode {
    /// The modifier written before `fun`, empty for the shared mode.
    pub fn as_str(self) -> &'static str {
        match self {
            ReceiverMode::Shared => "",
            ReceiverMode::Mutable => "mut",
            ReceiverMode::Consuming => "consuming",
        }
    }
}

/// The receiver of an extension function (`User` and its mode in `mut fun User.rename()`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Receiver {
    pub ty: Type,
    pub mode: ReceiverMode,
    pub span: Span,
}

/// The context parameter of a function, `scope: RowScope` in
/// `context(scope: RowScope) fun Modifier.weight()`: the function is callable only where a
/// candidate of that type is in context, and `name` is a shared borrow of it in the body.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContextParameter {
    pub name: String,
    pub ty: Type,
    /// From `context` to the closing `)`.
    pub span: Span,
}

/// A type parameter of a function, struct or enum (`T` or `T : Copy` in `fun <T : Copy> f()`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeParameter {
    pub name: String,
    /// The marker bounding the parameter (`: Copy` or `: Clone`); only function type parameters
    /// may have one.
    pub bound: Option<TypeMarker>,
    pub span: Span,
}

/// What a library-only modifier says the compiler provides for a body-less declaration (D5,
/// D6 and D12 of `standard-library`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LibraryKind {
    /// `intrinsic`: calls are typed by the declaration and lowered by the compiler.
    Intrinsic,
    /// `external`: a runtime entry point, called by the C symbol equal to the name.
    External,
}

impl LibraryKind {
    /// The modifier as written.
    pub fn as_str(self) -> &'static str {
        match self {
            LibraryKind::Intrinsic => "intrinsic",
            LibraryKind::External => "external",
        }
    }
}

/// An `external` or `intrinsic` modifier, with its span, where diagnostics about it point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LibraryModifier {
    pub kind: LibraryKind,
    pub span: Span,
}

/// Function declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionDecl {
    /// The context parameter written before the declaration's modifiers
    /// (`context(scope: RowScope) fun Modifier.weight()`), at most one.
    pub context: Option<ContextParameter>,
    pub visibility: Visibility,
    /// Native `compose` modifier (`compose fun`) per the first-class compose invariant.
    pub is_compose: bool,
    /// The receiver of an extension function (`fun T.name()`), the only kind of method; `None`
    /// for a plain function.
    pub receiver: Option<Receiver>,
    pub name: String,
    pub type_parameters: Vec<TypeParameter>,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    /// The body; `None` exactly when `library` is set (the parser rejects every other case).
    pub body: Option<FunctionBody>,
    /// The `external` or `intrinsic` modifier of a body-less library declaration.
    pub library: Option<LibraryModifier>,
    pub span: Span,
}

impl FunctionDecl {
    /// Whether the declaration is `intrinsic`.
    pub fn is_intrinsic(&self) -> bool {
        matches!(
            self.library,
            Some(LibraryModifier {
                kind: LibraryKind::Intrinsic,
                ..
            })
        )
    }

    /// Whether the declaration is `external`.
    pub fn is_external(&self) -> bool {
        matches!(
            self.library,
            Some(LibraryModifier {
                kind: LibraryKind::External,
                ..
            })
        )
    }
}

/// Struct field declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructField {
    pub is_var: bool, // true for `var`, false for `val`
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

/// The marker written after a struct or enum's name (`struct Point : Copy`): how its values may
/// be duplicated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeMarker {
    /// Duplicated implicitly by a bitwise copy. Implies `Clone`.
    Copy,
    /// Duplicated only by an explicit `.clone()`.
    Clone,
}

impl TypeMarker {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeMarker::Copy => "Copy",
            TypeMarker::Clone => "Clone",
        }
    }
}

/// Struct declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StructDecl {
    pub visibility: Visibility,
    pub name: String,
    /// The span of the name, where diagnostics about the declaration as a whole point.
    pub name_span: Span,
    pub type_parameters: Vec<TypeParameter>,
    pub marker: Option<TypeMarker>,
    pub fields: Vec<StructField>,
    pub span: Span,
}

/// Enum variant payload.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<Type>,
    pub span: Span,
}

/// Enum / Algebraic Data Type declaration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EnumDecl {
    pub visibility: Visibility,
    pub name: String,
    /// The span of the name, where diagnostics about the declaration as a whole point.
    pub name_span: Span,
    pub type_parameters: Vec<TypeParameter>,
    pub marker: Option<TypeMarker>,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

/// Top-level variable declaration. It is always a read-only `val`: a top-level `var` is rejected
/// by the parser, because top-level state is immutable.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TopLevelVarDecl {
    pub visibility: Visibility,
    pub name: String,
    pub ty: Option<Type>,
    pub initializer: Expr,
    pub span: Span,
}

/// An intrinsic extension property, `intrinsic val <T> List<T>.size: Int` (D6 of
/// `standard-library`): read without parentheses, typed by its declaration and lowered by the
/// compiler. Only the bundled library declares them, and only `intrinsic`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtensionPropertyDecl {
    pub visibility: Visibility,
    /// The `intrinsic` modifier.
    pub library: LibraryModifier,
    pub type_parameters: Vec<TypeParameter>,
    /// The receiver type (`List<T>`).
    pub receiver: Type,
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

/// Top-level declarations in a Prism file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Declaration {
    Function(FunctionDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Var(TopLevelVarDecl),
    ExtensionProperty(ExtensionPropertyDecl),
}

impl Declaration {
    pub fn visibility(&self) -> Visibility {
        match self {
            Declaration::Function(f) => f.visibility,
            Declaration::Struct(s) => s.visibility,
            Declaration::Enum(e) => e.visibility,
            Declaration::Var(v) => v.visibility,
            Declaration::ExtensionProperty(p) => p.visibility,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Declaration::Function(f) => &f.name,
            Declaration::Struct(s) => &s.name,
            Declaration::Enum(e) => &e.name,
            Declaration::Var(v) => &v.name,
            Declaration::ExtensionProperty(p) => &p.name,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Declaration::Function(f) => f.span,
            Declaration::Struct(s) => s.span,
            Declaration::Enum(e) => e.span,
            Declaration::Var(v) => v.span,
            Declaration::ExtensionProperty(p) => p.span,
        }
    }

    /// The `external` or `intrinsic` modifier of a library declaration, if it has one.
    pub fn library_modifier(&self) -> Option<LibraryModifier> {
        match self {
            Declaration::Function(f) => f.library,
            Declaration::ExtensionProperty(p) => Some(p.library),
            Declaration::Struct(_) | Declaration::Enum(_) | Declaration::Var(_) => None,
        }
    }
}
