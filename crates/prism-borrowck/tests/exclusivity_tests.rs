//! The exclusivity and escape checks on small files lowered from source: codes, messages,
//! labels and helps (`borrowck-exclusivity`).

use prism_borrowck::{check_file, BorrowError};

/// The MIR of `source`, a file `main` that must typecheck.
fn lower(source: &str) -> prism_mir::MirFile {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main")
        .with_prelude(prism_typechecker::test_prelude());
    let tree = checker.check_module(&file);
    assert!(
        checker.errors.is_empty(),
        "type errors: {:?}",
        checker.errors
    );
    prism_mir::lower_file(&tree)
}

/// The borrow errors of `source`, which must have some.
fn errors(source: &str) -> Vec<BorrowError> {
    match check_file(&lower(source)) {
        Ok(_) => panic!("expected borrow errors in:\n{}", source),
        Err(errors) => errors,
    }
}

/// An error as text: `code message @line:column label`, then each related label, note and help.
fn render(error: &BorrowError) -> String {
    let mut out = format!(
        "{} {} @{}:{} {}",
        error.code(),
        error.message(),
        error.primary.span.line,
        error.primary.span.column,
        error.primary.message
    );
    for label in &error.related {
        out.push_str(&format!(
            "\n  related @{}:{} {}",
            label.span.line, label.span.column, label.message
        ));
    }
    for note in &error.notes {
        out.push_str(&format!("\n  note: {}", note));
    }
    for help in &error.helps {
        out.push_str(&format!("\n  help: {}", help));
    }
    out
}

fn rendered(source: &str) -> String {
    errors(source)
        .iter()
        .map(render)
        .collect::<Vec<_>>()
        .join("\n")
}

fn passes(source: &str) {
    if let Err(errors) = check_file(&lower(source)) {
        panic!(
            "unexpected borrow errors:\n{}",
            errors.iter().map(render).collect::<Vec<_>>().join("\n")
        );
    }
}

const DECLARATIONS: &str = "\
struct User { var name: String }
struct Order { var user: User, var email: String }
struct Stack { var count: Int }
struct Button { val onClick: () -> Int }
mut fun Stack.push(n: Int) {}
fun Stack.size(): Int = count
fun print(n: &Int) {}
fun bump(r: &mut Int) {}
fun take(s: String) {}
fun greet(s: &String): Int = 1
fun make(): String = \"x\"
fun touch(a: &mut User, b: &mut String) {}
fun run(f: () -> Int): Int = f()
";

/// The rendered errors of `body` after the declarations, which start on line 14.
fn check(body: &str) -> String {
    rendered(&format!("{}{}", DECLARATIONS, body))
}

fn accept(body: &str) {
    passes(&format!("{}{}", DECLARATIONS, body))
}

#[test]
fn a_second_mutable_borrow_conflicts() {
    assert_eq!(
        check("fun f() { var x = 1; val m = &mut x; val k = &mut x; print(m) }\n"),
        "\
E5003 cannot borrow 'x' as mutable more than once at a time @14:46 second mutable borrow occurs here
  related @14:30 first mutable borrow occurs here
  related @14:60 first borrow later used here"
    );
}

#[test]
fn a_mutable_capture_conflicts_with_a_read() {
    assert_eq!(
        check("fun f() { var u = User(\"a\"); val g = { u.name = \"z\"; 1 }; greet(u.name); run(g) }\n"),
        "\
E5003 cannot borrow 'u.name' as shared because it is also borrowed as mutable @14:65 shared borrow occurs here
  related @14:38 closure borrows 'u.name' mutably here
  related @14:40 borrow occurs due to this use of 'u.name' in the closure
  related @14:78 mutable borrow later used here"
    );
}

#[test]
fn assigning_a_borrowed_variable() {
    assert_eq!(
        check("fun f() { var x = 1; val r = &x; x = 2; print(r) }\n"),
        "\
E5004 cannot assign to 'x' because it is borrowed @14:34 'x' is assigned here while borrowed
  related @14:30 'x' is borrowed here
  related @14:47 borrow later used here"
    );
}

#[test]
fn a_use_in_a_later_iteration() {
    assert_eq!(
        check("fun f(c: Bool) { var x = 1; val r = &x; while (c) { print(r); x = 2 } }\n"),
        "\
E5004 cannot assign to 'x' because it is borrowed @14:63 'x' is assigned here while borrowed
  related @14:37 'x' is borrowed here
  related @14:59 borrow later used here, in later iteration of loop"
    );
}

#[test]
fn moving_a_borrowed_value() {
    assert_eq!(
        check("fun f(s: String) { val r = &s; take(s); greet(r) }\n"),
        "\
E5005 cannot move out of 's' because it is borrowed @14:37 's' is moved here while borrowed
  related @14:28 's' is borrowed here
  related @14:47 borrow later used here
  help: clone it where it moves: 's.clone()'"
    );
}

