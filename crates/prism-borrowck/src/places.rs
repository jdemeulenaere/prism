//! Places as the exclusivity check sees them (D3, D6 and D13 of `borrowck-exclusivity`): which
//! types carry loans, when two places overlap, whether a place reaches into another through a
//! reference, and how a place is named in a diagnostic.

use prism_mir::{Body, Place, Projection};
use prism_typed_tree::{Type, TypeTable};

/// Whether a value of type `ty` can hold loans: a reference or a function type outside a
/// nominal type (`&T`, `&mut T`, `() -> Int`), a keeper (`Composition`, which holds the loans of
/// the weak captures of the content it keeps, D24 of `owned-state`), or a nullable or `Heap<T>`
/// of one. Other structs and enums never carry loans, since their fields and payloads cannot be
/// borrows and a capturing closure cannot be stored in them. Neither does a value of a type
/// parameter: type arguments are borrow-free, and a capturing closure is never passed for one
/// (D7 of `generics`).
pub fn carries_loans(ty: &Type) -> bool {
    match ty {
        Type::Reference { .. } | Type::Fn { .. } => true,
        Type::Struct { .. } => ty.contains_keeper(),
        Type::Nullable(inner) | Type::Heap(inner) => carries_loans(inner),
        _ => false,
    }
}

/// Whether a value of type `ty` keeps attached function values past the call that gives them
/// (D24 of `owned-state`): a keeper, or a nullable or `Heap<T>` of one.
pub fn contains_keeper(ty: &Type) -> bool {
    ty.contains_keeper()
}

/// Whether `ty` is a function type, or a nullable or `Heap<T>` of one. A type parameter is not:
/// what it stands for holds no capture loans (D7 of `generics`).
pub fn contains_function(ty: &Type) -> bool {
    match ty {
        Type::Fn { .. } => true,
        Type::Nullable(inner) | Type::Heap(inner) => contains_function(inner),
        _ => false,
    }
}

