//! Drop elaboration on small files lowered from source, checked through the dump of the
//! elaborated MIR, the validator and `verify_elaborated`.

use prism_borrowck::{check_file, verify_elaborated};
use prism_mir::{MirFile, MirPhase, StatementKind};

/// The MIR of `source`, a file `main` that must typecheck.
fn lower(source: &str) -> MirFile {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main");
    let tree = checker.check_module(&file);
    assert!(
        checker.errors.is_empty(),
        "type errors: {:?}",
        checker.errors
    );
    prism_mir::lower_file(&tree)
}

/// The elaborated MIR of `source`, after checking it is well formed and its drops verify.
fn elaborate(source: &str) -> MirFile {
    let file = match check_file(&lower(source)) {
        Ok(file) => file,
        Err(errors) => panic!("borrow errors: {:?}", errors),
    };
    assert_eq!(file.phase, MirPhase::Elaborated);
    let violations = file.validate();
    assert!(
        violations.is_empty(),
        "{}\n{}",
        violations.join("\n"),
        file.dump()
    );
    let violations = verify_elaborated(&file);
    assert!(
        violations.is_empty(),
        "{}\n{}",
        violations.join("\n"),
        file.dump()
    );
    file
}

/// The dump of function `name` in the dump of `file`: its header line up to the next body.
fn body_dump(file: &MirFile, name: &str) -> String {
    let dump = file.dump();
    let header = format!("fun main::{}(", name);
    let start = dump
        .find(&header)
        .unwrap_or_else(|| panic!("no {} in\n{}", name, dump));
    let rest = &dump[start..];
    let end = rest[1..]
        .find("\nfun ")
        .map(|end| end + 2)
        .unwrap_or(rest.len());
    rest[..end].to_string()
}

const TAKE: &str = "fun take(s: String) {}\n";

/// mir: "A moved value is not dropped".
#[test]
fn a_moved_value_is_not_dropped() {
    let file = elaborate(&format!("{}fun f(s: String) {{ take(s) }}\n", TAKE));
    let f = body_dump(&file, "f");
    assert!(f.contains("call main::take(move _2)"), "{}", f);
    assert!(f.contains("_2 = move _1"), "{}", f);
    assert!(!f.contains("drop"), "{}", f);
    assert!(!f.contains("drop flag"), "{}", f);
    assert!(file.dump().starts_with("module main\nphase elaborated\n"));
}

/// mir: "A conditionally moved value is dropped under a flag".
#[test]
fn a_conditionally_moved_value_is_dropped_under_a_flag() {
    let file = elaborate(&format!(
        "{}fun f(c: Bool, s: String) {{ if (c) take(s) }}\n",
        TAKE
    ));
    let f = body_dump(&file, "f");
    assert!(f.contains("local _5: Bool (drop flag)"), "{}", f);
    assert!(f.contains("bb0:\n    _5 = const true"), "{}", f);
    assert!(
        f.contains("_5 = const false @2:41\n    _3 = move _2"),
        "{}",
        f
    );
    assert!(
        f.contains("switch copy _5 [0 -> bb5, otherwise -> bb6]"),
        "{}",
        f
    );
    assert!(f.contains("bb6:\n    drop _2 -> bb5"), "{}", f);
    assert_eq!(f.matches("drop _2").count(), 1, "{}", f);
}

/// mir: "Assigning over a possibly moved variable".
#[test]
fn assigning_over_a_possibly_moved_variable() {
    let file = elaborate(&format!(
        "{}fun f(c: Bool) {{ var s = \"a\"; if (c) take(s); s = \"b\" }}\n",
        TAKE
    ));
    let f = body_dump(&file, "f");
    // The drop of the old value runs under the flag, the drop at the end of the block without.
    assert!(
        f.contains("switch copy _5 [0 -> bb5, otherwise -> bb7] @2:47"),
        "{}",
        f
    );
    assert!(f.contains("bb7:\n    drop _2 -> bb5 @2:47"), "{}", f);
    assert!(
        f.contains("_2 = const \"b\" @2:47\n    _5 = const true @2:47"),
        "{}",
        f
    );
    assert!(f.contains("    drop _2 -> bb6 @2:16\n  bb6:"), "{}", f);
    assert_eq!(f.matches("switch copy _5").count(), 1, "{}", f);
}

