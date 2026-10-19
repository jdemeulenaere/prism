//! Stable parameters and skippable composables (D7 of `compose-transform`).
//!
//! A parameter is stable when its type is `Copy` and holds no borrow, by its declared markers
//! alone (a type parameter only by its `Copy` bound), or when it is an `escaping` function type
//! that is not composable, or a nullable of one: such a value is kept when it is *keepable*,
//! which only its descriptor says, at run time (D9). A composable returning `Unit` whose
//! parameters are all stable is skippable. Deciding it costs one marker lookup per parameter
//! type, and nothing reads fields, bodies or other files.

use prism_mir::{Body, BodyOwner, LocalId};
use prism_typed_tree::{Type, TypeTable};

/// Whether a parameter of type `ty`, `escaping` or not, is stable.
pub fn is_stable(ty: &Type, escaping: bool, types: &TypeTable) -> bool {
    if is_function_parameter(ty) {
        return escaping && !is_composable_function(ty);
    }
    types.is_copy(ty) && ty.reference_outside_functions().is_none() && !is_reference(ty)
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
/// the context and the receiver included, is stable. Closures never are (D14).
pub fn is_skippable(body: &Body, types: &TypeTable) -> bool {
    if !body.is_compose || !matches!(body.owner, BodyOwner::Function(_)) {
        return false;
    }
    if *body.return_type() != Type::Unit {
        return false;
    }
    (1..=body.argument_count).all(|index| {
        let local = &body.locals[LocalId(index as u32)];
        is_stable(&local.ty, local.escaping, types)
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
            escaping: Vec::new(),
            ret: Box::new(Type::Unit),
        }
    }

    #[test]
    fn stability_follows_declared_markers() {
        let types = types();
        let parameter = |bound| {
            Type::Parameter(TypeParameter {
                index: 0,
                name: "T".to_string(),
                bound,
            })
        };
        let stable = [
            (Type::Int, false),
            (Type::Float, false),
            (Type::Bool, false),
            (Type::Char, false),
            (Type::Int.nullable(), false),
            (Type::plain_struct(NominalId::new("main", "Point")), false),
            (parameter(Some(TypeMarker::Copy)), false),
            (function(false), true),
            (function(false).nullable(), true),
        ];
        for (ty, escaping) in &stable {
            assert!(is_stable(ty, *escaping, &types), "{} is stable", ty);
        }
        let unstable = [
            (Type::String, false),
            (Type::plain_struct(NominalId::new("main", "User")), false),
            (
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Int),
                },
                false,
            ),
            (parameter(None), false),
            (parameter(Some(TypeMarker::Clone)), false),
            (function(false), false),
            (function(true), true),
            (Type::List(Box::new(Type::Int)), false),
        ];
        for (ty, escaping) in &unstable {
            assert!(!is_stable(ty, *escaping, &types), "{} is not stable", ty);
        }
    }
}
