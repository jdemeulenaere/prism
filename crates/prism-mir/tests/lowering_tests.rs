//! Lowering of small programs, checked through the MIR dump and the validator.

/// The MIR of `source`, a file `main` that must typecheck, after checking it is well formed.
fn mir(source: &str) -> String {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main")
        .with_prelude(prism_typechecker::test_prelude());
    let tree = checker.check_module(&file);
    assert!(
        checker.errors.is_empty(),
        "type errors: {:?}",
        checker.errors
    );
    let mir = prism_mir::lower_file(&tree);
    let violations = mir.validate();
    assert!(
        violations.is_empty(),
        "{}\n{}",
        violations.join("\n"),
        mir.dump()
    );
    mir.dump()
}

#[test]
fn straight_line_function() {
    let expected = "\
module main
fun main::f(_1: Int, _2: String): Int @1:1
  local _0: Int (return)
  local _1 a: Int (argument)
  local _2 s: String (argument)
  local _3 n: Int (user)
  local _4 t: String (user)
  bb0:
    storage_live _3 @1:33
    _3 = copy _1 + const 1 @1:41
    storage_live _4 @1:48
    _4 = move _2 @1:56
    _0 = copy _3 @1:59
    drop _4 -> bb1 @1:61
  bb1:
    storage_dead _4 @1:61
    storage_dead _3 @1:61
    drop _2 -> bb2 @1:1
  bb2:
    return @1:1
";
    let actual = mir("fun f(a: Int, s: String): Int { val n = a + 1; val t = s; n }\n");
    assert_eq!(actual, expected, "\n{}", actual);
}

