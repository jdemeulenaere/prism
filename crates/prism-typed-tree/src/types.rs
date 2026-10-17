//! The semantic type representation shared by the type checker and the typed tree.
//!
//! The rules over these types (lowering from annotations and ABIs, assignability, joins) live in
//! the type checker; this module holds only the representation and its rendering.

use std::fmt;

/// Module path of the compiler's built-in nominal types (such as `Range`).
pub const BUILTIN_MODULE: &str = "<builtin>";

/// Canonical identity of a struct or enum: its defining module (the root-relative file path
/// without extension, such as `a/lib`) and its name. Two nominal types are equal exactly when
/// their identities are equal.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NominalId {
    pub module: String,
    pub name: String,
}

impl NominalId {
    pub fn new(module: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            module: module.into(),
            name: name.into(),
        }
    }

    /// The built-in `Range` type produced by `a..b`.
    pub fn builtin_range() -> Self {
        Self::new(BUILTIN_MODULE, "Range")
    }
}

/// The definition behind a [`NominalId`], kept in the checker's type table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDef {
    /// Struct fields as `(name, is_var, type)`.
    Struct { fields: Vec<(String, bool, Type)> },
    /// Enum variants as `(name, payload types)`.
    Enum { variants: Vec<(String, Vec<Type>)> },
}

/// Semantic representation of a Prism type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    /// 64-bit signed integer.
    Int,
    /// 64-bit IEEE floating point.
    Float,
    /// Boolean (`true` or `false`).
    Bool,
    /// Unicode scalar value character literal (`'a'`).
    Char,
    /// UTF-8 string.
    String,
    /// Unit type `()`, equivalent to `void`.
    Unit,
    /// Bottom type `Nothing`, representing expressions that diverge or return.
    Nothing,
    /// Stack-scoped borrowed reference (`&T` or `&mut T`).
    Reference { mutable: bool, inner: Box<Type> },
    /// Nullable type `T?`.
    Nullable(Box<Type>),
    /// Single-owner heap boxed indirection `Heap<T>` for recursive types.
    Heap(Box<Type>),
    /// Nominal struct type. Its fields live in the checker's type table.
    Struct(NominalId),
    /// Nominal enum type. Its variants live in the checker's type table.
    Enum(NominalId),
    /// A type name with no struct or enum declaration in scope. Opaque types compare by name,
    /// and have no members.
    Opaque(String),
    /// First-class function type.
    Fn {
        is_compose: bool,
        params: Vec<Type>,
        ret: Box<Type>,
    },
    /// Generic type parameter (e.g. `T`).
    TypeParam(String),
    /// Sentinel type for error recovery and downstream error suppression.
    Error,
}

impl Type {
    /// Makes a type nullable. `Error` stays `Error`, and nullable types never nest (`T??` is
    /// `T?`).
    pub fn nullable(self) -> Type {
        match self {
            Type::Error | Type::Nullable(_) => self,
            other => Type::Nullable(Box::new(other)),
        }
    }

    /// Rewrites every opaque type name inside `self` with `bind`, keeping names it returns
    /// `None` for. Used to bind names lowered from annotations or ABI types to the struct or enum
    /// they denote.
    pub fn bind_names(self, bind: &dyn Fn(&str) -> Option<Type>) -> Type {
        match self {
            Type::Opaque(name) => bind(&name).unwrap_or(Type::Opaque(name)),
            Type::Reference { mutable, inner } => Type::Reference {
                mutable,
                inner: Box::new(inner.bind_names(bind)),
            },
            Type::Nullable(inner) => Type::Nullable(Box::new(inner.bind_names(bind))),
            Type::Heap(inner) => Type::Heap(Box::new(inner.bind_names(bind))),
            Type::Fn {
                is_compose,
                params,
                ret,
            } => Type::Fn {
                is_compose,
                params: params.into_iter().map(|p| p.bind_names(bind)).collect(),
                ret: Box::new(ret.bind_names(bind)),
            },
            other => other,
        }
    }
}

impl Type {
    /// Renders `expected` and `found` for a diagnostic. Nominal types show their bare names,
    /// unless both render the same, in which case each nominal type is qualified with its
    /// defining module (`a/lib::User` versus `b/lib::User`).
    pub fn render_pair(expected: &Type, found: &Type) -> (String, String) {
        let (e, f) = (expected.to_string(), found.to_string());
        if e == f && expected != found {
            (expected.qualified(), found.qualified())
        } else {
            (e, f)
        }
    }

    /// Renders the type with every nominal type qualified by its defining module.
    pub fn qualified(&self) -> String {
        match self {
            Type::Struct(id) | Type::Enum(id) => format!("{}::{}", id.module, id.name),
            Type::Reference { mutable, inner } => {
                format!(
                    "{}{}",
                    if *mutable { "&mut " } else { "&" },
                    inner.qualified()
                )
            }
            Type::Nullable(inner) => format!("{}?", inner.qualified()),
            Type::Heap(inner) => format!("Heap<{}>", inner.qualified()),
            Type::Fn {
                is_compose,
                params,
                ret,
            } => format!(
                "{}({}) -> {}",
                if *is_compose { "compose " } else { "" },
                params
                    .iter()
                    .map(Type::qualified)
                    .collect::<Vec<_>>()
                    .join(", "),
                ret.qualified()
            ),
            other => other.to_string(),
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Int => write!(f, "Int"),
            Type::Float => write!(f, "Float"),
            Type::Bool => write!(f, "Bool"),
            Type::Char => write!(f, "Char"),
            Type::String => write!(f, "String"),
            Type::Unit => write!(f, "Unit"),
            Type::Nothing => write!(f, "Nothing"),
            Type::Reference { mutable, inner } => {
                if *mutable {
                    write!(f, "&mut {}", inner)
                } else {
                    write!(f, "&{}", inner)
                }
            }
            Type::Nullable(inner) => write!(f, "{}?", inner),
            Type::Heap(inner) => write!(f, "Heap<{}>", inner),
            Type::Struct(id) | Type::Enum(id) => write!(f, "{}", id.name),
            Type::Opaque(name) => write!(f, "{}", name),
            Type::Fn {
                is_compose,
                params,
                ret,
            } => {
                let comp = if *is_compose { "compose " } else { "" };
                write!(f, "{}(", comp)?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") -> {}", ret)
            }
            Type::TypeParam(name) => write!(f, "{}", name),
            Type::Error => write!(f, "<error>"),
        }
    }
}
