//! Regression tests for typechecker bug fixes, driven by real Prism source through the parser,
//! the ABI extractor and the checker.

use prism_abi::{declared_types, FileAbi};
use prism_typechecker::{TypeChecker, TypeError};

/// Parses and checks a single source file, returning its diagnostics.
fn check(src: &str) -> Vec<TypeError> {
    let file = prism_parser::parse_source(src).expect("source must parse");
    let mut checker = TypeChecker::new(file.package.as_ref().map(|p| p.package_string()), "app");
    checker.check_module(&file);
    checker.errors
}

/// Checks `app` after ingesting the ABI of each source in `deps`, as `prism check` does for
/// direct imports. Dependency `i` gets the module path `dep{i}`.
fn check_with_imports(deps: &[&str], app: &str) -> Vec<TypeError> {
    let modules: Vec<(String, &str)> = deps
        .iter()
        .enumerate()
        .map(|(i, src)| (format!("dep{i}"), *src))
        .collect();
    let modules: Vec<(&str, &str)> = modules.iter().map(|(m, s)| (m.as_str(), *s)).collect();
    check_with_modules(&modules, app)
}

/// Checks `app` (module `app`) after ingesting the ABI of each `(module path, source)` in
/// `deps`, as `prism check` does for direct imports.
fn check_with_modules(deps: &[(&str, &str)], app: &str) -> Vec<TypeError> {
    let imports: Vec<&str> = deps.iter().map(|(module, _)| *module).collect();
    check_workspace(deps, &imports, app)
}

/// Checks `app` (module `app`) in a workspace of `(module path, source)` modules, of which `app`
/// directly imports those in `app_imports`. Each module's ABI binds type names as if it imported
/// every other module, as Phase 1 does for its direct imports.
fn check_workspace(modules: &[(&str, &str)], app_imports: &[&str], app: &str) -> Vec<TypeError> {
    let parsed: Vec<(&str, prism_ast::SourceFile)> = modules
        .iter()
        .map(|(module, src)| {
            (
                *module,
                prism_parser::parse_source(src).expect("module must parse"),
            )
        })
        .collect();
    let exported =
        |file: &prism_ast::SourceFile, name: &str| declared_types(file, true).get(name).copied();

    let file = prism_parser::parse_source(app).expect("app must parse");
    let mut checker = TypeChecker::new(file.package.as_ref().map(|p| p.package_string()), "app");
    for (module, dep_file) in &parsed {
        if !app_imports.contains(module) {
            continue;
        }
        let bind = |name: &str| {
            parsed
                .iter()
                .filter(|(other, _)| other != module)
                .find_map(|(other, f)| exported(f, name).map(|header| (header, other.to_string())))
        };
        checker
            .ingest_file_abi(&FileAbi::extract(dep_file, module, &bind).expect("ABI must extract"));
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
        "struct User {\n    var name: String\n}\nfun read(u: &User): String = u.name.clone()\nfun write(u: &mut User) {\n    u.name = \"b\"\n}\nfun f(): String {\n    var u = User(\"a\")\n    write(&mut u)\n    return read(&u)\n}\n",
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
        &["E4004"],
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
    // The first `x` also has no type to infer from (E4013); the second is the duplicate.
    assert_codes(
        "fun f() {\n    val g = { x, x -> 1 }\n}\n",
        &["E4013", "E4010"],
    );
    // With an expected type, only the duplicate is reported.
    assert_codes(
        "fun run(f: (Int, Int) -> Int): Int = f(1, 2)\nfun g(): Int = run { x, x -> 1 }\n",
        &["E4010"],
    );
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
        "struct A : Clone {\n    val name: String\n}\nstruct B {\n    val a: A?\n}\nfun f(b: B?): String? = b?.a?.name?.clone()\nfun g(b: B?): A? = b?.a?.clone()\n",
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
        &["E4005"],
    );
}

// `when`: exhaustiveness, statement position and borrowed subjects.

#[test]
fn variant_pattern_with_payload_value_does_not_cover_variant() {
    assert_codes(
        "enum Shape {\n    Circle(Int),\n    Square\n}\nfun f(s: Shape): Int = when (s) {\n    Shape.Circle(1) -> 1,\n    Shape.Square -> 2\n}\n",
        &["E4009"],
    );
}

