//! Stable parameters and skippable composables (D7 of `compose-transform`).
//!
//! A parameter is stable when its type is `Copy` and holds no borrow, by its declared markers
//! alone (a type parameter only by its `Copy` bound), or when it is an `escaping` function type
//! that is not composable, or a nullable of one: such a value is kept when it is *keepable*,
//! which only its descriptor says, at run time (D9). A composable returning `Unit` whose
//! parameters are all stable is skippable. Deciding it costs one marker lookup per parameter
//! type, and nothing reads fields, bodies or other files.

use prism_mir::{Body, BodyOwner, LocalId};
use prism_typed_tree::{ParameterKind, Type, TypeTable};

/// Whether a parameter of type `ty`, declared with `kind`, is stable: an `escaping` or
/// `attached` function value is kept as one (D9 of `owned-state`).
pub fn is_stable(ty: &Type, kind: ParameterKind, types: &TypeTable) -> bool {
    if is_function_parameter(ty) {
        return kind.keeps() && !is_composable_function(ty);
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
        is_stable(&local.ty, local.parameter_kind, types)
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
        ];
        for (ty, kind) in &stable {
            assert!(is_stable(ty, *kind, &types), "{} is stable", ty);
        }
        let unstable = [
            (Type::String, ParameterKind::Plain),
            (
                Type::plain_struct(NominalId::new("main", "User")),
                ParameterKind::Plain,
            ),
            (
                Type::Reference {
                    mutable: false,
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
        for (ty, kind) in &unstable {
            assert!(!is_stable(ty, *kind, &types), "{} is not stable", ty);
        }
    }
}
