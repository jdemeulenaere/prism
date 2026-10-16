//! Regression tests for typechecker bug fixes, driven by real Prism source through the parser,
//! the ABI extractor and the checker.

use prism_typechecker::{TypeChecker, TypeError};

/// Parses and checks a single source file, returning its diagnostics.
fn check(src: &str) -> Vec<TypeError> {
    let file = prism_parser::parse_source(src).expect("source must parse");
    let mut checker = TypeChecker::new(file.package.as_ref().map(|p| p.package_string()));
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
