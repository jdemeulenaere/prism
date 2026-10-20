//! The exclusivity and escape checks on small files lowered from source: codes, messages,
//! labels and helps (`borrowck-exclusivity`).

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

/// references: "A closure borrowing a field leaves the others free" and "A closure writing
/// through a mutable reference leaves the other fields free" (`path-captures`): a closure
/// borrows the paths it captures, which conflict only with overlapping places.
#[test]
fn closures_borrow_disjoint_paths() {
    accept("fun f() { var o = Order(User(\"a\"), \"b\"); val g = { o.email = \"z\"; 1 }; greet(o.user.name); o.user.name = \"c\"; run(g) }\n");
    accept("fun f() { var o = Order(User(\"a\"), \"b\"); val g = { greet(o.email) }; o.user.name = \"c\"; greet(o.email); run(g) }\n");
    assert_eq!(
        check("fun f() { var o = Order(User(\"a\"), \"b\"); val g = { o.user.name = \"z\"; 1 }; touch(&mut o.user, &mut o.email); run(g) }\n"),
        "\
E5003 cannot borrow 'o.user' as mutable because 'o.user.name' is borrowed as mutable @14:82 second mutable borrow occurs here
  related @14:50 closure borrows 'o.user.name' mutably here
  related @14:52 borrow occurs due to this use of 'o.user.name' in the closure
  related @14:114 first borrow later used here"
    );
    assert!(check(
        "fun f() { var o = Order(User(\"a\"), \"b\"); val g = { greet(o.email) }; o.email = \"c\"; run(g) }\n"
    )
    .starts_with("E5004 cannot assign to 'o.email' because it is borrowed"));
}

/// A closure writing a field through a `val` mutable reference borrows that path only, and a
/// `var` reference is captured whole, so re-pointing it conflicts (D1 of `path-captures`).
#[test]
fn paths_through_references() {
    let declarations = "struct P { var x: Int, var y: Int }\nfun run(f: () -> Int): Int = f()\n";
    passes(&format!(
        "{}fun f(r: &mut P): Int {{ val g = {{ r.x += 1; r.x }}; r.y = 5; return run(g) }}\n",
        declarations
    ));
    let found = rendered(&format!(
        "{}fun f(a: &P, b: &P): Int {{ var r = a; val g = {{ r.x }}; r = b; return run(g) }}\n",
        declarations
    ));
    assert!(found.starts_with("E5004 cannot assign to 'r'"), "{}", found);
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

/// `owned-state` D24: a composition keeps the states its content captures weakly borrowed while
/// it is used, through `setContent` too, and storing it is `E5010`; a keeper returned from a
/// function is tied to the arguments of its attached parameters.
#[test]
fn compositions_keep_their_weak_captures_borrowed() {
    let moved = rendered(
        "fun consume(s: MutableState<Int>) {}\n\
         fun main() { val s = mutableStateOf(0); val c = composition { println(s.value) }; consume(s); println(c.recompose()) }\n",
    );
    assert!(moved.starts_with("E5005 "), "{}", moved);
    assert!(
        moved.contains("the closure captures 's' weakly here"),
        "{}",
        moved
    );
    let replaced = rendered(
        "fun consume(s: MutableState<Int>) {}\n\
         fun main() { val c = composition { }; val s = mutableStateOf(0); c.setContent { println(s.value) }; consume(s); println(c.recompose()) }\n",
    );
    assert!(replaced.starts_with("E5005 "), "{}", replaced);
    let stored = rendered(
        "struct App { val c: Composition }\n\
         fun main() { val s = mutableStateOf(0); val app = App(composition { println(s.value) }) }\n",
    );
    assert!(stored.starts_with("E5010 "), "{}", stored);
    passes(
        "fun wrap(content: attached compose () -> Unit): Composition = composition(content)\n\
         fun main() { val s = mutableStateOf(1); val c = wrap { println(s.value) }; s.value = 2; println(c.recompose()) }\n",
    );
    // Without a later use of the composition, the state is free to move.
    passes(
        "fun consume(s: MutableState<Int>) {}\n\
         fun main() { val s = mutableStateOf(0); val c = composition { println(s.value) }; consume(s) }\n",
    );
}

/// `owned-state` D3: a borrow of a state's value is a borrow of the state object.
#[test]
fn a_borrowed_state_value_keeps_its_state_borrowed() {
    let moved = rendered(
        "fun take(s: MutableState<List<Int>>) {}\n\
         fun f() { val s = mutableStateOf(listOf(1)); val items = &s.value; take(s); println(items.size) }\n",
    );
    assert!(moved.starts_with("E5005 "), "{}", moved);
    passes("compose fun items(s: &State<List<Int>>): &List<Int> = &s.value\n");
}
