//! Subtyping rules for Prism, and the lowering of annotations and ABI types to semantic types.
//!
//! The semantic [`Type`] itself lives in `prism-typed-tree`, which the typed tree shares with
//! later stages; the rules stay with the type checker.

use prism_abi::{AbiType, NominalKind, PrimKind};
pub use prism_typed_tree::types::{NominalId, Type, TypeDef, BUILTIN_MODULE};

/// The assignability rules over semantic types: subtyping, and the join of two branches.
pub trait TypeRelations {
    /// Checks whether `self` is assignable to `expected`; see the implementation for `Type`.
    fn is_assignable_to(&self, expected: &Type) -> bool;
    /// Whether a shared borrow of `self` can be read as a shared borrow of `target`.
    fn is_shared_view_of(&self, target: &Type) -> bool;
    /// Joins the types of two branches into the least type both are assignable to.
    fn join(&self, other: &Type) -> Option<Type>;
}

impl TypeRelations for Type {
    /// Checks whether `self` is assignable to `expected` according to Prism's subtyping lattice:
    /// - `Error` unifies with any type (suppresses cascade errors)
    /// - `Nothing <= T` for all `T` (divergent expressions can be placed in any value branch)
    /// - `T <= T?` for all `T` (non-null value is assignable to nullable)
    /// - `&mut T <= &T` (mutable borrow can coerce to shared borrow at call sites); references are
    ///   otherwise invariant in their referent
    /// - Structs and Enums are nominal (matched by [`NominalId`])
    /// - All other types are strictly invariant (e.g. `Int` does NOT widen to `Float`)
    fn is_assignable_to(&self, expected: &Type) -> bool {
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

        // References are invariant: a reference points at existing storage, so it cannot change
        // the representation of its referent (`&Int` is not a `&(Int?)`). `&mut` may weaken to
        // `&` at the outer layer, and behind shared borrows.
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
            return if *exp_mut {
                *self_mut && (self_inner == exp_inner || is_error_pair(self_inner, exp_inner))
            } else {
                self_inner.is_shared_view_of(exp_inner)
            };
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

    /// Whether a shared borrow of `self` can be read as a shared borrow of `target`: the two are
    /// the same type, except that a reference inside `self` may be `&mut` where `target` has `&`.
    fn is_shared_view_of(&self, target: &Type) -> bool {
        if self == target || is_error_pair(self, target) {
            return true;
        }
        match (self, target) {
            (
                Type::Reference { inner, .. },
                Type::Reference {
                    mutable: false,
                    inner: target_inner,
                },
            ) => inner.is_shared_view_of(target_inner),
            _ => false,
        }
    }

    /// Joins the types of two branches (`if`/`else`, `when`) into the least type both are
    /// assignable to, or `None` if they have no common type.
    ///
    /// `Nothing` joins to the other side, and a nullable side makes the join nullable, so
    /// `Int` joined with `null` (`Nothing?`) is `Int?`.
    fn join(&self, other: &Type) -> Option<Type> {
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
}

/// Converts an AST type to a semantic `Type`. Nominal names stay [`Type::Opaque`] until the
/// checker binds them in scope.
pub fn type_from_ast(ast_ty: &prism_ast::types::Type) -> Type {
    match &ast_ty.kind {
        prism_ast::types::TypeKind::Named { name, type_args } => match name.as_str() {
            "Int" => Type::Int,
            "Float" | "Double" => Type::Float,
            "Bool" => Type::Bool,
            "Char" => Type::Char,
            "String" => Type::String,
            "Unit" => Type::Unit,
            "Nothing" => Type::Nothing,
            "Heap" if type_args.len() == 1 => Type::Heap(Box::new(type_from_ast(&type_args[0]))),
            // Nominal type or generic type parameter, bound by the checker's scope.
            _ => Type::Opaque(name.clone()),
        },
        prism_ast::types::TypeKind::Nullable(inner) => {
            Type::Nullable(Box::new(type_from_ast(inner)))
        }
        prism_ast::types::TypeKind::Reference { mutable, inner } => Type::Reference {
            mutable: *mutable,
            inner: Box::new(type_from_ast(inner)),
        },
        prism_ast::types::TypeKind::Unit => Type::Unit,
        prism_ast::types::TypeKind::Function {
            is_compose,
            params,
            return_type,
        } => Type::Fn {
            is_compose: *is_compose,
            params: params.iter().map(type_from_ast).collect(),
            ret: Box::new(type_from_ast(return_type)),
        },
    }
}

/// Converts a type recorded in an imported file's ABI. Phase 1 has already bound each
/// nominal name to its defining module; an unbound name stays opaque.
pub fn type_from_abi(ty: &AbiType) -> Type {
    match ty {
        AbiType::Prim(prim) => match prim {
            PrimKind::Int => Type::Int,
            PrimKind::Float => Type::Float,
            PrimKind::Bool => Type::Bool,
            PrimKind::Char => Type::Char,
            PrimKind::String => Type::String,
            PrimKind::Nothing => Type::Nothing,
        },
        AbiType::Unit => Type::Unit,
        AbiType::Nominal {
            kind, module, name, ..
        } => {
            let id = NominalId::new(module.clone(), name.clone());
            match kind {
                NominalKind::Struct => Type::plain_struct(id),
                NominalKind::Enum => Type::plain_enum(id),
            }
        }
        AbiType::Range => Type::plain_struct(NominalId::builtin_range()),
        AbiType::Unbound { name, .. } | AbiType::Parameter { name, .. } => {
            Type::Opaque(name.clone())
        }
        AbiType::Nullable(inner) => Type::Nullable(Box::new(type_from_abi(inner))),
        AbiType::Ref { mutable, inner } => Type::Reference {
            mutable: *mutable,
            inner: Box::new(type_from_abi(inner)),
        },
        AbiType::Heap(inner) => Type::Heap(Box::new(type_from_abi(inner))),
        AbiType::Fn {
            compose,
            params,
            ret,
        } => Type::Fn {
            is_compose: *compose,
            params: params.iter().map(type_from_abi).collect(),
            ret: Box::new(type_from_abi(ret)),
        },
    }
}

/// Whether either type is the error type, which is compatible with everything.
fn is_error_pair(a: &Type, b: &Type) -> bool {
    a == &Type::Error || b == &Type::Error
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
        let a_user = Type::plain_struct(NominalId::new("a/lib", "User"));
        let b_user = Type::plain_struct(NominalId::new("b/lib", "User"));
        let a_enum = Type::plain_enum(NominalId::new("a/lib", "User"));
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

    #[test]
    fn references_are_invariant_in_their_referent() {
        let shared = |t: Type| Type::Reference {
            mutable: false,
            inner: Box::new(t),
        };
        let mutable = |t: Type| Type::Reference {
            mutable: true,
            inner: Box::new(t),
        };
        let nullable_int = Type::Nullable(Box::new(Type::Int));
        // A reference cannot change its referent's representation.
        assert!(!shared(Type::Int).is_assignable_to(&shared(nullable_int.clone())));
        assert!(!mutable(Type::Int).is_assignable_to(&mutable(nullable_int.clone())));
        assert!(!mutable(Type::Int).is_assignable_to(&shared(nullable_int.clone())));
        assert!(!shared(Type::Nothing).is_assignable_to(&shared(Type::Int)));
        // `&mut` weakens to `&`, at the top and behind shared borrows only.
        assert!(mutable(Type::Int).is_assignable_to(&shared(Type::Int)));
        assert!(shared(mutable(Type::Int)).is_assignable_to(&shared(shared(Type::Int))));
        assert!(mutable(mutable(Type::Int)).is_assignable_to(&shared(shared(Type::Int))));
        assert!(!mutable(mutable(Type::Int)).is_assignable_to(&mutable(shared(Type::Int))));
        assert!(!shared(Type::Int).is_assignable_to(&mutable(Type::Int)));
        // Nullable references keep the nullable rules.
        assert!(shared(Type::Int).is_assignable_to(&Type::Nullable(Box::new(shared(Type::Int)))));
        assert!(Type::Nullable(Box::new(mutable(Type::Int)))
            .is_assignable_to(&Type::Nullable(Box::new(shared(Type::Int)))));
        // The error type is still absorbed behind a reference.
        assert!(shared(Type::Error).is_assignable_to(&shared(Type::Int)));
        assert!(mutable(Type::Int).is_assignable_to(&mutable(Type::Error)));
    }
}
