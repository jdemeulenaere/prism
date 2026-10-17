//! Places as the exclusivity check sees them (D3, D6 and D13 of `borrowck-exclusivity`): which
//! types carry loans, when two places overlap, whether a place reaches into another through a
//! reference, and how a place is named in a diagnostic.

use prism_mir::{Body, Place, Projection};
use prism_typed_tree::{Type, TypeTable};

/// Whether a value of type `ty` can hold loans: a reference or a function type outside a
/// nominal type (`&T`, `&mut T`, `() -> Int`), or a nullable or `Heap<T>` of one. Structs and
/// enums never carry loans, since their fields and payloads cannot be borrows and a capturing
/// closure cannot be stored in them.
pub fn carries_loans(ty: &Type) -> bool {
    match ty {
        Type::Reference { .. } | Type::Fn { .. } => true,
        Type::Nullable(inner) | Type::Heap(inner) => carries_loans(inner),
        _ => false,
    }
}

/// Whether `ty` is a function type, or a nullable or `Heap<T>` of one.
pub fn contains_function(ty: &Type) -> bool {
    match ty {
        Type::Fn { .. } => true,
        Type::Nullable(inner) | Type::Heap(inner) => contains_function(inner),
        _ => false,
    }
}

/// Whether a parameter of type `parameter` is an input borrow (D9): its type contains a
/// reference outside a function type (`&T`, `&mut T`, `(&T)?`).
pub fn is_input_borrow(parameter: &Type) -> bool {
    match parameter {
        Type::Reference { .. } => true,
        Type::Nullable(inner) | Type::Heap(inner) => is_input_borrow(inner),
        _ => false,
    }
}

/// Whether the places `a` and `b` of one body overlap: they have the same root local, and one
/// is a prefix of the other once equal projections are skipped. Two different fields are
/// disjoint; two indices always overlap, since the indices are not known.
pub fn places_overlap(a: &Place, b: &Place) -> bool {
    if a.local != b.local {
        return false;
    }
    for (left, right) in a.projections.iter().zip(&b.projections) {
        match (left, right) {
            (Projection::Field(i), Projection::Field(j)) if i != j => return false,
            _ => {}
        }
    }
    true
}

/// Whether `place` starts with every projection of `prefix` (equal projections, any index).
pub fn is_prefix(prefix: &Place, place: &Place) -> bool {
    prefix.local == place.local
        && prefix.projections.len() <= place.projections.len()
        && prefix
            .projections
            .iter()
            .zip(&place.projections)
            .all(|(left, right)| match (left, right) {
                (Projection::Index(_), Projection::Index(_)) => true,
                _ => left == right,
            })
}

/// Whether `place` continues `prefix` through a dereference of a reference (rather than of a
/// `Heap<T>`, which `prefix` owns): what it names is not owned by `prefix`, so writing,
/// dropping or ending `prefix` leaves it alone.
pub fn extends_through_reference(
    body: &Body,
    types: &TypeTable,
    place: &Place,
    prefix: &Place,
) -> bool {
    if !is_prefix(prefix, place) {
        return false;
    }
    let mut ty = prefix.ty(body, types);
    for projection in &place.projections[prefix.projections.len()..] {
        if *projection == Projection::Deref && matches!(ty, Type::Reference { .. }) {
            return true;
        }
        match prism_mir::projection_type(&ty, *projection, types) {
            Some(next) => ty = next,
            None => return false,
        }
    }
    false
}

/// The source name of `place` in `body`: its local's name followed by its field names, with
/// dereferences, payloads and indices implicit (`(*_1).user` of `o` is `o.user`), or `None` for
/// a place rooted at a temporary.
pub fn place_name(body: &Body, types: &TypeTable, place: &Place) -> Option<String> {
    let declaration = body.locals.get(place.local)?;
    let mut name = declaration.name.clone()?;
    let mut ty = declaration.ty.clone();
    for projection in &place.projections {
        if let (Projection::Field(index), Type::Struct(id)) = (projection, &ty) {
            if let Some((field, _, _)) = types.struct_fields(id).and_then(|f| f.get(*index)) {
                name = format!("{}.{}", name, field);
            }
        }
        match prism_mir::projection_type(&ty, *projection, types) {
            Some(next) => ty = next,
            None => break,
        }
    }
    Some(name)
}

