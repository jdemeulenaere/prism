//! Types of places and operands, and which values need a drop (D4 and D6 of `mir-lowering`).

use crate::mir::{Body, Constant, Operand, Place, Projection};
use prism_ast::Span;
use prism_typed_tree::{NominalId, Type, TypeDef, TypeTable};

/// The definition of the built-in `Range` in the MIR's type table: the fields `start` and `end`
/// of the type system, and `endInclusive`, which a range construction sets and a `for` loop
/// reads (D4). The type system does not expose `endInclusive`.
pub fn mir_range_definition() -> TypeDef {
    TypeDef::Struct {
        parameters: Vec::new(),
        fields: vec![
            ("start".to_string(), false, Type::Int),
            ("end".to_string(), false, Type::Int),
            ("endInclusive".to_string(), false, Type::Bool),
        ],
    }
}

/// The index of the `endInclusive` field of the built-in `Range` in the MIR.
pub const RANGE_END_INCLUSIVE: usize = 2;

/// The type table of a file's MIR: the typed tree's, with the MIR's definition of `Range`.
pub fn mir_type_table(types: &TypeTable) -> TypeTable {
    let mut types = types.clone();
    types.replace_definition(NominalId::builtin_range(), mir_range_definition());
    types
}

/// Whether a value of `ty` needs a drop where its scope ends: every type except `Copy` types,
/// mutable borrows `&mut T` and nullables of these (D6). Whether a drop does anything is decided
/// by drop elaboration and the layout of the type.
pub fn needs_drop(ty: &Type, types: &TypeTable) -> bool {
    match ty {
        Type::Reference { mutable: true, .. } => false,
        Type::Nullable(inner) => needs_drop(inner, types),
        other => !types.is_copy(other),
    }
}

/// The type of `base` after `projection`, or `None` when the projection does not apply to it.
pub fn projection_type(base: &Type, projection: Projection, types: &TypeTable) -> Option<Type> {
    match (projection, base) {
        (Projection::Deref, Type::Reference { inner, .. } | Type::Heap(inner)) => {
            Some(inner.as_ref().clone())
        }
        // A field has its declared type with the struct's type arguments substituted.
        (Projection::Field(index), Type::Struct { id, arguments }) => types
            .struct_fields(id)
            .and_then(|fields| fields.get(index))
            .map(|(_, _, ty)| ty.substitute(arguments)),
        (Projection::Payload, Type::Nullable(inner)) => Some(inner.as_ref().clone()),
        (Projection::Index(_), Type::String) => Some(Type::Char),
        _ => None,
    }
}

impl Place {
    /// The type of the place in `body`. Panics on a projection that does not apply, which a
    /// well-formed body never has.
    pub fn ty(&self, body: &Body, types: &TypeTable) -> Type {
        self.try_ty(body, types)
            .unwrap_or_else(|| panic!("ill-typed place {:?}", self))
    }

    /// The type of the place in `body`, or `None` when a projection does not apply or the local
    /// is not declared.
    pub fn try_ty(&self, body: &Body, types: &TypeTable) -> Option<Type> {
        let mut ty = body.locals.get(self.local)?.ty.clone();
        for projection in &self.projections {
            ty = projection_type(&ty, *projection, types)?;
        }
        Some(ty)
    }
}

impl Operand {
    /// The type of the operand, or `None` for a constant whose type is not recorded (a function,
    /// a variant constructor or a top-level value) or an ill-typed place.
    pub fn try_ty(&self, body: &Body, types: &TypeTable) -> Option<Type> {
        match self {
            Operand::Copy(place, _) | Operand::Move(place, _) => place.try_ty(body, types),
            Operand::Constant(constant) => constant.ty(),
        }
    }

    /// The span of the source that reads the operand's place, if it reads one.
    pub fn span(&self) -> Option<Span> {
        match self {
            Operand::Copy(_, span) | Operand::Move(_, span) => Some(*span),
            Operand::Constant(_) => None,
        }
    }

    /// The place the operand reads, if any.
    pub fn place(&self) -> Option<&Place> {
        match self {
            Operand::Copy(place, _) | Operand::Move(place, _) => Some(place),
            Operand::Constant(_) => None,
        }
    }
}