/// Whether a value of type `ty` lets whoever holds it write loans into what it points at: a
/// `&mut R` whose `R` carries loans (`&mut &Int`), a reference to a keeper (`&Composition`), or
/// a nullable of one.
pub fn writes_loans_through(ty: &Type) -> bool {
    match ty {
        Type::Reference {
            mutable: true,
            inner,
        } => carries_loans(inner),
        // A keeper keeps what an `attached` argument holds through any reference to it (D24 of
        // `owned-state`).
        Type::Reference { inner, .. } if inner.contains_keeper() => true,
        Type::Nullable(inner) => writes_loans_through(inner),
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

/// Whether a value of type `ty` is a shared input borrow (D10 of `borrowck-elision`): an input
/// borrow whose outermost reference, under nullables, is shared (`&T`, `(&T)?`).
pub fn is_shared_input_borrow(ty: &Type) -> bool {
    match ty {
        Type::Reference { mutable, .. } => !mutable,
        Type::Nullable(inner) | Type::Heap(inner) => is_shared_input_borrow(inner),
        _ => false,
    }
}

/// Whether a value of type `ty` can borrow a place whose type is itself a reference (`&T`,
/// `(&T)?`): `ty` contains a reference whose referent contains a reference (`&&Int`,
/// `&(&Int)?`), or a function type, whose closure may borrow any place. A value of any other
/// type points past such a place, never at it: a `&Int` copied out of a `&&Int`, or a call's
/// `&String` result, borrows what the reference refers to, not the reference.
pub fn can_borrow_reference(ty: &Type) -> bool {
    match ty {
        Type::Fn { .. } => true,
        Type::Reference { inner, .. } => is_input_borrow(inner) || can_borrow_reference(inner),
        Type::Nullable(inner) | Type::Heap(inner) => can_borrow_reference(inner),
        _ => false,
    }
}

/// Whether the places `a` and `b` of one body overlap: they have the same root local, and one
/// is a prefix of the other once equal projections are skipped. Two different fields, or two
/// different payloads of one variant, are disjoint; two indices always overlap, since the
/// indices are not known.
pub fn places_overlap(a: &Place, b: &Place) -> bool {
    if a.local != b.local {
        return false;
    }
    for (left, right) in a.projections.iter().zip(&b.projections) {
        match (left, right) {
            (Projection::Field(i), Projection::Field(j)) if i != j => return false,
            // Payloads of one variant are disjoint like fields; payloads of different variants
            // share storage, so they overlap (D16 of `payload-patterns`).
            (
                Projection::VariantField {
                    variant: v,
                    field: i,
                },
                Projection::VariantField {
                    variant: w,
                    field: j,
                },
            ) if v == w && i != j => return false,
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
        // A weak reference's view names a state object owned elsewhere (D10 of `owned-state`).
        if (*projection == Projection::Deref && matches!(ty, Type::Reference { .. }))
            || *projection == Projection::View
        {
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
        if let (Projection::Field(index), Type::Struct { id, .. }) = (projection, &ty) {
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

    #[test]
    fn type_parameters_are_borrow_free_and_function_free() {
        for bound in [None, Some(prism_ast::TypeMarker::Copy)] {
            let parameter = Type::Parameter(prism_typed_tree::TypeParameter {
                index: 0,
                name: "T".to_string(),
                bound,
            });
            let nullable = parameter.clone().nullable();
            for ty in [&parameter, &nullable] {
                assert!(!carries_loans(ty), "{}", ty);
                assert!(!contains_function(ty), "{}", ty);
                assert!(!writes_loans_through(ty), "{}", ty);
                assert!(!is_input_borrow(ty), "{}", ty);
                assert!(!can_borrow_reference(ty), "{}", ty);
            }
            // A borrow of a type-parameter value is an input borrow like any other.
            let borrowed = Type::Reference {
                mutable: false,
                inner: Box::new(parameter),
            };
            assert!(is_input_borrow(&borrowed) && carries_loans(&borrowed));
        }
    }

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

    /// D11 of `collections`: a list owns its elements, as a box owns its contents, and two
    /// elements of one list always overlap.
    #[test]
    fn a_list_owns_its_elements() {
        let file = lower(
            "fun f(xs: List<String>, ys: &List<String>) {\n    val a = &xs[0]\n    val b = &ys[1]\n}\n",
        );
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
        let (element, borrowed_element) = (&places[0], &places[1]);
        let xs = Place::local(LocalId(1));
        assert!(is_prefix(&xs, element) && places_overlap(&xs, element));
        // Writing, moving or dropping `xs` reaches its elements.
        assert!(!extends_through_reference(f, &file.types, element, &xs));
        assert_eq!(place_name(f, &file.types, element).as_deref(), Some("xs"));
        let other = xs.project(Projection::Index(LocalId(9)));
        assert!(places_overlap(element, &other));
        // An element of a borrowed list continues `ys` through its reference only.
        let ys = Place::local(LocalId(2));
        let list = ys.project(Projection::Deref);
        assert!(extends_through_reference(
            f,
            &file.types,
            borrowed_element,
            &ys
        ));
        assert!(!extends_through_reference(
            f,
            &file.types,
            borrowed_element,
            &list
        ));
    }

    #[test]
    fn loan_carrying_types() {
        let int_reference = Type::Reference {
            mutable: false,
            inner: Box::new(Type::Int),
        };
        let function = Type::function(false, vec![], Type::Int);
        assert!(carries_loans(&int_reference));
        assert!(carries_loans(&int_reference.clone().nullable()));
        assert!(carries_loans(&function));
        assert!(!carries_loans(&Type::String));
        assert!(contains_function(&function.clone().nullable()));
        assert!(!contains_function(&int_reference));
        assert!(is_input_borrow(&int_reference.clone().nullable()));
        assert!(!is_input_borrow(&function));
        assert!(is_shared_input_borrow(&int_reference.clone().nullable()));
        let mutable_reference = Type::Reference {
            mutable: true,
            inner: Box::new(Type::Int),
        };
        assert!(!is_shared_input_borrow(&mutable_reference));
        assert!(!is_shared_input_borrow(&Type::Int));
        // A `&Int` cannot borrow a reference; a `&&Int`, a `&(&Int)?` and a function can.
        let int_reference_reference = Type::Reference {
            mutable: false,
            inner: Box::new(int_reference.clone()),
        };
        assert!(!can_borrow_reference(&int_reference));
        assert!(!can_borrow_reference(&mutable_reference.nullable()));
        assert!(can_borrow_reference(&int_reference_reference));
        assert!(can_borrow_reference(&Type::Reference {
            mutable: false,
            inner: Box::new(int_reference.clone().nullable()),
        }));
        assert!(can_borrow_reference(&function));
    }
}
