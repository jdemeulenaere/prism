//! Semantic type representation and subtyping rules for Prism.

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
    /// Checks whether `self` is assignable to `expected` according to Prism's subtyping lattice:
    /// - `Error` unifies with any type (suppresses cascade errors)
    /// - `Nothing <= T` for all `T` (divergent expressions can be placed in any value branch)
    /// - `T <= T?` for all `T` (non-null value is assignable to nullable)
    /// - `&mut T <= &T` (mutable borrow can coerce to shared borrow at call sites)
    /// - Structs and Enums are nominal (matched by [`NominalId`])
    /// - All other types are strictly invariant (e.g. `Int` does NOT widen to `Float`)
    pub fn is_assignable_to(&self, expected: &Type) -> bool {
        if self == &Type::Error || expected == &Type::Error {
            return true;
        }

        if self == &Type::Nothing {
            return true;
        }

        if self == expected {
            return true;
        }

        // T <= T?
        if let Type::Nullable(inner) = expected {
            if self.is_assignable_to(inner) {
                return true;
            }
        }

        // T? <= U? if T <= U
        if let (Type::Nullable(self_inner), Type::Nullable(exp_inner)) = (self, expected) {
            if self_inner.is_assignable_to(exp_inner) {
                return true;
            }
        }

        // Heap<T> <= Heap<U> if T <= U
        if let (Type::Heap(self_inner), Type::Heap(exp_inner)) = (self, expected) {
            if self_inner.is_assignable_to(exp_inner) {
                return true;
            }
        }

        // &T <= &U if T <= U; &mut T <= &mut U only if T and U are mutually assignable,
        // because a value of type U could be written back through the borrow.
        if let (
            Type::Reference {
                mutable: self_mut,
                inner: self_inner,
            },
            Type::Reference {
                mutable: exp_mut,
                inner: exp_inner,
            },
        ) = (self, expected)
        {
            if self_mut == exp_mut
                && self_inner.is_assignable_to(exp_inner)
                && (!*exp_mut || exp_inner.is_assignable_to(self_inner))
            {
                return true;
            }
        }

        // &mut T <= &T
        if let (
            Type::Reference {
                mutable: true,
                inner: self_inner,
            },
            Type::Reference {
                mutable: false,
                inner: exp_inner,
            },
        ) = (self, expected)
        {
            if self_inner.is_assignable_to(exp_inner) {
                return true;
            }
        }

        // Function subtyping: matching compose flag, contravariant params, covariant return
        if let (
            Type::Fn {
                is_compose: self_comp,
                params: self_params,
                ret: self_ret,
            },
            Type::Fn {
                is_compose: exp_comp,
                params: exp_params,
                ret: exp_ret,
            },
        ) = (self, expected)
        {
            if self_comp == exp_comp
                && self_params.len() == exp_params.len()
                && exp_params
                    .iter()
                    .zip(self_params.iter())
                    .all(|(exp_p, self_p)| exp_p.is_assignable_to(self_p))
                && self_ret.is_assignable_to(exp_ret)
            {
                return true;
            }
        }

        false
    }

    /// Joins the types of two branches (`if`/`else`, `when`) into the least type both are
    /// assignable to, or `None` if they have no common type.
    ///
    /// `Nothing` joins to the other side, and a nullable side makes the join nullable, so
    /// `Int` joined with `null` (`Nothing?`) is `Int?`.
    pub fn join(&self, other: &Type) -> Option<Type> {
        if self == &Type::Error || other == &Type::Error {
            return Some(Type::Error);
        }
        if other.is_assignable_to(self) {
            return Some(self.clone());
        }
        if self.is_assignable_to(other) {
            return Some(other.clone());
        }
        match (self, other) {
            (Type::Nullable(inner), _) => inner.join(other).map(Type::nullable),
            (_, Type::Nullable(inner)) => self.join(inner).map(Type::nullable),
            _ => None,
        }
    }

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

    /// Converts an AST type to a semantic `Type`. Nominal names stay [`Type::Opaque`] until the
    /// checker binds them in scope.
    pub fn from_ast(ast_ty: &prism_ast::types::Type) -> Self {
        match &ast_ty.kind {
            prism_ast::types::TypeKind::Named { name, type_args } => match name.as_str() {
                "Int" => Type::Int,
                "Float" | "Double" => Type::Float,
                "Bool" => Type::Bool,
                "Char" => Type::Char,
                "String" => Type::String,
                "Unit" => Type::Unit,
                "Nothing" => Type::Nothing,
                "Heap" if type_args.len() == 1 => {
                    Type::Heap(Box::new(Type::from_ast(&type_args[0])))
                }
                // Nominal type or generic type parameter, bound by the checker's scope.
                _ => Type::Opaque(name.clone()),
            },
            prism_ast::types::TypeKind::Nullable(inner) => {
                Type::Nullable(Box::new(Type::from_ast(inner)))
            }
            prism_ast::types::TypeKind::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(Type::from_ast(inner)),
            },
            prism_ast::types::TypeKind::Unit => Type::Unit,
            prism_ast::types::TypeKind::Function {
                is_compose,
                params,
                return_type,
            } => Type::Fn {
                is_compose: *is_compose,
                params: params.iter().map(Type::from_ast).collect(),
                ret: Box::new(Type::from_ast(return_type)),
            },
        }
    }

    /// Parses a canonical type string into a semantic `Type`.
    pub fn parse_str(s: &str) -> Self {
        let s = s.trim();
        if let Some(stripped) = s.strip_suffix('?') {
            let inner = Self::parse_str(stripped);
            return Type::Nullable(Box::new(inner));
        }
        if let Some(stripped) = s.strip_prefix("&mut ") {
            let inner = Self::parse_str(stripped);
            return Type::Reference {
                mutable: true,
                inner: Box::new(inner),
            };
        }
        if let Some(stripped) = s.strip_prefix('&') {
            let inner = Self::parse_str(stripped);
            return Type::Reference {
                mutable: false,
                inner: Box::new(inner),
            };
        }
        if s.starts_with("Heap<") && s.ends_with('>') {
            let inner = Self::parse_str(&s[5..s.len() - 1]);
            return Type::Heap(Box::new(inner));
        }
        if s.starts_with("compose (") || s.starts_with('(') {
            let is_compose = s.starts_with("compose ");
            let s_no_comp = if is_compose { &s[8..] } else { s };
            if let Some(arrow_idx) = s_no_comp.find(") -> ") {
                let params_str = &s_no_comp[1..arrow_idx];
                let ret_str = &s_no_comp[arrow_idx + 5..];
                let params = if params_str.trim().is_empty() {
                    Vec::new()
                } else {
                    params_str.split(',').map(Self::parse_str).collect()
                };
                let ret = Self::parse_str(ret_str);
                return Type::Fn {
                    is_compose,
                    params,
                    ret: Box::new(ret),
                };
            }
        }
        match s {
            "Int" => Type::Int,
            "Float" | "Double" => Type::Float,
            "Bool" => Type::Bool,
            "Char" => Type::Char,
            "String" => Type::String,
            "Unit" | "()" => Type::Unit,
            "Nothing" => Type::Nothing,
            other => Type::Opaque(other.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_primitive_subtyping_and_immutability() {
        assert!(Type::Int.is_assignable_to(&Type::Int));
        assert!(Type::Char.is_assignable_to(&Type::Char));
        // No implicit widening
        assert!(!Type::Int.is_assignable_to(&Type::Float));
        assert!(!Type::Float.is_assignable_to(&Type::Int));
        assert!(!Type::String.is_assignable_to(&Type::Int));
    }

    #[test]
    fn test_nothing_bottom_type_subtyping() {
        assert!(Type::Nothing.is_assignable_to(&Type::Int));
        assert!(Type::Nothing.is_assignable_to(&Type::String));
        assert!(Type::Nothing.is_assignable_to(&Type::Unit));
        assert!(Type::Nothing.is_assignable_to(&Type::Nullable(Box::new(Type::Int))));
        // Nothing is not a supertype
        assert!(!Type::Int.is_assignable_to(&Type::Nothing));
    }

    #[test]
    fn test_nullable_subtyping() {
        let opt_int = Type::Nullable(Box::new(Type::Int));
        assert!(Type::Int.is_assignable_to(&opt_int));
        assert!(!opt_int.is_assignable_to(&Type::Int));
        assert!(!Type::String.is_assignable_to(&opt_int));
    }

    #[test]
    fn test_reference_subtyping() {
        let r_mut = Type::Reference {
            mutable: true,
            inner: Box::new(Type::Int),
        };
        let r_shared = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Int),
        };
        // &mut Int can coerce to &Int
        assert!(r_mut.is_assignable_to(&r_shared));
        // &Int cannot coerce to &mut Int
        assert!(!r_shared.is_assignable_to(&r_mut));
    }

    #[test]
    fn test_nominal_identity_equality() {
        let a_user = Type::Struct(NominalId::new("a/lib", "User"));
        let b_user = Type::Struct(NominalId::new("b/lib", "User"));
        let a_enum = Type::Enum(NominalId::new("a/lib", "User"));
        assert!(a_user.is_assignable_to(&a_user.clone()));
        assert!(!a_user.is_assignable_to(&b_user));
        assert!(!a_user.is_assignable_to(&a_enum));
        assert!(!a_enum.is_assignable_to(&a_user));
        assert!(Type::Opaque("T".into()).is_assignable_to(&Type::Opaque("T".into())));
        assert!(!Type::Opaque("User".into()).is_assignable_to(&a_user));
    }

    #[test]
    fn test_error_sentinel_unification() {
        assert!(Type::Error.is_assignable_to(&Type::Int));
        assert!(Type::Int.is_assignable_to(&Type::Error));
        assert!(Type::Error.is_assignable_to(&Type::Error));
    }
}
