//! The use check on small files lowered from source: codes, messages, labels and helps.

use prism_borrowck::{check_file, BorrowError};

/// The MIR of `source`, a file `main` that must typecheck.
fn lower(source: &str) -> prism_mir::MirFile {
    let file = prism_parser::parse_source(source).expect("the source parses");
    let mut checker = prism_typechecker::TypeChecker::new(None, "main").with_library_imports();
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

const TAKE: &str = "fun take(s: String) {}\n";

#[test]
fn a_second_move() {
    let source = format!("{}fun f(s: String) {{ take(s); take(s) }}\n", TAKE);
    assert_eq!(
        rendered(&source),
        "\
E5001 use of moved value 's' @2:34 value used here after move
  related @2:25 value moved here
  note: 's' has type 'String', which is not 'Copy'
  help: clone it where it moves: 's.clone()'"
    );
}

#[test]
fn a_conditional_move_is_a_possible_move() {
    let source = format!(
        "{}fun f(c: Bool, s: String) {{ if (c) take(s); take(s) }}\n",
        TAKE
    );
    assert_eq!(
        rendered(&source),
        "\
E5001 use of possibly moved value 's' @2:50 value used here after move
  related @2:41 value moved here
  note: 's' has type 'String', which is not 'Copy'
  help: clone it where it moves: 's.clone()'"
    );
}

#[test]
fn a_move_in_a_loop_is_labelled_as_a_previous_iteration() {
    let source = format!(
        "{}fun f(c: Bool, s: String) {{ while (c) take(s) }}\n",
        TAKE
    );
    assert_eq!(
        rendered(&source),
        "\
E5001 use of possibly moved value 's' @2:44 value used here after move
  related @2:44 value moved here, in previous iteration of loop
  note: 's' has type 'String', which is not 'Copy'
  help: clone it where it moves: 's.clone()'"
    );
    let source = format!(
        "{}fun f(c: Bool) {{ var s = \"a\"; while (c) take(s) }}\n",
        TAKE
    );
    assert!(
        rendered(&source).contains("help: assign 's' a new value before the next iteration"),
        "{}",
        rendered(&source)
    );
}

/// Both operands are moved by one aggregate statement, each labelled at its own span.
#[test]
fn the_second_operand_of_a_construction_sees_the_first_move() {
    let source = "\
struct Pair { val a: String, val b: String }
fun f(s: String): Pair = Pair(s, s)
";
    assert_eq!(
        rendered(source),
        "\
E5001 use of moved value 's' @2:34 value used here after move
  related @2:31 value moved here
  note: 's' has type 'String', which is not 'Copy'
  help: clone it where it moves: 's.clone()'"
    );
}

#[test]
fn a_borrow_after_a_move() {
    let source = format!("{}fun f(s: String) {{ take(s); val r = &s }}\n", TAKE);
    let out = rendered(&source);
    assert!(
        out.starts_with("E5001 use of moved value 's' @2:37 value borrowed here after move"),
        "{}",
        out
    );
}

#[test]
fn a_closure_capturing_a_moved_value() {
    let source = format!(
        "{}fun greet(s: &String): Int = 1\nfun run(f: () -> Int): Int = f()\nfun f(s: String): Int {{ take(s); return run {{ greet(s) }} }}\n",
        TAKE
    );
    let out = rendered(&source);
    assert!(
        out.starts_with("E5001 use of moved value 's' @4:"),
        "{}",
        out
    );
    assert!(out.contains("value captured here after move"), "{}", out);
    assert_eq!(errors(&source).len(), 1, "{}", out);
}

#[test]
fn assigning_a_field_of_a_moved_value() {
    let source = "\
struct User { var name: String }
fun take(u: User) {}
fun f() { var u = User(\"a\"); take(u); u.name = \"b\" }
";
    let out = rendered(source);
    assert!(
        out.starts_with("E5001 use of moved value 'u' @3:"),
        "{}",
        out
    );
    assert!(out.contains("value assigned here after move"), "{}", out);
    assert!(!out.contains("help"), "{}", out);
}

#[test]
fn copy_help_for_a_plain_enum() {
    let source = "\
enum Color { Red, Green }
fun paint(c: Color) {}
fun f(c: Color) { paint(c); paint(c) }
";
    assert_eq!(
        rendered(source),
        "\
E5001 use of moved value 'c' @3:35 value used here after move
  related @3:25 value moved here
  note: 'c' has type 'Color', which is not 'Copy'
  help: add ': Copy' to the declaration of 'Color' to copy its values instead of moving them"
    );
    passes("enum Color : Copy { Red, Green }\nfun paint(c: Color) {}\nfun f(c: Color) { paint(c); paint(c) }\n");
}

#[test]
fn no_help_for_a_struct_that_cannot_be_copy_or_cloned() {
    let source = "\
struct Name { val text: String }
fun h(n: Name) {}
fun f(n: Name) { h(n); h(n) }
";
    let out = rendered(source);
    assert!(out.starts_with("E5001 use of moved value 'n'"), "{}", out);
    assert!(!out.contains("help"), "{}", out);
}

#[test]
fn a_continue_that_skips_a_declaration_the_condition_reads() {
    let source = "\
fun next(): Int = 0
fun f(c: Bool) { do { val s = \"a\"; if (c) continue; val x = next() } while (x > 0) }
";
    assert_eq!(
        rendered(source),
        "\
E5002 use of possibly uninitialized value 'x' @2:77 'x' used here, but it is not initialized on every path
  related @2:43 the declaration of 'x' is skipped here
  help: declare 'x' before the jump that skips it"
    );
}

#[test]
fn moves_on_separate_paths_and_reinitialization_are_accepted() {
    passes(&format!(
        "{}fun f(c: Bool, s: String) {{ if (c) take(s) else take(s) }}\n",
        TAKE
    ));
    passes(&format!(
        "{}fun f(c: Bool, s: String) {{ if (c) {{ take(s); return }}; take(s) }}\n",
        TAKE
    ));
    passes(&format!(
        "{}fun f(a: String, b: String) {{ var s = a; take(s); s = b; take(s) }}\n",
        TAKE
    ));
    passes(&format!(
        "{}fun f(c: Bool, s: String) {{ while (c) {{ take(s); break }} }}\n",
        TAKE
    ));
}

/// The source text of the primary label of the only error of `source`, then of its related
/// labels.
fn label_texts(source: &str) -> Vec<String> {
    let errors = errors(source);
    assert_eq!(errors.len(), 1, "{:?}", errors);
    let error = &errors[0];
    std::iter::once(&error.primary)
        .chain(&error.related)
        .map(|label| source[label.span.start..label.span.end].to_string())
        .collect()
}

/// ownership: "Labels point at the operands". Each use and move is labelled at the operand that
/// uses or moves the value, not at the statement holding it.
#[test]
fn labels_point_at_the_operands() {
    let declarations = "\
struct Pair { val a: String, val b: String }
struct User { var name: String }
struct Point { var x: Int }
struct Stack { var count: Int }
enum Color { Red, Green }
mut fun Stack.push(n: Int) {}
fun take(s: String) {}
fun takeUser(u: User) {}
fun takePoint(p: Point) {}
fun takeStack(s: Stack) {}
fun paint(c: Color) {}
fun run(f: () -> Int): Int = f()
fun next(): Int = 0
";
    let cases: &[(&str, &[&str])] = &[
        ("fun f(s: String): Pair = Pair(s, s)", &["s", "s"]),
        (
            "fun f(s: String): Bool { val t = s; return s == \"a\" }",
            &["s", "s"],
        ),
        (
            "fun f(s: String?): Bool { val t = s; return s == null }",
            &["s", "s"],
        ),
        (
            "fun f() { var u = User(\"a\"); takeUser(u); u.name = \"b\" }",
            &["u.name", "u"],
        ),
        (
            "fun f() { var p = Point(1); takePoint(p); p.x = 5 }",
            &["p.x", "p"],
        ),
        (
            "fun f(t: String) { var s = \"a\"; s = t; s = t }",
            &["t", "t"],
        ),
        (
            "fun f(c: Color): Int { paint(c); return when (c) { Red -> 1; Green -> 2 } }",
            &["c", "c"],
        ),
        (
            "fun f() { var v = Stack(0); takeStack(v); v.push(1) }",
            &["v", "v"],
        ),
        (
            "fun f(g: () -> Int): Int { val a = run(g); return g() }",
            &["g", "g"],
        ),
        (
            "fun f(c: Bool) { do { val s = \"a\"; if (c) continue; val x = next() } while (x > 0) }",
            &["x", "continue"],
        ),
    ];
    for (body, expected) in cases {
        let source = format!("{}{}\n", declarations, body);
        assert_eq!(&label_texts(&source), expected, "{}", body);
    }
}
