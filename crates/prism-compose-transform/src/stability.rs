//! Parameters that can be kept, and restartable composables (D7 of `compose-transform`, D1 of
//! `strong-skipping`).
//!
//! There is no stability inference: a composable returning `Unit` is restartable, and so
//! skippable, when every parameter can be kept. A parameter can be kept when its type is `Copy`
//! and holds no borrow, by its declared markers alone (a type parameter only by its `Copy`
//! bound), compared by `==`; when it is a shared borrow of a type with a capture function (a
//! state object, kept as its weak handle and compared by identity, D13 of `owned-state`); when
//! it is any other shared borrow `&T`, or a nullable of one, kept as its identity, its address
//! and its source's version (D2 of `strong-skipping`); or when it is an `escaping` or
//! `attached` function type that is not composable, or a nullable of one: such a value is kept
//! when it is *keepable*, which only its descriptor says, at run time (D9). By-value values that
//! are not `Copy`, `&mut` borrows and composable content lambdas cannot be kept: a restart
//! could pass them again only by cloning or aliasing. Deciding it costs one marker or
//! capture-function lookup per parameter type, and nothing reads fields, bodies or other files.

use prism_mir::{Body, BodyOwner, LocalId};
use prism_typed_tree::{ParameterKind, Type, TypeTable};

/// Whether a parameter of type `ty`, declared with `kind`, can be kept: an `escaping` or
/// `attached` function value is kept as one (D9 of `owned-state`), a shared borrow as its
/// identity (D1 of `strong-skipping`).
pub fn is_keepable(ty: &Type, kind: ParameterKind, types: &TypeTable) -> bool {
    if is_function_parameter(ty) {
        return kind.keeps() && !is_composable_function(ty);
    }
    if is_shared_reference(ty) {
        return true;
    }
    types.is_copy(ty) && ty.reference_outside_functions().is_none() && !is_reference(ty)
}

/// Whether a parameter of type `ty` takes a version word after the change words: a shared
/// borrow, or a nullable of one, of a type without a capture function (D4 of
/// `strong-skipping`). A borrowed state object keeps its cell's identity instead.
pub fn has_version_word(ty: &Type, types: &TypeTable) -> bool {
    is_shared_reference(ty) && !is_borrowed_state(ty, types)
}

/// Whether `ty` is a shared borrow `&T` or a nullable `(&T)?` of one.
fn is_shared_reference(ty: &Type) -> bool {
    match ty {
        Type::Reference { mutable, .. } => !mutable,
        Type::Nullable(inner) => matches!(**inner, Type::Reference { mutable: false, .. }),
        _ => false,
    }
}

/// Whether `ty` is a shared borrow of a type with a capture function (`&State<T>`,
/// `&MutableState<T>`): a parameter kept as its weak handle (D13 of `owned-state`).
pub fn is_borrowed_state(ty: &Type, types: &TypeTable) -> bool {
    match ty {
        Type::Reference {
            mutable: false,
            inner,
        } => types.has_capture_function(inner),
        _ => false,
    }
}

/// The type a stable parameter of type `ty` is kept as: a borrowed state's weak reference, else
/// `ty` itself.
pub fn kept_type(ty: &Type, types: &TypeTable) -> Type {
    match ty {
        Type::Reference { inner, .. } if is_borrowed_state(ty, types) => Type::Struct {
            id: prism_typed_tree::IntrinsicStruct::WeakReference.id(),
            arguments: vec![inner.as_ref().clone()],
        },
        _ => ty.clone(),
    }
}

/// Whether `ty` is a function type or a nullable of one: a parameter kept as a function value.
pub fn is_function_parameter(ty: &Type) -> bool {
    match ty {
        Type::Fn { .. } => true,
        Type::Nullable(inner) => matches!(**inner, Type::Fn { .. }),
        _ => false,
    }
}

