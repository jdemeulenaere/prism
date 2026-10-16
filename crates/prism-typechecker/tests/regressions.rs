//! Regression tests for typechecker bug fixes, driven by real Prism source through the parser,
//! the ABI extractor and the checker.

use prism_abi::FileAbi;
use prism_typechecker::{TypeChecker, TypeError};

/// Parses and checks a single source file, returning its diagnostics.
fn check(src: &str) -> Vec<TypeError> {
    let file = prism_parser::parse_source(src).expect("source must parse");
    let mut checker = TypeChecker::new(file.package.as_ref().map(|p| p.package_string()));
    checker.check_module(&file);
    checker.errors
}

/// Checks `app` after ingesting the ABI of each source in `deps`, as `prism check` does for
/// direct imports.
fn check_with_imports(deps: &[&str], app: &str) -> Vec<TypeError> {
    let file = prism_parser::parse_source(app).expect("app must parse");
    let mut checker = TypeChecker::new(file.package.as_ref().map(|p| p.package_string()));
    for dep in deps {
        let dep_file = prism_parser::parse_source(dep).expect("dependency must parse");
        checker.ingest_file_abi(&FileAbi::extract(&dep_file).expect("ABI must extract"));
    }
    checker.check_module(&file);
    checker.errors
}

fn codes(errors: &[TypeError]) -> Vec<&'static str> {
    errors.iter().map(|e| e.code()).collect()
}

#[track_caller]
fn assert_ok(src: &str) {
    let errors = check(src);
    assert!(
        errors.is_empty(),
        "expected no errors, got {:?}\n{}",
        errors,
        src
    );
}

#[track_caller]
fn assert_codes(src: &str, expected: &[&str]) {
    let errors = check(src);
    assert_eq!(codes(&errors), expected, "{:?}\n{}", errors, src);
}

const ORDER: &str = "struct User {\n    var name: String\n}\nstruct Order {\n    var user: User\n    val frozen: User\n}\n";

// Mutable places: every hop of an assignment target must be mutable.

#[test]
fn nested_field_write_through_shared_reference_rejected() {
    assert_codes(
        &format!("{ORDER}fun r(o: &Order) {{\n    o.user.name = \"x\"\n}}\n"),
        &["E4003"],
    );
}

#[test]
fn nested_field_write_through_val_binding_rejected() {
    assert_codes(
        &format!("{ORDER}fun m() {{\n    val o = Order(User(\"a\"), User(\"b\"))\n    o.user.name = \"x\"\n}}\n"),
        &["E4003"],
    );
}

#[test]
fn nested_field_write_through_val_field_rejected() {
    assert_codes(
        &format!("{ORDER}fun r(o: &mut Order) {{\n    o.frozen.name = \"x\"\n}}\n"),
        &["E4003"],
    );
}

#[test]
fn nested_field_write_through_mutable_place_accepted() {
    assert_ok(&format!(
        "{ORDER}fun r(o: &mut Order) {{\n    o.user.name = \"x\"\n}}\nfun m() {{\n    var o = Order(User(\"a\"), User(\"b\"))\n    o.user.name = \"x\"\n    o.user = User(\"c\")\n}}\n"
    ));
}

#[test]
fn field_write_through_mutable_reference_held_in_val_accepted() {
    // A `val` binding holding `&mut User` still grants mutable access to the referent.
    assert_ok(&format!(
        "{ORDER}fun r(u: &mut User) {{\n    val alias = u\n    alias.name = \"x\"\n}}\n"
    ));
}

// Mutable borrows of fields check the base place.

const POINT: &str = "struct P {\n    var x: Int\n}\nfun bump(r: &mut Int) {\n}\n";

#[test]
fn mutable_borrow_of_field_through_shared_reference_rejected() {
    assert_codes(
        &format!("{POINT}fun f(p: &P) {{\n    bump(&mut p.x)\n}}\n"),
        &["E4003"],
    );
}

#[test]
fn mutable_borrow_of_field_of_val_binding_rejected() {
    assert_codes(
        &format!("{POINT}fun g() {{\n    val p = P(1)\n    bump(&mut p.x)\n}}\n"),
        &["E4003"],
    );
}

#[test]
fn mutable_borrow_of_field_of_mutable_place_accepted() {
    assert_ok(&format!(
        "{POINT}fun f(p: &mut P) {{\n    bump(&mut p.x)\n}}\nfun g() {{\n    var p = P(1)\n    bump(&mut p.x)\n}}\n"
    ));
}

// References compare their inner types with assignability, not structural equality.

#[test]
fn borrow_of_constructed_struct_matches_annotated_reference() {
    assert_ok(
        "struct User {\n    var name: String\n}\nfun read(u: &User): String = u.name\nfun write(u: &mut User) {\n    u.name = \"b\"\n}\nfun f(): String {\n    var u = User(\"a\")\n    write(&mut u)\n    return read(&u)\n}\n",
    );
}

#[test]
fn borrow_of_enum_value_matches_annotated_reference() {
    assert_ok(
        "enum Color {\n    Red,\n    Green\n}\nfun read(c: &Color): Int = 1\nfun f(): Int {\n    val c = Color.Red\n    return read(&c)\n}\n",
    );
}

#[test]
fn mutable_reference_stays_invariant() {
    // `&mut Int` must not become `&mut Int?`, or `null` could be written through it.
    assert_codes(
        "fun take(r: &mut Int?) {\n}\nfun f() {\n    var x = 1\n    take(&mut x)\n}\n",
        &["E4002"],
    );
}

// Errors absorb the wrappers around them.

#[test]
fn error_inside_borrow_does_not_cascade() {
    assert_codes(
        &format!("{POINT}fun f(p: &mut P) {{\n    bump(&mut p.nope)\n}}\nfun g(p: &mut P) {{\n    val r: &P = &nope\n}}\n"),
        &["E4006", "E4001"],
    );
}