#[test]
fn a_local_that_ends_while_borrowed() {
    assert_eq!(
        check("fun f(c: Bool) { var a = 1; var r: &Int = &a; if (c) { val n = 1; r = &n }; print(r) }\n"),
        "\
E5006 'n' does not live long enough @14:71 borrowed value does not live long enough
  related @14:74 'n' dropped here while still borrowed
  related @14:83 borrow later used here"
    );
}

#[test]
fn a_temporary_that_ends_while_borrowed() {
    assert_eq!(
        check("fun f(s: String) { var r: &String = &s; r = &make(); greet(r) }\n"),
        "\
E5006 temporary value dropped while borrowed @14:45 this borrows a temporary value
  related @14:41 temporary value is freed at the end of this statement
  related @14:60 borrow later used here
  help: bind the value to a local with 'val' before borrowing it, so that it lives as long as its block"
    );
}

#[test]
fn borrows_that_do_not_conflict() {
    // The borrow ends at its last use.
    accept("fun f() { var x = 1; val r = &x; print(r); x = 2 }\n");
    // The receiver of `size` is borrowed and released before `push` borrows it mutably.
    accept("fun f() { var v = Stack(0); v.push(v.size()); v.push(v.count) }\n");
    // Different fields are borrowed independently.
    accept("fun f(o: &mut Order) { touch(&mut o.user, &mut o.email) }\n");
    accept(
        "fun f(o: Order) { var p = o; val a = &mut p.user; val b = &mut p.email; touch(a, b) }\n",
    );
    // Re-pointing a reference ends its old borrow.
    accept("fun f() { var a = 1; var b = 2; var r: &Int = &a; r = &b; a = 3; print(r) }\n");
    // A closure's borrows end with its last use.
    accept(
        "fun f() { var u = User(\"a\"); val k = { u.name = \"z\"; 1 }; run(k); u.name = \"b\" }\n",
    );
}

/// references: "Escaping closures own their captures", "Escaping Function Values Hold No
/// Borrows" and "Escaping Parameters Keep Function Values" (`escaping-closures`).
#[test]
fn escaping_closures_own_their_captures_and_hold_no_borrows() {
    accept("fun f(s: String): () -> Int = { greet(s) }\n");
    accept("fun g(s: String): Button = Button({ greet(s) })\n");
    accept("fun h(slot: &mut (() -> Int), s: String) { *slot = { greet(s) } }\n");
    accept("fun m(a: Int, p: () -> Int = { a }): Int = p()\n");
    // A closure copying an `Int` read through a reference owns no reference
    // (`path-captures`, D4).
    accept("fun n(r: &Int): () -> Int = { r }\n");
    accept("fun o(r: &Int): Button = Button({ r + 1 })\n");
    assert_eq!(
        check("fun f(r: &String): () -> Int = { greet(r) }\n"),
        "\
E5007 a closure that captures the reference 'r' cannot escape @14:32 returned here
  related @14:40 'r' is captured here
  note: a closure that escapes owns what it captures, and a reference cannot be owned
  help: pass the value 'r' refers to instead of a reference"
    );
    let stored = check("fun g(r: &String): Button = Button({ greet(r) + 1 })\n");
    assert!(
        stored.starts_with(
            "E5007 a closure that captures the reference 'r' cannot escape @14:36 stored in a field here"
        ),
        "{}",
        stored
    );
    let parameter = check("fun k(p: () -> Int): () -> Int = p\n");
    assert_eq!(
        parameter,
        "\
E5007 the parameter 'p' cannot escape @14:34 returned here
  related @14:7 'p' is a parameter here
  note: an unmarked function-typed parameter may borrow its caller's variables
  help: declare it as 'p: escaping () -> Int' if the function may keep it"
    );
}

#[test]
fn closures_without_captures_may_escape() {
    accept("fun f(): () -> Int = { 1 }\n");
    accept("fun g(): Button = Button({ 2 })\n");
    accept("fun h(s: String): Int = run { s.clone(); 3 }\n");
}

/// references: "A later part mutating a borrowed part rejected" (`string-templates`): a string
/// part stays borrowed until the template is built, as an operand of `+` does.
#[test]
fn a_later_part_mutating_a_borrowed_part_rejected() {
    let declarations = "fun grow(s: &mut String): Int { *s = s + \"!\"; return s.length }\n";
    let body = "fun f() { var s = \"a\"; val t = \"$s ${grow(&mut s)}\" }\n";
    assert_eq!(
        rendered(&format!("{}{}", declarations, body)),
        "\
E5003 cannot borrow 's' as mutable because it is also borrowed as shared @2:43 mutable borrow occurs here
  related @2:34 shared borrow occurs here
  related @2:34 shared borrow later used here"
    );
    // A scalar part is read at its position, so a later part may write it.
    passes(
        "fun bump(r: &mut Int): Int { *r = r + 1; return r }\nfun f(): String { var n = 1; return \"$n ${bump(&mut n)} $n\" }\n",
    );
    // Parts are never moved: the string stays usable after the template.
    passes("fun f(s: String): String { val t = \"$s$s\"; return s + t }\n");
}