impl Constant {
    /// The type of the constant, when it is recorded.
    pub fn ty(&self) -> Option<Type> {
        match self {
            Constant::Int(_) => Some(Type::Int),
            Constant::Float(_) => Some(Type::Float),
            Constant::Bool(_) => Some(Type::Bool),
            Constant::Char(_) => Some(Type::Char),
            Constant::String(_) => Some(Type::String),
            Constant::Unit => Some(Type::Unit),
            Constant::Null(ty) => Some(ty.clone()),
            Constant::Function(_)
            | Constant::VariantConstructor { .. }
            | Constant::TopLevelValue(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::{BodyOwner, LocalDeclaration, LocalId, LocalKind};
    use prism_ast::{Span, TypeMarker};
    use prism_typed_tree::{Arena, DeclarationPath};

    fn reference(mutable: bool, inner: Type) -> Type {
        Type::Reference {
            mutable,
            inner: Box::new(inner),
        }
    }

    fn table() -> TypeTable {
        let mut types = mir_type_table(&TypeTable::with_builtins());
        let user = NominalId::new("main", "User");
        types.insert_definition(
            user.clone(),
            TypeDef::Struct {
                parameters: Vec::new(),
                fields: vec![
                    ("id".to_string(), false, Type::Int),
                    ("name".to_string(), true, Type::String.nullable()),
                ],
            },
        );
        types.insert_marker(user, None, Vec::new());
        let point = NominalId::new("main", "Point");
        types.insert_definition(
            point.clone(),
            TypeDef::Struct {
                parameters: Vec::new(),
                fields: vec![("x".to_string(), false, Type::Int)],
            },
        );
        types.insert_marker(point, Some(TypeMarker::Copy), Vec::new());
        types
    }

    fn body_with_local(ty: Type) -> Body {
        let mut locals = Arena::new();
        for (ty, kind) in [(Type::Unit, LocalKind::Return), (ty, LocalKind::Argument)] {
            locals.alloc(LocalDeclaration {
                ty,
                kind,
                mutable: false,
                name: None,
                binding: None,
                span: Span::dummy(),
            });
        }
        Body {
            owner: BodyOwner::Function(DeclarationPath::new("main", "f")),
            is_compose: false,
            type_parameters: Vec::new(),
            locals,
            argument_count: 1,
            capture_count: 0,
            capture_modes: Vec::new(),
            blocks: Arena::new(),
            span: Span::dummy(),
        }
    }

    #[test]
    fn place_types_follow_projections() {
        let types = table();
        let user = Type::plain_struct(NominalId::new("main", "User"));
        let body = body_with_local(reference(false, reference(true, user.clone())));
        let base = Place::local(LocalId(1));
        assert_eq!(
            base.ty(&body, &types),
            reference(false, reference(true, user.clone()))
        );
        let referent = base.project(Projection::Deref).project(Projection::Deref);
        assert_eq!(referent.ty(&body, &types), user);
        let name = referent.project(Projection::Field(1));
        assert_eq!(name.ty(&body, &types), Type::String.nullable());
        assert_eq!(
            name.project(Projection::Payload).ty(&body, &types),
            Type::String
        );
        let character = name
            .project(Projection::Payload)
            .project(Projection::Index(LocalId(0)));
        assert_eq!(character.ty(&body, &types), Type::Char);
        assert_eq!(
            referent.project(Projection::Field(2)).try_ty(&body, &types),
            None
        );
        assert_eq!(
            base.project(Projection::Payload).try_ty(&body, &types),
            None
        );
    }

    #[test]
    fn range_has_its_mir_fields() {
        let types = table();
        let body = body_with_local(Type::plain_struct(NominalId::builtin_range()));
        let end_inclusive =
            Place::local(LocalId(1)).project(Projection::Field(RANGE_END_INCLUSIVE));
        assert_eq!(end_inclusive.ty(&body, &types), Type::Bool);
        let heap = body_with_local(Type::Heap(Box::new(Type::Int)));
        let referent = Place::local(LocalId(1)).project(Projection::Deref);
        assert_eq!(referent.ty(&heap, &types), Type::Int);
    }

    #[test]
    fn values_needing_a_drop() {
        let types = table();
        let user = Type::plain_struct(NominalId::new("main", "User"));
        let point = Type::plain_struct(NominalId::new("main", "Point"));
        for ty in [
            Type::String,
            Type::String.nullable(),
            Type::Heap(Box::new(Type::Int)),
            user.clone(),
            user.clone().nullable(),
            Type::Opaque("File".to_string()),
            Type::Fn {
                is_compose: false,
                params: vec![],
                ret: Box::new(Type::Int),
            },
        ] {
            assert!(needs_drop(&ty, &types), "{} needs a drop", ty);
        }
        for ty in [
            Type::Int,
            Type::Unit,
            Type::Nothing,
            point.clone(),
            point.nullable(),
            Type::plain_struct(NominalId::builtin_range()),
            reference(false, user.clone()),
            reference(true, user.clone()),
            reference(true, user).nullable(),
            Type::Int.nullable(),
        ] {
            assert!(!needs_drop(&ty, &types), "{} needs no drop", ty);
        }
    }
}