#[test]
fn value_named_like_a_variant_does_not_cover_variant() {
    assert_codes(
        "enum Color {\n    Red,\n    Green\n}\nstruct H {\n    val Green: Color\n}\nfun f(c: Color, h: H): Int = when (c) {\n    Color.Red -> 1,\n    h.Green -> 2\n}\n",
        &["E4009"],
    );
}

#[test]
fn when_statement_does_not_require_else() {
    assert_ok(
        "fun g() {\n}\nfun f(x: Int) {\n    when (x) {\n        1 -> g()\n    }\n    val y = 2\n}\nfun h(x: Int) {\n    when (x) {\n        1 -> g()\n    }\n}\n",
    );
    // In value position it still does.
    assert_codes(
        "fun f(x: Int): Int {\n    val y = when (x) {\n        1 -> 1\n    }\n    return y\n}\n",
        &["E4009"],
    );
}

#[test]
fn when_over_borrowed_enum_matches_its_variants() {
    assert_ok(
        "enum Color {\n    Red,\n    Green\n}\nfun f(c: &Color): Int = when (c) {\n    Color.Red -> 1,\n    Color.Green -> 2\n}\nfun g(c: &mut Color): Int = when (c) {\n    Red -> 1,\n    Green -> 2\n}\n",
    );
}

// Enum variants are namespaced by their enum.

const TWO_ENUMS: &str =
    "enum A {\n    None,\n    Some(Int)\n}\nenum B {\n    None,\n    Other\n}\n";

#[test]
fn same_named_variants_in_two_enums_accepted() {
    assert_ok(&format!(
        "{TWO_ENUMS}fun f(a: A): Int = when (a) {{\n    None -> 0,\n    Some(1) -> 1,\n    else -> 2\n}}\nfun g(b: B): Int = when (b) {{\n    None -> 0,\n    Other -> 1\n}}\nfun h(): B = B.None\nfun k(): A = Some(3)\n"
    ));
}

#[test]
fn ambiguous_bare_variant_outside_when_pattern_rejected() {
    assert_codes(&format!("{TWO_ENUMS}fun h(): B = None\n"), &["E4001"]);
}

#[test]
fn imported_enum_variant_named_like_a_local_variant_accepted() {
    let errors = check_with_imports(
        &["package lib\npublic enum Opt {\n    None,\n    Has(Int)\n}\n"],
        "package app\nenum Mode {\n    None,\n    Fast\n}\nfun f(o: Opt): Int = when (o) {\n    Opt.None -> 0,\n    else -> 1\n}\nfun g(): Mode = Mode.None\n",
    );
    assert!(errors.is_empty(), "{:?}", errors);
}

// Parameterless closures and composable function types (harden-parser-grammar).

#[test]
fn trailing_lambda_passed_to_a_non_function_parameter_rejected() {
    assert_codes(
        "fun takesInt(n: Int) { }\nfun f() { takesInt { 42 } }\n",
        &["E4004"],
    );
}

#[test]
fn parameterless_trailing_lambda_checked_against_its_parameter_type() {
    assert_ok("fun run(body: () -> Int): Int = body()\nfun f(): Int = run { 1 }\n");
}

#[test]
fn non_composable_trailing_lambda_inside_a_compose_fun() {
    assert_codes(
        "compose fun Child() { }\nfun later(f: () -> Unit) { }\ncompose fun P() { later { Child() } }\n",
        &["E4008"],
    );
}

#[test]
fn composable_content_lambda_may_call_composables() {
    assert_ok(
        "compose fun Item() { }\ncompose fun Column(content: compose () -> Unit) { content() }\ncompose fun Screen() { Column { Item() } }\n",
    );
}

#[test]
fn composable_function_type_is_not_a_plain_function_type() {
    assert_codes(
        "fun later(f: () -> Unit) { }\ncompose fun P(content: compose () -> Unit) { later(content) }\n",
        &["E4004"],
    );
}

#[test]
fn declaration_as_an_unbraced_branch_body_never_reaches_the_checker() {
    let src = "fun f(c: Bool): Int {\n    if (c) val x = 1\n    return x\n}\n";
    let err = prism_parser::parse_source(src).expect_err("a declaration is not a branch body");
    assert_eq!(err.code, "E2001");
}

// Canonical nominal identity (typechecker-soundness D3).