#[test]
fn values_constructions_and_assignments() {
    let actual = mir("\
struct User { val id: Int, var name: String }
val LIMIT = 3
fun f(u: &mut User): Int {
    u.name = \"b\"
    var x = LIMIT
    x = x * 2
    val v = User(id = x, name = \"a\")
    val r = 0..<x
    return v.id
}
");
    for line in [
        "val main::LIMIT: Int @2:1",
        "_0 = const 3",
        "drop (*_1).name -> bb1 @4:5",
        "(*_1).name = const \"b\" @4:5",
        "_3 = const &main::LIMIT",
        "_2 = copy (*_3)",
        "_2 = copy _2 * const 2",
        "_4 = struct main::User(copy _2, const \"a\")",
        "_5 = struct <builtin>::Range(const 0, copy _2, const false)",
        "_0 = copy _4.id",
    ] {
        assert!(actual.contains(line), "missing {:?} in\n{}", line, actual);
    }
}

#[test]
fn borrowed_temporary_of_a_val_lives_with_its_binding() {
    let actual = mir("fun f(): Int { val r = &\"x\"; return 1 }\n");
    for line in [
        "local _1: String (temporary)",
        "local _2 r: &String (user)",
        "_1 = const \"x\"",
        "_2 = &_1 (written)",
    ] {
        assert!(actual.contains(line), "missing {:?} in\n{}", line, actual);
    }
    // `r` is marked dead before its temporary is dropped.
    let r_dead = actual.find("storage_dead _2").unwrap();
    let drop = actual.find("drop _1").unwrap();
    assert!(r_dead < drop, "{}", actual);
}

#[test]
fn discarded_places_are_read_and_unit_bodies_return_unit() {
    // A string's character is computed by the built-in `get`, not a place, so it is a call
    // whose result is discarded.
    let actual = mir("fun f(s: String, i: Int) { s; s[i] }\n");
    for line in [
        "read _1 @1:28",
        "_3 = copy _2",
        "_5 = call builtin get(move _4, move _3)",
        "_0 = const ()",
    ] {
        assert!(actual.contains(line), "missing {:?} in\n{}", line, actual);
    }
}

#[test]
fn empty_ranges_and_do_while_loops_are_well_formed() {
    let actual = mir("\
fun g(i: Int) {}
fun f(c: Bool) {
    for (i in 0..<0) g(i)
    do { val s = \"x\"; if (c) continue } while (c)
}
");
    // A half-open range is empty when its start is not below its end, and its last element is
    // computed only once it is known not to be empty.
    assert!(
        actual.contains("copy _2.start >= copy _2.end"),
        "{}",
        actual
    );
    assert!(actual.contains("copy _2.end - const 1"), "{}", actual);
}

#[test]
fn loops_over_a_range_value_read_its_end_inclusive_field() {
    let actual = mir("\
fun g(i: Int) {}
fun f(r: Range, c: Bool) { for (i in r) { if (c) continue; g(i) } }
");
    assert!(actual.contains("switch copy _3.endInclusive"), "{}", actual);
}

#[test]
fn string_subjects_and_chains_are_compared_through_borrows() {
    let actual = mir("\
struct User { val name: String }
fun f(s: String): Int = when (s) { \"a\" -> 1; else -> 2 }
fun g(u: User?): Bool = u?.name == \"a\"
fun h(a: String, b: String): String = a + b
");
    assert!(actual.contains("= &_1 (operator)"), "{}", actual);
    assert!(
        actual.contains("= &(_1 as payload).name (operator)"),
        "{}",
        actual
    );
    assert!(actual.contains("_0 = move _3 + move _4"), "{}", actual);
}

#[test]
fn repeated_variants_nullable_subjects_and_jumps_are_well_formed() {
    let actual = mir("\
enum Color { Red, Green }
fun f(c: Color): Int = when (c) { Red -> 1; Red -> 2; Green -> 3 }
fun g(c: Color): Int = when (c) { Red, Green -> 1; else -> 2 }
fun h(s: String?): Int = when (s) { null -> 0; \"a\" -> 1; else -> 2 }
fun k(x: Int) { var i = 0; while (i < x) { i = i + 1; if (i == 2) continue; if (i == 5) break } }
");
    // The second `Red` branch is never taken, so it has no block.
    assert!(!actual.contains("const 2 @2:"), "{}", actual);
    assert!(
        actual.contains("[0 -> bb1, 1 -> bb1, otherwise -> bb2]"),
        "{}",
        actual
    );
}

/// The MIR of `source` as data, a file `main` that must typecheck.
fn lowered(source: &str) -> prism_mir::MirFile {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main")
        .with_prelude(prism_typechecker::test_prelude());
    let tree = checker.check_module(&file);
    assert!(checker.errors.is_empty(), "{:?}", checker.errors);
    prism_mir::lower_file(&tree)
}

#[test]
fn receivers_are_known_from_the_callee_and_the_owner() {
    use prism_mir::TerminatorKind;
    let file = lowered(
        "\
struct Team { var lead: String }
fun Team.captain(o: &String): &String = &lead
fun first(a: &String): &String = a
fun f(t: Team, s: String, g: (&String) -> Int): Int {
    val c = t.captain(&s)
    val d = first(&s)
    val e = t.lead.clone()
    val h = { 1 }
    return g(&s)
}
",
    );
    let mut owners = Vec::new();
    let mut calls = Vec::new();
    for (_, body) in file.bodies.iter() {
        owners.push((format!("{:?}", body.owner), body.owner.has_receiver()));
        for (_, block) in body.blocks.iter() {
            if let TerminatorKind::Call { callee, .. } = &block.terminator.kind {
                calls.push(callee.has_receiver());
            }
        }
    }
    let receivers: Vec<bool> = owners.iter().map(|(_, receiver)| *receiver).collect();
    // `Team.captain`, `first`, `f` and its closure.
    assert_eq!(receivers, vec![true, false, false, false], "{:?}", owners);
    // `t.captain(&s)`, `first(&s)`, `clone()`, `g(&s)`.
    assert_eq!(calls, vec![true, false, true, false]);
}

// --- Generic Bodies Are Lowered Once (the `mir` spec)

#[test]
fn dropping_a_type_parameter_value() {
    let actual = mir("fun <T> discard(x: T) {}\nfun <T : Copy> keep(x: T): Int = 1\n");
    // `discard` records `T` and drops its argument; a `Copy` value is never dropped.
    assert!(
        actual.contains("fun <T> main::discard(_1: T): Unit"),
        "{}",
        actual
    );
    assert!(actual.contains("drop _1 -> bb1"), "{}", actual);
    let keep = &actual[actual.find("fun <T : Copy> main::keep").expect("keep")..];
    assert!(!keep.contains("drop"), "{}", keep);
}

#[test]
fn a_call_with_type_arguments() {
    let actual = mir("fun <T> id(x: T): T = x\nfun f(s: String): String = id(s)\n");
    assert!(
        actual.contains("= call main::id<String>(move _"),
        "{}",
        actual
    );
}

#[test]
fn defaults_and_closures_of_a_generic_function_share_its_type_parameters() {
    let file = prism_parser::parse_source(
        "fun <T> pass(x: T, f: () -> Int = { 1 }, n: Int = 2): T = x\nfun g(): Int = pass(3)\n",
    )
    .expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main")
        .with_prelude(prism_typechecker::test_prelude());
    let tree = checker.check_module(&file);
    assert!(checker.errors.is_empty(), "{:?}", checker.errors);
    let mir = prism_mir::lower_file(&tree);
    assert!(mir.validate().is_empty(), "{:?}", mir.validate());
    let generic: Vec<(String, usize)> = mir
        .bodies
        .iter()
        .map(|(id, body)| (mir.body_name(id), body.type_parameters.len()))
        .collect();
    assert_eq!(
        generic,
        vec![
            ("main::pass".to_string(), 1),
            ("main::pass f".to_string(), 1),
            ("main::pass f#closure0".to_string(), 1),
            ("main::pass n".to_string(), 1),
            ("main::g".to_string(), 0),
        ]
    );
}

#[test]
fn a_destructuring_that_misses_a_part_or_mistypes_a_target_is_reported() {
    use prism_mir::StatementKind;
    let source = "enum Pair { Of(String, String, Int) }\nfun f(p: Pair): String = when (p) { Pair.Of(val a, _, val n) -> a }\n";
    // The well-formed MIR of the scenario "Destructuring with an unbound part".
    mir(source);
    type Parts = Vec<(usize, prism_mir::LocalId)>;
    let edit = |change: &dyn Fn(&mut Parts)| {
        let mut file = lowered(source);
        for (_, body) in file.bodies.iter_mut() {
            for (_, block) in body.blocks.iter_mut() {
                for statement in &mut block.statements {
                    if let StatementKind::Destructure { parts, .. } = &mut statement.kind {
                        change(parts);
                    }
                }
            }
        }
        file.validate().join("\n")
    };
    let missing = edit(&|parts| parts.retain(|(index, _)| *index != 1));
    assert!(
        missing.contains("lists part 1 of _1 0 times"),
        "{}",
        missing
    );
    // `_4` is `n`, an `Int`.
    let mistyped = edit(&|parts| parts[0].1 = prism_mir::LocalId(4));
    assert!(
        mistyped.contains("moves part 0 (a String) into _4, a Int"),
        "{}",
        mistyped
    );
}