fn is_composable_function(ty: &Type) -> bool {
    match ty {
        Type::Fn { is_compose, .. } => *is_compose,
        Type::Nullable(inner) => is_composable_function(inner),
        _ => false,
    }
}

fn is_reference(ty: &Type) -> bool {
    match ty {
        Type::Reference { .. } => true,
        Type::Nullable(inner) => is_reference(inner),
        _ => false,
    }
}

/// Whether the composable function `body` is skippable: it returns `Unit` and every parameter,
/// the context and the receiver included, can be kept. Closures never are (D14).
pub fn is_skippable(body: &Body, types: &TypeTable) -> bool {
    if !body.is_compose || !matches!(body.owner, BodyOwner::Function(_)) {
        return false;
    }
    if *body.return_type() != Type::Unit {
        return false;
    }
    (1..=body.argument_count).all(|index| {
        let local = &body.locals[LocalId(index as u32)];
        is_keepable(&local.ty, local.parameter_kind, types)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use prism_ast::TypeMarker;
    use prism_typed_tree::{NominalId, TypeDef, TypeParameter};

    fn types() -> TypeTable {
        let mut types = TypeTable::with_builtins();
        for (name, marker) in [("Point", Some(TypeMarker::Copy)), ("User", None)] {
            let id = NominalId::new("main", name);
            types.insert_definition(
                id.clone(),
                TypeDef::Struct {
                    parameters: Vec::new(),
                    fields: vec![("x".to_string(), false, Type::Int)],
                },
            );
            types.insert_marker(id, marker, Vec::new());
        }
        types
    }

    fn function(is_compose: bool) -> Type {
        Type::Fn {
            is_compose,
            receiver: false,
            params: Vec::new(),
            kinds: Vec::new(),
            ret: Box::new(Type::Unit),
        }
    }

    #[test]
    fn keepability_follows_parameter_kinds() {
        let types = types();
        let parameter = |bound| {
            Type::Parameter(TypeParameter {
                index: 0,
                name: "T".to_string(),
                bound,
            })
        };
        let keepable = [
            (Type::Int, ParameterKind::Plain),
            (Type::Float, ParameterKind::Plain),
            (Type::Bool, ParameterKind::Plain),
            (Type::Char, ParameterKind::Plain),
            (Type::Int.nullable(), ParameterKind::Plain),
            (
                Type::plain_struct(NominalId::new("main", "Point")),
                ParameterKind::Plain,
            ),
            (parameter(Some(TypeMarker::Copy)), ParameterKind::Plain),
            (function(false), ParameterKind::Escaping),
            (function(false), ParameterKind::Attached),
            (function(false).nullable(), ParameterKind::Escaping),
            (
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Int),
                },
                ParameterKind::Plain,
            ),
            (
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::String),
                }
                .nullable(),
                ParameterKind::Plain,
            ),
        ];
        for (ty, kind) in &keepable {
            assert!(is_keepable(ty, *kind, &types), "{} can be kept", ty);
        }
        let unkeepable = [
            (Type::String, ParameterKind::Plain),
            (
                Type::plain_struct(NominalId::new("main", "User")),
                ParameterKind::Plain,
            ),
            (
                Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::Int),
                },
                ParameterKind::Plain,
            ),
            (parameter(None), ParameterKind::Plain),
            (parameter(Some(TypeMarker::Clone)), ParameterKind::Plain),
            (function(false), ParameterKind::Plain),
            (function(true), ParameterKind::Escaping),
            (Type::List(Box::new(Type::Int)), ParameterKind::Plain),
        ];
        for (ty, kind) in &unkeepable {
            assert!(!is_keepable(ty, *kind, &types), "{} cannot be kept", ty);
        }
        // Borrows other than of state objects take a version word.
        let borrow = Type::Reference {
            mutable: false,
            inner: Box::new(Type::String),
        };
        assert!(has_version_word(&borrow, &types));
        assert!(has_version_word(&borrow.nullable(), &types));
        assert!(!has_version_word(&Type::Int, &types));
    }
}