#[track_caller]
fn assert_import_codes(deps: &[(&str, &str)], app: &str, expected: &[&str]) {
    let errors = check_with_modules(deps, app);
    assert_eq!(codes(&errors), expected, "{:?}\n{}", errors, app);
}

#[test]
fn builtin_range_distinct_from_a_user_struct_named_range() {
    assert_codes(
        "struct Range {\n    val a: Int\n    val b: String\n}\nfun f(): String {\n    val r: Range = 1..10\n    return r.b.clone()\n}\n",
        &["E4002"],
    );
    // Without a user declaration, `Range` names the built-in type.
    assert_ok("fun f(): Int {\n    val r: Range = 1..10\n    return r.start\n}\n");
}

#[test]
fn struct_and_enum_never_equal() {
    assert_codes(
        "struct S {\n    val x: Int\n}\nenum E {\n    A\n}\nfun f() {\n    val s: S = E.A\n}\n",
        &["E4002"],
    );
    // A struct and an enum sharing a name across modules stay distinct.
    assert_import_codes(
        &[
            ("m/kinds", "package m\npublic enum Kind {\n    A\n}\npublic fun k(): Kind = Kind.A\n"),
            ("n/kinds", "package n\npublic struct Kind {\n    val x: Int\n}\npublic fun takeStruct(v: Kind): Int = v.x\n"),
        ],
        "package app\nfun f(): Int = takeStruct(k())\n",
        &["E4004"],
    );
}

#[test]
fn annotation_and_constructor_denote_the_same_type() {
    assert_ok(
        "struct User {\n    val name: String\n}\nfun read(u: User): String = u.name.clone()\nfun f(): String {\n    val a: User = User(\"x\")\n    return read(a)\n}\n",
    );
}

#[test]
fn same_named_structs_from_different_modules_stay_distinct() {
    let a = "package a\npublic struct User {\n    val name: String\n}\npublic struct Order {\n    val user: User\n}\n";
    let b = "package b\npublic struct User {\n    val id: Int\n}\n";
    assert_import_codes(
        &[("b/lib", b), ("a/lib", a)],
        "package app\nfun g(o: Order): Int = o.user.id\n",
        &["E4006"],
    );
    assert_import_codes(
        &[("b/lib", b), ("a/lib", a)],
        "package app\nfun g(o: Order): String = o.user.name.clone()\n",
        &[],
    );
}

#[test]
fn same_named_types_are_qualified_in_mismatch_messages() {
    let errors =
        check("struct Range {\n    val a: Int\n}\nfun f() {\n    val r: Range = 1..10\n}\n");
    assert_eq!(
        errors[0].message(),
        "type mismatch: expected 'app::Range', found '<builtin>::Range'"
    );
}

// Return and argument mismatch codes (typechecker-soundness D2).

#[test]
fn returned_value_of_the_wrong_type_reports_e4005() {
    assert_codes("fun f(): Int {\n    return \"s\"\n}\n", &["E4005"]);
    assert_codes("fun f(): Int = \"s\"\n", &["E4005"]);
    // The final expression of a non-Unit block body is the returned value.
    assert_codes("fun f(): Int {\n    \"s\"\n}\n", &["E4005"]);
    // A closure body is the closure's returned value.
    assert_codes(
        "fun run(f: (Int) -> Int): Int = f(1)\nfun g(): Int = run { x -> \"s\" }\n",
        &["E4005"],
    );
}

#[test]
fn returning_a_value_from_a_unit_function_reports_e4005() {
    assert_codes("fun f() {\n    return 5\n}\n", &["E4005"]);
}

#[test]
fn mismatch_inside_a_returned_branch_reports_e4005() {
    assert_codes(
        "fun f(c: Bool): Int {\n    return if (c) \"a\" else 1\n}\n",
        &["E4005"],
    );
    assert_codes(
        "fun f(c: Bool): Int = when {\n    c -> \"a\"\n    else -> 1\n}\n",
        &["E4005"],
    );
}