/// `'name'`, or "a temporary value" for a place without a name.
pub fn quoted(name: &Option<String>) -> String {
    match name {
        Some(name) => format!("'{}'", name),
        None => "a temporary value".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_bodies::{body, lower};
    use prism_mir::{LocalId, Rvalue, StatementKind};

    const SOURCE: &str = "\
struct User { var name: String }
struct Order { var user: User, var email: String }
struct Point { var x: Int }
fun touch(a: &mut User, b: &mut String) {}
fun f(o: &mut Order, h: Heap<Point>, s: String) {
    touch(&mut o.user, &mut o.email)
    val r = &h.x
    val c = s[0]
    val d = s[1]
}
";

    /// The places borrowed in `f`, in order.
    fn borrowed_places() -> (prism_mir::MirFile, Vec<Place>) {
        let file = lower(SOURCE);
        let f = body(&file, "f");
        let mut places = Vec::new();
        for (_, block) in f.blocks.iter() {
            for statement in &block.statements {
                if let StatementKind::Assign(_, rvalue) = &statement.kind {
                    if let Rvalue::Borrow { place, .. } = rvalue.as_ref() {
                        places.push(place.clone());
                    }
                }
            }
        }
        (file, places)
    }

    #[test]
    fn fields_are_disjoint_and_prefixes_overlap() {
        let (file, places) = borrowed_places();
        let f = body(&file, "f");
        let (user, email) = (&places[0], &places[1]);
        assert_eq!(place_name(f, &file.types, user).as_deref(), Some("o.user"));
        assert_eq!(
            place_name(f, &file.types, email).as_deref(),
            Some("o.email")
        );
        assert!(!places_overlap(user, email));
        let order = Place::local(LocalId(1)).project(Projection::Deref);
        assert!(places_overlap(&order, user) && places_overlap(user, &order));
        assert!(places_overlap(user, user));
        // `o.user` continues `o` through the reference, not `(*o)`.
        let o = Place::local(LocalId(1));
        assert!(extends_through_reference(f, &file.types, user, &o));
        assert!(!extends_through_reference(f, &file.types, user, &order));
        assert_eq!(place_name(f, &file.types, &o).as_deref(), Some("o"));
    }

    #[test]
    fn a_heap_is_owned_and_indices_overlap() {
        let (file, places) = borrowed_places();
        let f = body(&file, "f");
        let x = &places[2];
        let h = Place::local(LocalId(2));
        assert!(is_prefix(&h, x));
        assert!(!extends_through_reference(f, &file.types, x, &h));
        assert_eq!(place_name(f, &file.types, x).as_deref(), Some("h.x"));
        let s = Place::local(LocalId(3));
        let first = s.project(Projection::Index(LocalId(10)));
        let second = s.project(Projection::Index(LocalId(11)));
        assert!(places_overlap(&first, &second));
    }

    #[test]
    fn loan_carrying_types() {
        let int_reference = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Int),
        };
        let function = Type::Fn {
            params: vec![],
            ret: Box::new(Type::Int),
            is_compose: false,
        };
        assert!(carries_loans(&int_reference));
        assert!(carries_loans(&int_reference.clone().nullable()));
        assert!(carries_loans(&function));
        assert!(!carries_loans(&Type::String));
        assert!(contains_function(&function.clone().nullable()));
        assert!(!contains_function(&int_reference));
        assert!(is_input_borrow(&int_reference.nullable()));
        assert!(!is_input_borrow(&function));
    }
}