/// mir: "Reinitialization after a move drops nothing".
#[test]
fn reinitialization_after_a_move_drops_nothing() {
    let file = elaborate(&format!(
        "{}fun f() {{ var s = \"a\"; take(s); s = \"b\" }}\n",
        TAKE
    ));
    let f = body_dump(&file, "f");
    assert!(!f.contains("drop flag"), "{}", f);
    assert_eq!(f.matches("drop _1").count(), 1, "{}", f);
    // The only drop is at the end of the block, after `"b"` is written.
    let write = f.find("_1 = const \"b\"").unwrap();
    assert!(f.find("drop _1").unwrap() > write, "{}", f);
}

/// mir: "Deterministic scope exit cleanup".
#[test]
fn deterministic_scope_exit_cleanup() {
    let file =
        elaborate("fun g() {}\nfun f(c: Bool) { val a = \"a\"; if (c) { val b = \"b\" }; g() }\n");
    let f = body_dump(&file, "f");
    assert!(!f.contains("drop flag"), "{}", f);
    let drop_b = f.find("drop _3").expect("b is dropped");
    let call_g = f.find("call main::g()").unwrap();
    let drop_a = f.find("drop _2").expect("a is dropped");
    assert!(drop_b < call_g && call_g < drop_a, "{}", f);
}

/// mir: "Drop of a declaration that a continue skipped".
#[test]
fn drop_of_a_declaration_that_a_continue_skipped() {
    let file = elaborate(
        "fun f(c: Bool, d: Bool) { do { val s = \"a\"; if (c) continue; val t = \"b\" } while (d) }\n",
    );
    let f = body_dump(&file, "f");
    assert!(f.contains("local _5: Bool (drop flag)"), "{}", f);
    // `s` is dropped without a flag on both edges of the condition, `t` under its flag on both.
    assert_eq!(f.matches("drop _3").count(), 2, "{}", f);
    assert_eq!(f.matches("switch copy _5").count(), 2, "{}", f);
    assert_eq!(f.matches("drop _4").count(), 2, "{}", f);
    // The `continue` path marks `t` live and clears its flag without setting it.
    assert!(
        f.contains("storage_live _4 @1:52\n    _5 = const false @1:52\n    goto"),
        "{}",
        f
    );
}

#[test]
fn a_call_destination_sets_its_flag_where_the_call_returns() {
    let file = elaborate(&format!(
        "{}fun make(): String = \"a\"\nfun f(c: Bool) {{ var s = make(); if (c) take(s) }}\n",
        TAKE
    ));
    let f = body_dump(&file, "f");
    assert!(f.contains("_2 = call main::make() -> bb1"), "{}", f);
    assert!(f.contains("bb1:\n    _5 = const true"), "{}", f);
}

#[test]
fn verify_reports_a_drop_of_a_moved_local() {
    // The MIR as lowered still drops the moved `s`: marked elaborated, it fails verification.
    let mut file = lower(&format!("{}fun f(s: String) {{ take(s) }}\n", TAKE));
    file.phase = MirPhase::Elaborated;
    let violations = verify_elaborated(&file).join("\n");
    assert!(
        violations.contains("main::f: bb1 drops _1, which may hold no value there"),
        "{}",
        violations
    );
}

#[test]
fn verify_reports_a_flag_that_is_not_cleared_by_a_move() {
    let mut file = elaborate(&format!(
        "{}fun f(c: Bool, s: String) {{ if (c) take(s) }}\n",
        TAKE
    ));
    let body = file
        .bodies
        .ids()
        .find(|id| file.body_name(*id) == "main::f")
        .unwrap();
    let body = &mut file.bodies[body];
    for (_, block) in body.blocks.iter_mut() {
        block.statements.retain(|statement| {
            !matches!(
                &statement.kind,
                StatementKind::Assign(place, rvalue)
                    if place.local.0 == 5
                        && **rvalue
                            == prism_mir::Rvalue::Use(prism_mir::Operand::Constant(
                                prism_mir::Constant::Bool(false)
                            ))
            )
        });
    }
    let violations = verify_elaborated(&file).join("\n");
    assert!(
        violations.contains("the drop flag 5 does not mirror whether _2 holds a value"),
        "{}",
        violations
    );
}