#[test]
fn argument_of_the_wrong_type_reports_e4004() {
    let f = "fun f(a: Int): Int = a\nstruct P {\n    val x: Int\n}\nenum O {\n    Some(Int)\n}\n";
    assert_codes(&format!("{f}fun g(): Int = f(\"s\")\n"), &["E4004"]);
    assert_codes(&format!("{f}fun g(): Int = f(a = \"s\")\n"), &["E4004"]);
    assert_codes(&format!("{f}fun g(): P = P(\"s\")\n"), &["E4004"]);
    assert_codes(&format!("{f}fun g(): O = O.Some(\"s\")\n"), &["E4004"]);
    // A mismatch inside an argument's branch is still an argument mismatch.
    assert_codes(
        &format!("{f}fun g(c: Bool): Int = f(if (c) \"s\" else 1)\n"),
        &["E4004"],
    );
}

#[test]
fn other_expectations_keep_the_generic_mismatch_code() {
    assert_codes("fun f() {\n    val x: Int = \"s\"\n}\n", &["E4002"]);
    assert_codes("fun f(a: Int, b: Int = \"s\"): Int = a\n", &["E4002"]);
    // A local initializer inside a returned block resets the origin.
    assert_codes(
        "fun f(): Int {\n    val x: Int = \"s\"\n    return 1\n}\n",
        &["E4002"],
    );
    // An operand inside an argument resets the origin too.
    assert_codes(
        "fun f(a: Bool): Int = 1\nfun g(): Int = f(!\"s\")\n",
        &["E4002"],
    );
}

#[test]
fn type_name_in_return_position_is_not_a_mismatch() {
    assert_codes(
        "struct User {\n    val name: String\n}\nfun f(): User = User\n",
        &["E4002"],
    );
}

// Operators read through references (typechecker-soundness D1).

#[test]
fn arithmetic_on_a_borrowed_integer() {
    assert_ok("fun f(a: &Int): Int = a + 1\n");
    assert_ok("fun f(a: &mut Int, b: &Int): Int = a * b - -a\n");
    assert_ok("fun f(s: &String): String = s + \"!\"\n");
}

#[test]
fn equality_and_comparison_between_a_borrow_and_a_value() {
    assert_ok("fun f(a: &Int, b: Int): Bool = a == b\n");
    assert_ok("fun f(a: &Int, b: &mut Int): Bool = a < b && a != 0\n");
}

#[test]
fn borrowed_boolean_as_a_condition() {
    assert_ok("fun f(c: &Bool): Int = if (c) 1 else 2\n");
    assert_ok("fun f(c: &Bool, d: &mut Bool): Bool = !c || d\n");
    assert_ok("fun f(c: &Bool): Int = when {\n    c -> 1\n    else -> 2\n}\n");
}

// A `Copy` value is copied out of a borrow where a value is expected (`references`, "Copy-out of
// a borrowed primitive"); a non-`Copy` one is a move out of the borrow (`type-system`, "Borrow
// passed where a value is expected").
#[test]
fn borrow_passed_where_a_value_is_expected() {
    assert_ok("fun g(x: Int): Int = x\nfun f(a: &Int): Int = g(a)\n");
    assert_ok("fun f(a: &Int): Int = a\n");
    assert_ok("fun f(a: &Int) {\n    val x: Int = a\n}\n");
    assert_codes(
        "fun g(s: String): Int = 1\nfun f(a: &String): Int = g(a)\n",
        &["E4017"],
    );
    assert_codes("fun f(a: &String): String = a\n", &["E4017"]);
    assert_codes(
        "fun f(a: &String) {\n    val x: String = a\n}\n",
        &["E4017"],
    );
}

#[test]
fn operators_do_not_read_through_heap() {
    assert_codes("fun f(h: Heap<Int>): Int = h + 1\n", &["E4002"]);
}

// Order-independent inference of private declarations (typechecker-soundness D5).

#[test]
fn use_before_an_inferred_function_declaration() {
    let before = "fun g(): String {\n    val s: String = h()\n    return s\n}\nfun h() = 42\n";
    let after = "fun h() = 42\nfun g(): String {\n    val s: String = h()\n    return s\n}\n";
    assert_codes(before, &["E4002"]);
    assert_codes(after, &["E4002"]);
    assert_ok("fun g(): Int = h() + 1\nfun h() = 42\n");
}

#[test]
fn use_before_an_inferred_top_level_value() {
    assert_codes("fun g(): String = top\nval top = 42\n", &["E4005"]);
    assert_ok("fun g(): Int = top\nval top = h()\nfun h() = 42\n");
}