#[test]
fn error_inside_safe_navigation_does_not_cascade() {
    assert_codes(
        "struct A {\n    val name: String\n}\nfun f(a: A?): Int = a?.nope\n",
        &["E4006"],
    );
}

// Type names are not values.

const USER_AND_ENUM: &str =
    "struct User {\n    val name: String\n}\nenum A {\n    X,\n    Y(Int)\n}\n";

#[test]
fn struct_name_as_value_rejected() {
    assert_codes(
        &format!("{USER_AND_ENUM}fun f(): User = User\n"),
        &["E4002"],
    );
    assert_codes(
        &format!("{USER_AND_ENUM}fun f() {{\n    val u = A\n}}\n"),
        &["E4002"],
    );
}

#[test]
fn field_access_on_struct_name_rejected() {
    assert_codes(
        &format!("{USER_AND_ENUM}fun g(): String = User.name\n"),
        &["E4006"],
    );
}

#[test]
fn enum_name_called_without_variant_rejected() {
    assert_codes(&format!("{USER_AND_ENUM}fun h(): A = A()\n"), &["E4004"]);
}

#[test]
fn calling_a_struct_typed_value_rejected() {
    assert_codes(
        &format!(
            "{USER_AND_ENUM}fun h(): User {{\n    val u = User(\"a\")\n    return u(\"b\")\n}}\n"
        ),
        &["E4004"],
    );
}

#[test]
fn constructors_and_variant_paths_still_work() {
    assert_ok(&format!(
        "{USER_AND_ENUM}fun f(): User = User(\"a\")\nfun g(): A = A.X\nfun h(): A = A.Y(1)\nfun k(a: A): Int = when (a) {{\n    A.X -> 1,\n    else -> 2\n}}\n"
    ));
}

// `package` visibility applies to type names in annotations.

const SECRET_LIB: &str =
    "package a\npackage struct Secret {\n    val code: Int\n}\npublic struct Open {\n    val code: Int\n}\n";

#[test]
fn package_type_in_annotation_from_other_package_rejected() {
    let errors = check_with_imports(
        &[SECRET_LIB],
        "package app\nfun g(s: Secret): Int = s.code\nfun h(): Secret? = null\nfun k() {\n    val s: Secret? = null\n}\nstruct Holder {\n    val s: Secret\n}\n",
    );
    assert_eq!(codes(&errors), ["E4007"; 4], "{:?}", errors);
}

#[test]
fn package_type_in_annotation_within_package_accepted() {
    let errors = check_with_imports(&[SECRET_LIB], "package a\nfun g(s: Secret): Int = s.code\n");
    assert!(errors.is_empty(), "{:?}", errors);
    let errors = check_with_imports(&[SECRET_LIB], "package app\nfun g(s: Open): Int = s.code\n");
    assert!(errors.is_empty(), "{:?}", errors);
}

// Expressions that used to go unchecked.

#[test]
fn elvis_right_side_checked_when_left_side_is_not_nullable() {
    assert_codes("fun f(x: Int): Int = x ?: undefinedThing\n", &["E4001"]);
    assert_codes("fun f(x: Int): Int = x ?: \"s\"\n", &["E4002"]);
    assert_ok("fun f(x: Int): Int = x ?: 0\n");
}

#[test]
fn default_parameter_values_checked() {
    assert_codes("fun f(a: Int, b: Int = \"s\"): Int = a\n", &["E4002"]);
    assert_codes("fun f(b: Int = nope): Int = b\n", &["E4001"]);
    assert_ok("fun f(a: Int, b: Int = a + 1): Int = b\n");
}

#[test]
fn duplicate_closure_parameters_rejected_without_expected_type() {
    assert_codes("fun f() {\n    val g = { x, x -> 1 }\n}\n", &["E4010"]);
}

#[test]
fn parameter_assigned_twice_by_named_and_positional_arguments_rejected() {
    let f = "fun f(a: Int, b: Int = 2): Int = a\n";
    assert_codes(&format!("{f}fun g(): Int = f(a = 1, 5)\n"), &["E4004"]);
    assert_codes(&format!("{f}fun g(): Int = f(5, a = 1)\n"), &["E4004"]);
    assert_ok(&format!("{f}fun g(): Int = f(5, b = 1)\n"));

    let user = "struct User {\n    val name: String\n    val city: String\n}\n";
    // `"b"` re-assigns `name`, so `city` is also missing.
    assert_codes(
        &format!("{user}fun g(): User = User(name = \"a\", \"b\")\n"),
        &["E4004", "E4004"],
    );
    assert_ok(&format!(
        "{user}fun g(): User = User(\"a\", city = \"b\")\n"
    ));
}

// Nullable types: chains flatten and branches join with `null`.

#[test]
fn safe_navigation_chain_through_nullable_field_flattens() {
    assert_ok(
        "struct A {\n    val name: String\n}\nstruct B {\n    val a: A?\n}\nfun f(b: B?): String? = b?.a?.name\nfun g(b: B?): A? = b?.a\n",
    );
}

#[test]
fn branches_join_with_null_into_nullable_type() {
    assert_ok(
        "fun f(c: Bool): Int? {\n    val x = if (c) 1 else null\n    val y = if (c) null else 2\n    val z: Int? = when {\n        c -> 1,\n        else -> null\n    }\n    val w = when {\n        c -> null,\n        else -> 3\n    }\n    return w\n}\n",
    );
    // The joined type is nullable, so it is still not an `Int`.
    assert_codes(
        "fun f(c: Bool): Int {\n    val x = if (c) 1 else null\n    return x\n}\n",
        &["E4002"],
    );
}
