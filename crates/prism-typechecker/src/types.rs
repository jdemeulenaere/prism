//! Semantic type representation and subtyping rules for Prism.

use std::fmt;

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
    /// User-defined nominal struct type.
    Struct {
        name: String,
        fields: Vec<(String, bool, Type)>, // (name, is_var, type)
    },
    /// User-defined nominal enum type.
    Enum {
        name: String,
        variants: Vec<(String, Vec<Type>)>, // (name, payload types)
    },
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
    /// - Structs and Enums are nominal (matched by name)
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

        // Nominal Structs / Enums match by name (including nominal annotation conversions)
        match (self, expected) {
            (Type::Struct { name: n1, .. }, Type::Struct { name: n2, .. }) => n1 == n2,
            (Type::Enum { name: n1, .. }, Type::Enum { name: n2, .. }) => n1 == n2,
            (Type::Enum { name: n1, .. }, Type::Struct { name: n2, .. }) => n1 == n2,
            (Type::Struct { name: n1, .. }, Type::Enum { name: n2, .. }) => n1 == n2,
            _ => false,
        }
    }

    /// Converts an AST type to a semantic `Type`.
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
                _ => {
                    // Nominal type or generic type parameter
                    Type::Struct {
                        name: name.clone(),
                        fields: Vec::new(),
                    }
                }
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
                params,
                return_type,
            } => Type::Fn {
                is_compose: false,
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
            other => Type::Struct {
                name: other.to_string(),
                fields: Vec::new(),
            },
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
            Type::Struct { name, .. } => write!(f, "{}", name),
            Type::Enum { name, .. } => write!(f, "{}", name),
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
    fn test_error_sentinel_unification() {
        assert!(Type::Error.is_assignable_to(&Type::Int));
        assert!(Type::Int.is_assignable_to(&Type::Error));
        assert!(Type::Error.is_assignable_to(&Type::Error));
    }
}