#[test]
fn recursive_inferred_function_reports_e4013() {
    assert_codes(
        "fun fact(n: Int) = if (n < 1) 1 else n * fact(n - 1)\n",
        &["E4013"],
    );
    // Mutual recursion through two inferred functions.
    assert_codes("fun a() = b()\nfun b() = a()\n", &["E4013"]);
    // A value whose initializer reads itself.
    assert_codes("val v = v + 1\n", &["E4013"]);
    // An explicit return type breaks the cycle.
    assert_ok("fun fact(n: Int): Int = if (n < 1) 1 else n * fact(n - 1)\n");
}

#[test]
fn errors_in_an_inferred_body_reported_once() {
    assert_codes(
        "fun g(): Int = h()\nfun k(): Int = h()\nfun h() = undefinedThing\n",
        &["E4001"],
    );
}

#[test]
fn inferred_body_does_not_see_the_caller_locals() {
    // `h` is inferred while `g`'s body is being checked, but `x` is local to `g`.
    assert_codes(
        "fun g(): Int {\n    val x = 1\n    return h()\n}\nfun h() = x\n",
        &["E4001"],
    );
}

// Closure parameter inference (typechecker-soundness D6).

#[test]
fn unannotated_closure_parameter_without_an_expected_type() {
    let errors = check("fun f() {\n    val g = { x -> x + 1 }\n}\n");
    assert_eq!(codes(&errors), ["E4013"]);
    assert!(
        errors[0].message().contains("'x'"),
        "{}",
        errors[0].message()
    );
}

#[test]
fn calls_through_an_uninferred_closure_are_not_silently_accepted() {
    assert_codes(
        "fun f() {\n    val g = { x -> x + 1 }\n    val s: String = g(\"str\")\n}\n",
        &["E4013"],
    );
}

#[test]
fn closure_against_an_expected_function_type_still_inferred() {
    assert_ok("fun run(f: (Int) -> Int): Int = f(1)\nfun g(): Int = run { x -> x + 1 }\n");
    // A parameterless closure has nothing to infer.
    assert_ok("fun f() {\n    val g = { 1 }\n}\n");
}

// Ambiguous imported names (typechecker-soundness D7).

const HELPER_A: (&str, &str) = (
    "a/lib",
    "package a\npublic fun helper(x: Int): Int = x\npublic struct User {\n    val id: Int\n}\n",
);
const HELPER_B: (&str, &str) = ("b/lib", "package b\npublic fun helper(x: String): Int = 1\npublic struct User {\n    val name: String\n}\n");

#[test]
fn using_a_name_exported_by_two_imports() {
    let errors = check_with_modules(
        &[HELPER_A, HELPER_B],
        "package app\nfun g(): Int = helper(\"s\")\n",
    );
    assert_eq!(codes(&errors), ["E4012"], "{:?}", errors);
    let message = errors[0].message();
    assert!(
        message.contains("/a/lib") && message.contains("/b/lib"),
        "{}",
        message
    );
}

#[test]
fn unused_colliding_names_accepted() {
    assert_import_codes(
        &[HELPER_A, HELPER_B],
        "package app\nfun g(): Int = 1\n",
        &[],
    );
}

#[test]
fn ambiguous_type_name_in_an_annotation() {
    assert_import_codes(
        &[HELPER_A, HELPER_B],
        "package app\nfun f(u: User): Int = 1\n",
        &["E4012"],
    );
}

#[test]
fn local_declaration_colliding_with_an_import() {
    // The local declaration shadows the import: `helper("s")` resolves to it.
    assert_import_codes(
        &[HELPER_A],
        "package app\nfun helper(x: String): Int = 1\nfun g(): Int = helper(\"s\")\n",
        &[],
    );
}

#[test]
fn local_declaration_resolving_an_ambiguity() {
    // Both imports export `helper` and `User`; the local declarations win, so no E4012.
    assert_import_codes(
        &[HELPER_A, HELPER_B],
        "package app\nfun helper(x: Int): Int = x\nstruct User {\n    val age: Int\n}\nfun g(u: User): Int = helper(u.age)\n",
        &[],
    );
}

#[test]
fn same_module_imported_twice_is_not_ambiguous() {
    assert_import_codes(
        &[HELPER_A, HELPER_A],
        "package app\nfun g(): Int = helper(1)\n",
        &[],
    );
}

// Discarded block values (typechecker-soundness D8).

