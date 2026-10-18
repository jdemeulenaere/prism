//! Subtyping rules for Prism, and the lowering of annotations and ABI types to semantic types.
//!
//! The semantic [`Type`] itself lives in `prism-typed-tree`, which the typed tree shares with
//! later stages; the rules stay with the type checker.

use prism_abi::{AbiType, AbiTypeParameter, NominalKind, PrimKind};
pub use prism_typed_tree::types::{NominalId, Type, TypeDef, TypeParameter, BUILTIN_MODULE};

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
    /// - `Heap<T>` is invariant in its contents
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

        // Generic types are invariant in their arguments (D11 of `generics`); an argument that
        // is an earlier error matches any.
        if let (
            Type::Struct {
                id: self_id,
                arguments: self_arguments,
            },
            Type::Struct { id, arguments },
        )
        | (
            Type::Enum {
                id: self_id,
                arguments: self_arguments,
            },
            Type::Enum { id, arguments },
        ) = (self, expected)
        {
            if self_id == id
                && self_arguments.len() == arguments.len()
                && self_arguments
                    .iter()
                    .zip(arguments)
                    .all(|(found, expected)| same_modulo_errors(found, expected))
            {
                return true;
            }
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

        // Boxes are invariant (D8 of `heap-boxes`): `Heap<Int>` and `Heap<Int?>` hold contents
        // of different layouts, so a box cannot be read as a box of another type.
        if let (Type::Heap(self_inner), Type::Heap(exp_inner)) = (self, expected) {
            return same_modulo_errors(self_inner, exp_inner);
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

/// Converts a type recorded in an imported file's ABI, in a signature or definition whose
/// declaration has the type `parameters`. Phase 1 has already bound each nominal name to its
/// defining module; an unbound name stays opaque, and a type parameter is the declaration's.
pub fn type_from_abi(ty: &AbiType, parameters: &[TypeParameter]) -> Type {
    let convert = |inner: &AbiType| type_from_abi(inner, parameters);
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
            kind,
            module,
            name,
            args,
            ..
        } => {
            let id = NominalId::new(module.clone(), name.clone());
            let arguments = args.iter().map(convert).collect();
            match kind {
                NominalKind::Struct => Type::Struct { id, arguments },
                NominalKind::Enum => Type::Enum { id, arguments },
            }
        }
        AbiType::Range => Type::plain_struct(NominalId::builtin_range()),
        AbiType::Unbound { name, .. } => Type::Opaque(name.clone()),
        AbiType::Parameter { index, name } => parameters
            .get(*index as usize)
            .map_or_else(|| Type::Opaque(name.clone()), TypeParameter::as_type),
        AbiType::Nullable(inner) => Type::Nullable(Box::new(convert(inner))),
        AbiType::Ref { mutable, inner } => Type::Reference {
            mutable: *mutable,
            inner: Box::new(convert(inner)),
        },
        AbiType::Heap(inner) => Type::Heap(Box::new(convert(inner))),
        AbiType::Fn {
            compose,
            params,
            ret,
        } => Type::Fn {
            is_compose: *compose,
            params: params.iter().map(convert).collect(),
            ret: Box::new(convert(ret)),
        },
    }
}

/// The type parameters an ABI records for a declaration, as semantic type parameters.
pub fn type_parameters_from_abi(parameters: &[AbiTypeParameter]) -> Vec<TypeParameter> {
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

/// Whether either type is the error type, which is compatible with everything.
fn is_error_pair(a: &Type, b: &Type) -> bool {
    a == &Type::Error || b == &Type::Error
}

/// Whether `a` and `b` are the same type, where an error anywhere inside one matches the other.
fn same_modulo_errors(a: &Type, b: &Type) -> bool {
    if a == b || is_error_pair(a, b) {
        return true;
    }
    match (a, b) {
        (Type::Nullable(a), Type::Nullable(b)) | (Type::Heap(a), Type::Heap(b)) => {
            same_modulo_errors(a, b)
        }
        (
            Type::Reference {
                mutable: a_mutable,
                inner: a,
            },
            Type::Reference {
                mutable: b_mutable,
                inner: b,
            },
        ) => a_mutable == b_mutable && same_modulo_errors(a, b),
        (
            Type::Struct {
                id: a_id,
                arguments: a,
            },
            Type::Struct {
                id: b_id,
                arguments: b,
            },
        )
        | (
            Type::Enum {
                id: a_id,
                arguments: a,
            },
            Type::Enum {
                id: b_id,
                arguments: b,
            },
        ) => {
            a_id == b_id
                && a.len() == b.len()
                && a.iter().zip(b).all(|(a, b)| same_modulo_errors(a, b))
        }
        (
            Type::Fn {
                is_compose: a_compose,
                params: a_params,
                ret: a_ret,
            },
            Type::Fn {
                is_compose: b_compose,
                params: b_params,
                ret: b_ret,
            },
        ) => {
            a_compose == b_compose
                && a_params.len() == b_params.len()
                && a_params
                    .iter()
                    .zip(b_params)
                    .all(|(a, b)| same_modulo_errors(a, b))
                && same_modulo_errors(a_ret, b_ret)
        }
        _ => false,
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
