//! Lowering of small programs, checked through the MIR dump and the validator.

/// The MIR of `source`, a file `main` that must typecheck, after checking it is well formed.
fn mir(source: &str) -> String {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main");
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
    drop _4 -> bb1 @1:31
  bb1:
    storage_dead _4 @1:31
    storage_dead _3 @1:31
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
    let actual = mir("fun f(s: String, i: Int) { s; s[i] }\n");
    for line in [
        "read _1 @1:28",
        "_3 = copy _2",
        "read _1[_3]",
        "_0 = const ()",
    ] {
        assert!(actual.contains(line), "missing {:?} in\n{}", line, actual);
    }
}