#[test]
fn unit_function_ending_in_a_non_unit_call() {
    assert_ok("fun g(): Int = 1\nfun f() {\n    g()\n}\n");
    assert_ok("fun g(): Int = 1\nfun f(): Unit {\n    g()\n}\n");
}

#[test]
fn unit_closure_ending_in_a_non_unit_expression() {
    assert_ok("fun run(f: (Int) -> Unit) { }\nfun g() {\n    run { x -> x + 1 }\n}\n");
    assert_ok("fun later(f: () -> Unit) { }\nfun g() {\n    later { 42 }\n}\n");
}

#[test]
fn errors_in_a_discarded_expression_still_reported() {
    assert_codes("fun f() {\n    undefinedThing\n}\n", &["E4001"]);
    assert_codes(
        "fun run(f: (Int) -> Unit) { }\nfun g() {\n    run { x -> x + \"s\" }\n}\n",
        &["E4002"],
    );
}

#[test]
fn expression_body_of_a_unit_function_still_checked() {
    assert_codes("fun g(): Int = 1\nfun f(): Unit = g()\n", &["E4005"]);
}

// Imported signatures keep their declared types (typechecker-soundness D4).

const HIGHER_ORDER_LIB: (&str, &str) = (
    "lib/h",
    "package lib\npublic fun hof(f: ((Int) -> Int) -> Int): Int = f { k -> k + 1 }\npublic fun pick(f: (Int) -> Int?): Int = 1\n",
);

#[test]
fn higher_order_parameter_type_across_files() {
    assert_import_codes(
        &[HIGHER_ORDER_LIB],
        "package app\nfun g(): Int = hof { k -> k(1) }\n",
        &[],
    );
    // `k` is a function, so it cannot be used as an `Int`.
    assert_import_codes(
        &[HIGHER_ORDER_LIB],
        "package app\nfun g(): Int = hof { k -> k + 1 }\n",
        &["E4002"],
    );
}

#[test]
fn function_type_returning_a_nullable_across_files() {
    assert_import_codes(
        &[HIGHER_ORDER_LIB],
        "package app\nfun p(): Int = pick { x -> null }\n",
        &[],
    );
}

#[test]
fn imported_signature_names_resolve_in_the_exporter() {
    // `svc` binds `Item` to `model/types`; the importer imports both, so the types match.
    let model = (
        "model/types",
        "package model\npublic struct Item {\n    val price: Int\n}\n",
    );
    let svc = ("a/svc", "package a\npublic fun first(): Item = Item(1)\n");
    assert_import_codes(
        &[svc, model],
        "package app\nfun g(): Int = first().price\nfun h(i: Item): Int = i.price\nfun k(): Int = h(first())\n",
        &[],
    );
}

// Members of a type require importing its module (typechecker-soundness D9).

const SHOP_MODELS: (&str, &str) = (
    "shop/models",
    "package shop\npublic struct Order {\n    var price: Int\n}\npublic enum Kind {\n    Retail,\n    Bulk\n}\n",
);
const SHOP_API: (&str, &str) = (
    "shop/api",
    "package shop\nimport models\npublic fun first(): Order = Order(1)\npublic fun send(o: Order): Unit { }\npublic fun kind(): Kind = Kind.Retail\n",
);

#[track_caller]
fn assert_shop_codes(app_imports: &[&str], app: &str, expected: &[&str]) -> Vec<TypeError> {
    let errors = check_workspace(&[SHOP_MODELS, SHOP_API], app_imports, app);
    assert_eq!(codes(&errors), expected, "{:?}\n{}", errors, app);
    errors
}

#[test]
fn field_access_on_a_type_from_an_unimported_module() {
    let errors = assert_shop_codes(
        &["shop/api"],
        "package app\nfun g(): Int = first().price\n",
        &["E4014"],
    );
    let message = errors[0].message();
    for part in ["'Order'", "shop/models", "import /shop/models"] {
        assert!(message.contains(part), "{}", message);
    }
}

#[test]
fn passing_the_value_along_without_touching_its_members() {
    assert_shop_codes(
        &["shop/api"],
        "package app\nfun keep(): Unit {\n    send(first())\n}\nfun copy() {\n    val o = first()\n    send(o)\n}\n",
        &[],
    );
}

#[test]
fn importing_the_defining_module() {
    assert_shop_codes(
        &["shop/api", "shop/models"],
        "package app\nfun g(): Int = first().price\n",
        &[],
    );
}

#[test]
fn naming_an_unimported_type_in_an_annotation() {
    // Reported at the annotation, with the import hint, and nowhere else.
    let errors = assert_shop_codes(
        &["shop/api"],
        "package app\nfun total(o: Order): Int = o.price\n",
        &["E4014"],
    );
    let message = errors[0].message();
    for part in ["'Order'", "shop/models", "import /shop/models"] {
        assert!(message.contains(part), "{}", message);
    }
    // Every annotation site: local, return type, struct field.
    assert_shop_codes(
        &["shop/api"],
        "package app\nfun g() {\n    val o: Order = first()\n}\n",
        &["E4014"],
    );
    assert_shop_codes(
        &["shop/api"],
        "package app\nfun g(): Order = first()\n",
        &["E4014"],
    );
    assert_shop_codes(
        &["shop/api"],
        "package app\nstruct Box {\n    val o: Order\n}\n",
        &["E4014"],
    );
    // Importing the defining module makes the name usable.
    assert_shop_codes(
        &["shop/api", "shop/models"],
        "package app\nfun total(o: Order): Int = o.price\n",
        &[],
    );
}

#[test]
fn writing_matching_or_constructing_an_unimported_type() {
    // Field assignment.
    assert_shop_codes(
        &["shop/api"],
        "package app\nfun g() {\n    var o = first()\n    o.price = 2\n}\n",
        &["E4014"],
    );
    // Matching on its variants.
    assert_shop_codes(
        &["shop/api"],
        "package app\nfun g(): Int = when (kind()) {\n    else -> 1\n}\n",
        &["E4014"],
    );
    // Constructing it or naming a variant.
    assert_shop_codes(
        &["shop/api"],
        "package app\nfun g() {\n    send(Order(1))\n}\n",
        &["E4014"],
    );
    assert_shop_codes(
        &["shop/api"],
        "package app\nfun g(): Int = 1\nval k = Kind.Bulk\n",
        &["E4014"],
    );
    assert_shop_codes(
        &["shop/api"],
        "package app\nfun g(): Int = when (kind()) {\n    Kind.Retail -> 1\n    else -> 2\n}\n",
        // Reported once, at the subject: patterns are not checked against an unusable enum.
        &["E4014"],
    );
}

// --- Extension functions (`extension-methods`)

const EXTENSION_MODELS: &str = "public struct User {\n    val name: String\n}\n";
const EXTENSION_UI: &str = "public fun User.shout(): String = \"!\"\n";

#[test]
fn an_imported_extension_is_not_a_bare_function() {
    let deps = [("models", EXTENSION_MODELS), ("ui", EXTENSION_UI)];
    // Spec "Extension Calls Resolve Through the File and Its Imports": an extension is reached
    // through its receiver only.
    assert_eq!(
        codes(&check_with_modules(
            &deps,
            "fun f(u: User): String = shout()\n"
        )),
        vec!["E4001"]
    );
    assert!(check_with_modules(&deps, "fun f(u: User): String = u.shout()\n").is_empty());
}

#[test]
fn a_recursive_unannotated_extension_cannot_be_inferred() {
    assert_codes(
        "struct User {\n    val n: Int\n}\nfun User.count() = count()\n",
        &["E4013"],
    );
}

#[test]
fn unannotated_extensions_are_inferred_per_receiver() {
    assert_ok(
        "struct User {\n    val n: Int\n}\nstruct Team {\n    val n: Int\n}\nfun f(u: User, t: Team) {\n    val a: Int = u.size()\n    val b: String = t.size()\n}\nfun User.size() = 1\nfun Team.size() = \"x\"\n",
    );
}

#[test]
fn a_field_through_implicit_this_is_not_a_top_level_value() {
    // The field `id` hides the top-level `val id`, so returning it moves a field of `this`.
    assert_codes(
        "val id = \"global\"\nstruct User {\n    val id: String\n}\nfun User.key(): String = id\n",
        &["E4017"],
    );
    assert_ok("val id = \"global\"\nstruct User {\n    val id: String\n}\nfun User.key(): &String = &id\n");
}

#[test]
fn this_cannot_be_reassigned() {
    assert_codes(
        "struct User {\n    val n: Int\n}\nmut fun User.f(other: &mut User) {\n    this = other\n}\n",
        &["E4003"],
    );
}
