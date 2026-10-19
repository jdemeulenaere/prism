//! The compile-time benchmark of the whole pipeline: how long each stage spends on generated
//! programs of growing size, so that a super-linear step shows up as a growth factor well above
//! the size factor.
//!
//! Each shape is generated at three sizes four times apart (256, 1024 and 4096 of the thing it
//! repeats; nesting shapes at depths 30, 60 and 120, since the parser rejects nesting deeper
//! than 256 with `E2002`), and the shapes nesting calls through their trailing lambdas at four
//! depths each twice the one before (16 to 128, each program holding [`NESTS`] such nests). One
//! file is compiled stage by stage, as `prism run` compiles it:
//!
//! - `lex` and `parse`: the lexer's tokens, then the parser's AST;
//! - `abi`: Phase 1, the file's ABI;
//! - `type`: Phase 2's type checker, which builds the typed tree;
//! - `mir`: MIR lowering;
//! - `borrow`: the borrow checker, with drop elaboration and capture inference;
//! - `compose`: the compose transform (`compose-transform`);
//! - `inst`: the collection of the instances of generics the file needs, from its generic
//!   records, with the cross-file recursion check and the support check of every instance;
//! - `lower`: the backend's support check and lowering to Cranelift IR, of the file's unit and
//!   of every instance unit;
//! - `clif`: Cranelift's compilation of every function and glue function.
//!
//! It is ignored by default, since timings of a debug build or a busy machine vary. Run it with
//!
//! ```text
//! cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture
//! ```
//!
//! `PRISM_TIMING_SHAPES=<name>,<name>` runs only the shapes whose names contain one of the
//! names; `PRISM_TIMING_CRANELIFT=<name>=<value>,...` overrides Cranelift settings for the
//! `clif` column (`opt_level=speed`, `regalloc_algorithm=single_pass`). It reports each stage's time and the growth of the slowest stage
//! from one size to the next, skips the larger sizes of a shape once one size takes over 20 s,
//! and fails when any stage of at least 5 ms grows more than twice as fast as the size, or when
//! the `borrow` stage of a shape of [`LINEAR_BORROW`] grows on average faster than x4.6 per x4
//! (`n^1.1`), or when a stage of at least 5 ms of a shape of [`LINEAR_DEPTH`] grows on average
//! faster than x2.5 per doubling of the depth (`n^1.3`), only when `PRISM_ASSERT_TIMING=1` is
//! set.

use cranelift_codegen::settings::{self, Configurable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Module};
use std::fmt::Write;
use std::time::{Duration, Instant};

/// A generated program: its shape, the size it repeats something to, and its source.
struct Generated {
    size: usize,
    source: String,
}

/// A family of programs: its name, whether the backend compiles it, and its generator.
struct Shape {
    name: &'static str,
    compiled: bool,
    sizes: &'static [usize],
    generate: fn(usize) -> String,
}

const SIZES: &[usize] = &[256, 1024, 4096];

/// The shapes whose borrow check is held to near-linear growth: many loans live at once, or
/// made one after the other over a body, each held by a local (the loan flow's states, and the
/// conflict check's search among them). Their `borrow` time may grow at most `BORROW_GROWTH`
/// per x4 on average from the smallest size to the largest, which leaves room for the slower
/// memory accesses of larger programs, while a quadratic step grows x16.
const LINEAR_BORROW: [&str; 7] = [
    "handlers-kept-to-end",
    "live-handlers",
    "many-closures",
    "closures-kept-past-blocks",
    "live-borrows",
    "template-computed-parts",
    "when-patterns",
];
const BORROW_GROWTH: f64 = 4.6;
/// The shapes nesting calls through their trailing lambdas, every stage of which is held to
/// near-linear growth in the depth: each stage taking at least 5 ms at the deepest nest may
/// grow at most `DEPTH_GROWTH` per doubling of the depth on average, from the shallowest to
/// the deepest. A step quadratic in the depth grows x4; the closures' symbols, which name every
/// enclosing closure (`main::f#closure0#closure0`), leave the backend's growth slightly above
/// x2.
const LINEAR_DEPTH: [&str; 6] = [
    "nested-trailing-lambdas",
    "nested-receiver-lambdas",
    "nested-lambdas-with-parameters",
    "nested-generic-lambdas",
    "nested-generic-implicit-lambdas",
    "nested-compose-ui",
];
const DEPTH_GROWTH: f64 = 2.5;
const DEPTHS: &[usize] = &[30, 60, 120];
/// The depths of the shapes nesting calls through their trailing lambdas, as a compose UI does
/// (`Column { Row { Button(onClick = { ... }) { Text("...") } } }`): each doubles the one
/// before, up to the parser's limit of 256 nesting levels, which a call through its lambda
/// takes two of (its closure and the closure's body).
const CALL_DEPTHS: &[usize] = &[16, 32, 64, 128];
/// How many nests a program of a depth shape holds, one per function.
const NESTS: usize = 32;

fn join(count: usize, separator: &str, item: impl Fn(usize) -> String) -> String {
    (0..count).map(item).collect::<Vec<_>>().join(separator)
}

/// A call with `n` `Int` arguments.
fn arguments_plain(n: usize) -> String {
    format!(
        "fun f({}): Int = a0\nfun main() {{ println(f({})) }}\n",
        join(n, ", ", |i| format!("a{i}: Int")),
        join(n, ", ", |i| i.to_string()),
    )
}

/// A call with `n` arguments `x.clone()`.
fn arguments_clone(n: usize) -> String {
    format!(
        "fun f({}): Int = a0.length\nfun main() {{ val x = 1.toString(); println(f({})) }}\n",
        join(n, ", ", |i| format!("a{i}: String")),
        join(n, ", ", |_| "x.clone()".to_string()),
    )
}

/// A call lending one string to `n` borrowed parameters.
fn arguments_borrow(n: usize) -> String {
    format!(
        "fun f({}): Int = a0.length\nfun main() {{ val x = 1.toString(); println(f({})) }}\n",
        join(n, ", ", |i| format!("a{i}: &String")),
        join(n, ", ", |_| "x".to_string()),
    )
}

/// `n` `val`s, each computed from the one before.
fn many_values(n: usize) -> String {
    let mut body = String::from("    val v0 = 1\n");
    for i in 1..n {
        writeln!(body, "    val v{i} = v{} + {i}", i - 1).unwrap();
    }
    format!("fun main() {{\n{body}    println(v{})\n}}\n", n - 1)
}

/// `n` statements, alternating assignments and calls.
fn many_statements(n: usize) -> String {
    let mut body = String::from("    var x = 0\n");
    for i in 0..n {
        if i % 2 == 0 {
            writeln!(body, "    x += {i}").unwrap();
        } else {
            body.push_str("    println(x)\n");
        }
    }
    format!("fun main() {{\n{body}}}\n")
}

/// `n` sequential `if`s.
fn sequential_ifs(n: usize) -> String {
    let mut body = String::from("    var r = 0\n");
    for i in 0..n {
        writeln!(body, "    if (x == {i}) r += {i} else r -= 1").unwrap();
    }
    format!("fun f(x: Int): Int {{\n{body}    return r\n}}\nfun main() {{ println(f(3)) }}\n")
}

/// A `when` with `n` branches over an `Int`.
fn when_branches(n: usize) -> String {
    format!(
        "fun f(x: Int): Int = when (x) {{\n{}    else -> -1\n}}\nfun main() {{ println(f(3)) }}\n",
        join(n, "", |i| format!("    {i} -> {}\n", i * 2)),
    )
}

/// `if`s nested `n` deep, each in its own block.
fn nested_blocks(n: usize) -> String {
    let mut body = String::new();
    for i in 0..n {
        writeln!(body, "{}if (x > {i}) {{ val y{i} = {i}", " ".repeat(i)).unwrap();
    }
    body.push_str("println(x)");
    body.push_str(&" }".repeat(n));
    format!("fun f(x: Int) {{\n{body}\n}}\nfun main() {{ f(3) }}\n")
}

/// Closures nested `n` deep, each returning the next, which captures its local.
fn nested_closures(n: usize) -> String {
    let mut body = String::from("val c0 = 0; val k = ");
    for i in 1..n {
        write!(body, "{{ val c{i} = c{} + 1; ", i - 1).unwrap();
    }
    write!(body, "c{}", n - 1).unwrap();
    body.push_str(&" }".repeat(n - 1));
    format!("fun main() {{ {body} }}\n")
}

/// `n` closures in one body, each capturing a local and the closure before it is called.
fn many_closures(n: usize) -> String {
    let mut body = String::from("var total = 0\n");
    for i in 0..n {
        writeln!(body, "val v{i} = {i}; run {{ total += v{i} }}").unwrap();
    }
    format!("fun run(f: () -> Unit) {{ f() }}\nfun main() {{\n{body}println(total)\n}}\n")
}

/// `n` lambdas with a receiver in sequence, each reading a field and calling an extension
/// through its `this` and calling a context extension (D13 of `receiver-lambdas`).
fn receiver_lambdas(n: usize) -> String {
    let mut body = String::from("var total = 0\n");
    for i in 0..n {
        writeln!(body, "total += nest {{ depth + twice() + {i}.weight() }}").unwrap();
    }
    format!(
        "struct Scope : Copy {{ val depth: Int }}\nfun Scope.twice(): Int = depth * 2\ncontext(scope: Scope)\nfun Int.weight(): Int = this + scope.depth\nfun nest(f: Scope.() -> Int): Int = f(Scope(1))\nfun main() {{\n{body}println(total)\n}}\n"
    )
}

/// `n` scope types, a context extension of one name and receiver per scope, and `n` calls,
/// each in a lambda with its own scope's receiver (D13 of `receiver-lambdas`).
fn context_overloads(n: usize) -> String {
    let mut source = String::new();
    for i in 0..n {
        writeln!(
            source,
            "struct S{i} : Copy {{}}\nfun s{i}(f: S{i}.() -> Int): Int = f(S{i}())\ncontext(s: S{i})\nfun Int.tag(): Int = this + {i}"
        )
        .unwrap();
    }
    let mut body = String::from("var total = 0\n");
    for i in 0..n {
        writeln!(body, "total += s{i} {{ {i}.tag() }}").unwrap();
    }
    write!(source, "fun main() {{\n{body}println(total)\n}}\n").unwrap();
    source
}

/// `n` header-less closures in one body, each using its implicit `it` (D8 of `implicit-it`).
fn implicit_parameter_closures(n: usize) -> String {
    let mut body = String::from("var total = 0\n");
    for i in 0..n {
        writeln!(body, "each {{ total += it + {i} }}").unwrap();
    }
    format!("fun each(f: (Int) -> Unit) {{ f(1) }}\nfun main() {{\n{body}println(total)\n}}\n")
}

/// Header-less closures nested `n` deep, each with its own `it`, reading the enclosing one
/// through a `val` it captures (D8 of `implicit-it`).
fn nested_implicit_parameter_closures(n: usize) -> String {
    let mut body = String::from("apply { val v0 = it; ");
    for i in 1..n {
        write!(body, "apply {{ val v{i} = it + v{}; ", i - 1).unwrap();
    }
    write!(body, "v{}", n - 1).unwrap();
    body.push_str(&" }".repeat(n));
    format!("fun apply(f: (Int) -> Int): Int = f(1)\nfun main() {{ println({body}) }}\n")
}

/// `NESTS` functions, each holding the nest `nest(n, k)` for its index `k`: enough copies that
/// the stages of the depth shapes take measurable time.
fn nests(
    declarations: &str,
    main: &str,
    n: usize,
    nest: impl Fn(usize, usize) -> String,
) -> String {
    let mut source = String::from(declarations);
    for k in 0..NESTS {
        source.push_str(&nest(n, k));
    }
    source.push_str(main);
    source
}

/// Calls nested `n` deep through their trailing lambdas, which take no parameter, as a compose
/// UI nests its layouts.
fn nested_trailing_lambdas(n: usize) -> String {
    nests(
        "fun column(content: () -> Unit) { content() }\n",
        "fun main() { f0() }\n",
        n,
        |n, k| {
            let mut body = String::new();
            for i in 0..n {
                write!(body, "column {{ val v{i} = {i}; ").unwrap();
            }
            body.push_str("println(1)");
            body.push_str(&" }".repeat(n));
            format!("fun f{k}() {{ {body} }}\n")
        },
    )
}

/// Lambdas with a receiver nested `n` deep, alternating two scope types, each level calling a
/// context extension of its scope and the innermost reading a captured local of the outermost
/// (D13 of `receiver-lambdas`).
fn nested_receiver_lambdas(n: usize) -> String {
    nests(
        "struct A : Copy { val depth: Int }\nstruct B : Copy { val depth: Int }\nfun a(f: A.() -> Unit) { f(A(1)) }\nfun b(f: B.() -> Unit) { f(B(2)) }\ncontext(s: A)\nfun Int.ma(): Int = this + s.depth\ncontext(s: B)\nfun Int.mb(): Int = this + s.depth\n",
        "fun main() { f0() }\n",
        n,
        |n, k| {
            let mut body = format!("val outer = {k}; ");
            for i in 0..n {
                let (scope, extension) = if i % 2 == 0 { ("a", "ma") } else { ("b", "mb") };
                write!(body, "{scope} {{ val v{i} = {i}.{extension}(); ").unwrap();
            }
            body.push_str("println(outer)");
            body.push_str(&" }".repeat(n));
            format!("fun f{k}() {{ {body} }}\n")
        },
    )
}

/// Calls nested `n` deep through their trailing lambdas, each declaring its parameter.
fn nested_lambdas_with_parameters(n: usize) -> String {
    nests(
        "fun apply(x: Int, f: (Int) -> Int): Int = f(x)\n",
        "fun main() { println(f0()) }\n",
        n,
        |n, k| {
            let mut body = String::from("apply(0) { a0 -> ");
            for i in 1..n {
                write!(body, "apply(a{} + 1) {{ a{i} -> ", i - 1).unwrap();
            }
            write!(body, "a{}", n - 1).unwrap();
            body.push_str(&" }".repeat(n));
            format!("fun f{k}(): Int = {body}\n")
        },
    )
}

/// Generic calls nested `n` deep through their trailing lambdas, each declaring its parameter,
/// whose type the call infers from its first argument.
fn nested_generic_lambdas(n: usize) -> String {
    nests(
        "fun <T> applyTo(x: T, f: (T) -> T): T = f(x)\n",
        "fun main() { println(f0()) }\n",
        n,
        |n, k| {
            let mut body = String::from("applyTo(0) { a0 -> ");
            for i in 1..n {
                write!(body, "applyTo(a{} + 1) {{ a{i} -> ", i - 1).unwrap();
            }
            write!(body, "a{}", n - 1).unwrap();
            body.push_str(&" }".repeat(n));
            format!("fun f{k}(): Int = {body}\n")
        },
    )
}

/// Generic calls nested `n` deep through their trailing lambdas, each using its implicit `it`
/// and returning a type the call infers from the lambda's body, so each lambda is checked
/// after the call's other arguments (D17 of `generics`).
fn nested_generic_implicit_lambdas(n: usize) -> String {
    nests(
        "fun <T, R> map(x: T, f: (T) -> R): R = f(x)\n",
        "fun main() { println(f0()) }\n",
        n,
        |n, k| {
            let mut body = String::from("map(0) { ");
            for _ in 1..n {
                body.push_str("map(it + 1) { ");
            }
            body.push_str("it");
            body.push_str(&" }".repeat(n));
            format!("fun f{k}(): Int = {body}\n")
        },
    )
}

/// A compose UI nested `n` deep: layouts and buttons with a click handler, a text in each.
/// The backend does not compile composables yet, so only the front end is timed.
fn nested_compose_ui(n: usize) -> String {
    nests(
        "compose fun Column(content: compose () -> Unit) { content() }\n\
         compose fun Button(onClick: () -> Unit, content: compose () -> Unit) { content() }\n\
         compose fun Text(text: String) {}\n",
        "",
        n,
        |n, k| {
            let mut body = String::new();
            for i in 0..n {
                if i % 2 == 0 {
                    write!(body, "Column {{ Text(\"{i}\"); ").unwrap();
                } else {
                    write!(body, "Button({{ println({i}) }}) {{ ").unwrap();
                }
            }
            body.push_str("Text(\"end\")");
            body.push_str(&" }".repeat(n));
            format!("compose fun Screen{k}() {{ {body} }}\n")
        },
    )
}

/// One composable calling `n` skippable composables (D24 of `compose-transform`).
fn composable_calls(n: usize) -> String {
    format!(
        "compose fun Badge(n: Int) {{ }}\n\
         compose fun Screen(k: Int) {{\n{}\n}}\n\
         fun main() {{ val c = composition {{ Screen(1) }} }}\n",
        join(n, "\n", |i| format!(
            "Badge({})",
            if i % 2 == 0 { "k" } else { "1" }
        )),
    )
}

/// `n` sequential `if`s, each calling a composable in its branch.
fn composable_branches(n: usize) -> String {
    format!(
        "compose fun Badge(n: Int) {{ }}\n\
         compose fun Screen(k: Int) {{\n{}\n}}\n\
         fun main() {{ val c = composition {{ Screen(1) }} }}\n",
        join(n, "\n", |i| format!(
            "if (k > {i}) {{ Badge({i}) }} else {{ Badge(k) }}"
        )),
    )
}

/// `if`s nested `n` deep, each branch calling a composable.
fn nested_composable_branches(n: usize) -> String {
    let mut body = String::new();
    for i in 0..n {
        write!(body, "if (k > {i}) {{ Badge({i}); ").unwrap();
    }
    body.push_str(&" }".repeat(n));
    format!(
        "compose fun Badge(n: Int) {{ }}\n\
         compose fun Screen(k: Int) {{ {body} }}\n\
         fun main() {{ val c = composition {{ Screen(1) }} }}\n",
    )
}

/// `n` skippable composables of four stable parameters, each calling the previous one.
fn many_composables(n: usize) -> String {
    let mut source = String::from("struct Point : Copy { val x: Int }\n");
    for i in 0..n {
        let call = if i == 0 {
            String::new()
        } else {
            format!("Item{}(a, b, c, p)", i - 1)
        };
        writeln!(
            source,
            "compose fun Item{i}(a: Int, b: Bool, c: Double, p: Point) {{ {call} }}"
        )
        .unwrap();
    }
    writeln!(
        source,
        "fun main() {{ val c = composition {{ Item{}(1, true, 1.0, Point(2)) }} }}",
        n - 1
    )
    .unwrap();
    source
}

/// `n` content lambdas passed to a composable.
fn composable_lambdas(n: usize) -> String {
    format!(
        "compose fun Column(content: compose () -> Unit) {{ content() }}\n\
         compose fun Text(n: Int) {{ }}\n\
         compose fun Screen(k: Int) {{\n{}\n}}\n\
         fun main() {{ val c = composition {{ Screen(1) }} }}\n",
        join(n, "\n", |i| format!("Column {{ Text({i}); Text(k) }}")),
    )
}

/// One closure capturing `n` locals.
fn many_captures(n: usize) -> String {
    format!(
        "fun run(f: () -> Int): Int = f()\n\
         fun main() {{\n{}\nprintln(run {{ var t = 0\n{}\nt }})\n}}\n",
        join(n, "\n", |i| format!("val v{i} = {i}")),
        join(n, "\n", |i| format!("t += v{i}")),
    )
}

/// `n` uses of functions as values, each an adapter closure.
fn many_adapters(n: usize) -> String {
    format!(
        "fun show(n: Int?): Int = n ?: 0\nfun apply(f: (Int) -> Int): Int = f(1)\n\
         fun main() {{\nvar total = 0\n{}\nprintln(total)\n}}\n",
        join(n, "\n", |_| "total += apply(show)".to_string()),
    )
}

/// `n` instances of a generic whose body builds a closure.
fn generic_closures(n: usize) -> String {
    let mut source = String::from(
        "fun run(f: () -> Int): Int = f()\n\
         fun <T : Copy> count(x: T, f: (T) -> Int): Int = run { f(x) + 1 }\n",
    );
    for i in 0..n {
        writeln!(source, "struct S{i} : Copy {{ val v: Int }}").unwrap();
    }
    writeln!(
        source,
        "fun main() {{\n{}\n}}",
        join(n, "\n", |i| format!(
            "println(count(S{i}({i})) {{ s -> s.v }})"
        ))
    )
    .unwrap();
    source
}

/// `n` handlers stored in structs, each capturing a string by move, built in one body and
/// called through a borrow (`escaping-closures`): the capture decision, owned environments
/// and their drop functions.
fn escaping_handlers(n: usize) -> String {
    let mut body = String::from("    var total = 0\n");
    for i in 0..n {
        writeln!(body, "    val s{i} = {i}.toString()").unwrap();
        writeln!(body, "    val b{i} = Button({{ s{i}.length }})").unwrap();
        writeln!(body, "    total += fire(b{i})").unwrap();
    }
    format!(
        "struct Button {{ val onClick: () -> Int }}\nfun fire(b: &Button): Int = b.onClick()\nfun main() {{\n{body}    println(total)\n}}\n"
    )
}

/// `n` functions, each returning a closure that owns a string and copies a handle, passed on
/// to an `escaping` parameter (`escaping-closures`).
fn returned_closures(n: usize) -> String {
    let mut source = String::from(
        "struct Handle : Copy { val id: Int }\nfun keep(f: escaping () -> Int): () -> Int = f\n",
    );
    for i in 0..n {
        writeln!(
            source,
            "fun make{i}(s: String, h: Handle): () -> Int = keep {{ s.length + h.id + {i} }}"
        )
        .unwrap();
    }
    writeln!(
        source,
        "fun main() {{\n    var total = 0\n{}\n    println(total)\n}}",
        join(n, "\n", |i| format!(
            "    total += make{i}({i}.toString(), Handle({i}))()"
        ))
    )
    .unwrap();
    source
}

/// `n` closures, each stored in a `var` declared outside the block of what it captures, so
/// each owns that capture and borrows a counter (`escaping-closures`, D1 case 2). Each `var`
/// is declared in a block of its own: `n` function-typed `var`s live to the end of one body,
/// each holding a capture loan, are super-linear in the borrow checker's loan flow whether the
/// closures own anything or not (ROADMAP).
fn closures_kept_past_blocks(n: usize) -> String {
    let mut body = String::from("    val c = 1 > 0\n    var count = 0\n    var total = 0\n");
    for i in 0..n {
        writeln!(
            body,
            "    if (c) {{\n        var h: () -> Int = {{ 0 }}\n        if (c) {{ val s = {i}.toString(); h = {{ count += 1; s.length }} }}\n        total += h()\n    }}"
        )
        .unwrap();
    }
    format!("fun main() {{\n{body}    println(total + count)\n}}\n")
}

/// `n` function-typed `var`s that stay in scope to the end of one body, each re-pointed at a
/// closure that borrows a counter mutably, then called: `n` capture loans of one place, each
/// held to the end by a dead local (the loan flow's **Priority** follow-up of
/// `escaping-closures` in ROADMAP).
fn handlers_kept_to_end(n: usize) -> String {
    let mut body = String::from("    var count = 0\n    var total = 0\n");
    for i in 0..n {
        writeln!(
            body,
            "    var h{i}: () -> Int = {{ 0 }}\n    h{i} = {{ count += 1; 1 }}\n    total += h{i}()"
        )
        .unwrap();
    }
    format!("fun main() {{\n{body}    println(total + count)\n}}\n")
}

/// `n` handlers in `val`s, each borrowing a shared counter and a value of its own, all live
/// until they are called at the end of the body, with a read of the counter after each: `n`
/// live capture loans of one place, as a composable declaring many handlers holds them.
fn live_handlers(n: usize) -> String {
    let mut body = String::from("    var count = 0\n    var total = 0\n");
    for i in 0..n {
        writeln!(
            body,
            "    val v{i} = {i}\n    val h{i} = {{ count + v{i} }}\n    total += count"
        )
        .unwrap();
    }
    for i in 0..n {
        writeln!(body, "    total += h{i}()").unwrap();
    }
    format!("fun main() {{\n{body}    println(total)\n}}\n")
}

/// `n` functions, each calling the one before.
fn many_functions(n: usize) -> String {
    let mut source = String::from("fun f0(x: Int): Int = x\n");
    for i in 1..n {
        writeln!(source, "fun f{i}(x: Int): Int = f{}(x) + 1", i - 1).unwrap();
    }
    writeln!(source, "fun main() {{ println(f{}(0)) }}", n - 1).unwrap();
    source
}

/// `n` public functions, which Phase 1 puts in the ABI.
fn public_functions(n: usize) -> String {
    let mut source = String::from("package bench\npublic fun f0(x: Int): Int = x\n");
    for i in 1..n {
        writeln!(source, "public fun f{i}(x: Int): Int = f{}(x) + 1", i - 1).unwrap();
    }
    writeln!(source, "fun main() {{ println(f{}(0)) }}", n - 1).unwrap();
    source
}

/// A struct of `n` `String` fields, built, cloned, compared and dropped.
fn struct_fields(n: usize) -> String {
    format!(
        "struct S : Clone {{ {} }}\n\
         fun make(): S = S({})\n\
         fun main() {{ val s = make(); val t = s.clone(); println(s == t); println(t.f{}) }}\n",
        join(n, ", ", |i| format!("val f{i}: String")),
        join(n, ", ", |i| format!("{i}.toString()")),
        n - 1,
    )
}

/// An enum of `n` variants, matched by a `when` naming each one.
fn enum_variants(n: usize) -> String {
    format!(
        "enum E : Copy {{ {} }}\n\
         fun f(e: E): Int = when (e) {{\n{}}}\n\
         fun main() {{ println(f(E.V3(3))) }}\n",
        join(n, ", ", |i| format!("V{i}(Int)")),
        join(n, "", |i| format!("    E.V{i}(val x) -> x + {i}\n")),
    )
}

/// An enum of `n` variants holding a `Bool`, matched by a `when` with two patterns per variant,
/// whose exhaustiveness the checker works out.
fn when_patterns(n: usize) -> String {
    format!(
        "enum E : Copy {{ {} }}\n\
         fun f(e: E): Int = when (e) {{\n{}}}\n\
         fun main() {{ println(f(E.V3(true))) }}\n",
        join(n, ", ", |i| format!("V{i}(Bool)")),
        join(n, "", |i| format!(
            "    E.V{i}(true) -> {i}\n    E.V{i}(false) -> -{i}\n"
        )),
    )
}

/// `n` concatenations, in chains of eight.
fn string_concatenation(n: usize) -> String {
    let mut body = String::from("    var s = \"\"\n");
    for i in 0..n / 8 {
        writeln!(
            body,
            "    s = s + {}",
            join(8, " + ", |j| format!("\"{}\"", i * 8 + j))
        )
        .unwrap();
    }
    format!("fun main() {{\n{body}    println(s.length)\n}}\n")
}

/// `n` strings, each borrowed, with every borrow live until the end.
fn live_borrows(n: usize) -> String {
    let mut body = String::new();
    for i in 0..n {
        writeln!(body, "    val s{i} = {i}.toString()").unwrap();
        writeln!(body, "    val r{i} = &s{i}").unwrap();
    }
    for i in 0..n {
        writeln!(body, "    println(r{i})").unwrap();
    }
    format!("fun main() {{\n{body}}}\n")
}

/// `n` strings, half of them moved on one path only, so that their drops need flags.
fn moves_and_drops(n: usize) -> String {
    let mut body = String::new();
    for i in 0..n {
        writeln!(body, "    val s{i} = {i}.toString()").unwrap();
        if i % 2 == 0 {
            writeln!(body, "    if (c) take(s{i})").unwrap();
        }
    }
    format!(
        "fun take(s: String) {{}}\nfun f(c: Bool) {{\n{body}}}\nfun main() {{ f(true); f(false) }}\n"
    )
}

/// One template of `n` parts, alternating `Int`, `String` and `Bool` parts with text between
/// them (`string-templates`).
fn template_parts(n: usize) -> String {
    let parts = join(n, " ", |i| match i % 3 {
        0 => format!("n{i}=$n"),
        1 => format!("s{i}=$s"),
        _ => format!("b{i}=${{n > {i}}}"),
    });
    format!(
        "fun f(n: Int, s: &String): String = \"{parts}\"\nfun main() {{ println(f(1, \"x\").length) }}\n"
    )
}

/// One template of `n` computed parts, each a string temporary (`${s + s}`) or an `Int`
/// computed by a call (`${next(i)}`), so that every string part's borrow is live until the
/// template is built and every scalar part before a call is stabilized.
fn template_computed_parts(n: usize) -> String {
    let parts = join(n, "", |i| {
        if i % 2 == 0 {
            "${s + s}".to_string()
        } else {
            format!("${{next({i})}}")
        }
    });
    format!(
        "fun next(i: Int): Int = i + 1\nfun f(s: &String): String = \"{parts}\"\nfun main() {{ println(f(\"x\").length) }}\n"
    )
}

/// One template of `n` lines' worth of text (64 characters each) around a single part.
fn template_long_text(n: usize) -> String {
    let text = "abcdefghijklmnopqrstuvwxyz \\$ ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456";
    format!(
        "fun f(n: Int): String = \"{}${{n}}{}\"\nfun main() {{ println(f(1).length) }}\n",
        text.repeat(n / 2),
        text.repeat(n / 2)
    )
}

/// `n` statements, each printing a template of a few parts.
fn many_templates(n: usize) -> String {
    let mut body = String::from("    val s = \"x\"\n");
    for i in 0..n {
        writeln!(body, "    println(\"v{i} = ${{n + {i}}} of $s\")").unwrap();
    }
    format!("fun f(n: Int) {{\n{body}}}\nfun main() {{ f(1) }}\n")
}

/// Templates nested `n` deep, each the only part of the next.
fn nested_templates(n: usize) -> String {
    let mut template = "\"$n\"".to_string();
    for _ in 1..n {
        template = format!("\"(${{{template}}})\"");
    }
    format!("fun f(n: Int): String = {template}\nfun main() {{ println(f(1)) }}\n")
}

/// `n` calls of one generic function, at one type: one instance.
fn generic_call_sites(n: usize) -> String {
    let mut body = String::from("    var x = 0\n");
    for i in 0..n {
        writeln!(body, "    x += keep({i})").unwrap();
    }
    format!("fun <T> keep(x: T): T = x\nfun main() {{\n{body}    println(x)\n}}\n")
}

/// One generic function called at `n` struct types: `n` instances.
fn many_instances(n: usize) -> String {
    let mut source = String::from("fun <T> keep(x: T): T = x\n");
    for i in 0..n {
        writeln!(source, "struct S{i} : Copy {{ val v: Int }}").unwrap();
    }
    source.push_str("fun main() {\n    var x = 0\n");
    for i in 0..n {
        writeln!(source, "    x += keep(S{i}({i})).v").unwrap();
    }
    source.push_str("    println(x)\n}\n");
    source
}

/// A chain of `n` generic functions, each calling the next at its own type parameter, called
/// once: `n` instances reached transitively.
fn generic_call_chain(n: usize) -> String {
    let mut source = String::new();
    for i in 0..n - 1 {
        writeln!(
            source,
            "fun <T> g{i}(x: T, n: Int): Int = g{}(x, n + 1)",
            i + 1
        )
        .unwrap();
    }
    writeln!(source, "fun <T> g{}(x: T, n: Int): Int = n", n - 1).unwrap();
    source.push_str("fun main() { println(g0(\"a\", 0)) }\n");
    source
}

/// Generic calls nested `n` deep: `keep(keep(...keep(1)))`.
fn nested_generic_calls(n: usize) -> String {
    let call = (0..n).fold("1".to_string(), |inner, _| format!("keep({inner})"));
    format!("fun <T> keep(x: T): T = x\nfun main() {{ println({call}) }}\n")
}

/// `n` generic structs, each holding the one before at its own type parameter, instantiated
/// at the last: the type graph of the polymorphic recursion check, and `n` nested layouts.
fn nested_generic_types(n: usize) -> String {
    let mut source = String::from("struct S0<T>(val v: T)\n");
    for i in 1..n {
        writeln!(
            source,
            "struct S{i}<T>(val v: S{}<T>, val w: Heap<S{}<T>>?)",
            i - 1,
            i - 1
        )
        .unwrap();
    }
    writeln!(
        source,
        "fun <T> keep(x: S{}<T>): Int = 1\nfun main() {{ println(1) }}",
        n - 1
    )
    .unwrap();
    source
}

/// One `listOf` call with `n` arguments: one generic call whose element type joins `n` bounds
/// (D4 of `collections`).
fn list_literal(n: usize) -> String {
    format!(
        "fun main() {{\n    val xs = listOf({})\n    println(xs.size)\n}}\n",
        join(n, ", ", |i| i.to_string())
    )
}

/// `n` statements of list member calls and element places on one list (D6 and D7 of
/// `collections`).
fn list_operations(n: usize) -> String {
    let mut body = String::from("    var xs = listOf(0)\n    var total = 0\n");
    for i in 0..n {
        match i % 4 {
            0 => writeln!(body, "    xs.add({i})").unwrap(),
            1 => writeln!(body, "    xs[0] += {i}").unwrap(),
            2 => writeln!(body, "    total += xs.size + xs[xs.size - 1]").unwrap(),
            _ => writeln!(body, "    if (xs.contains({i})) total += xs.indexOf({i})").unwrap(),
        }
    }
    format!("fun main() {{\n{body}    println(total)\n}}\n")
}

/// `n` pairs of range loops, as [`list_loops`] has list loops: its baseline.
fn range_loops(n: usize) -> String {
    let mut body = String::from("    var total = 0\n");
    for i in 0..n {
        writeln!(
            body,
            "    for (x in 0..<3) total += x + {i}\n    for (y in 0..<3) total += y"
        )
        .unwrap();
    }
    format!("fun main() {{\n{body}    println(total)\n}}\n")
}

/// `n` loops over one list, each holding its borrow (D9 and D16 of `collections`).
fn list_loops(n: usize) -> String {
    let mut body = String::from("    var xs = listOf(1, 2, 3)\n    var total = 0\n");
    for i in 0..n {
        writeln!(
            body,
            "    for (x in xs) total += x + {i}\n    for (y in &mut xs) *y += 1"
        )
        .unwrap();
    }
    format!("fun main() {{\n{body}    println(total)\n}}\n")
}

/// Lists of `n` struct element types, each dropped, cloned and compared: `n` sets of list glue
/// (D15 of `collections`).
fn list_element_types(n: usize) -> String {
    let mut source = String::new();
    for i in 0..n {
        writeln!(source, "struct S{i} : Clone {{ val s: String }}").unwrap();
    }
    source.push_str("fun main() {\n    var same = true\n");
    for i in 0..n {
        writeln!(
            source,
            "    val l{i} = listOf(S{i}(\"a\".clone()))\n    same = same && l{i}.clone() == l{i}"
        )
        .unwrap();
    }
    source.push_str("    println(same)\n}\n");
    source
}

/// A list of lists nested `n` deep: glue `n` types deep.
fn nested_lists(n: usize) -> String {
    let value = (0..n).fold("1".to_string(), |inner, _| format!("listOf({inner})"));
    format!("fun main() {{\n    val xs = {value}\n    println(xs == xs.clone())\n}}\n")
}

/// `n` functions whose parameters are structs and enums nested eight deep, each reaching every
/// definition below it twice: their slot contracts hash the fingerprints of those types, which
/// are memoized per unit (D5 and D6 of `hot-reload`), so the `lower` stage stays linear in `n`.
fn contract_fingerprints(n: usize) -> String {
    let mut source =
        String::from("enum E0 { A, B(Int) }\nstruct S0 : Copy { val x: Int, val y: Double }\n");
    for depth in 1..8 {
        let below = depth - 1;
        writeln!(
            source,
            "struct S{depth} {{ val a: S{below}, val b: S{below}, val e: E{below} }}\n\
             enum E{depth} {{ A(S{below}), B(E{below}, Int), C }}"
        )
        .unwrap();
    }
    for i in 0..n {
        writeln!(source, "fun f{i}(a: &S7, b: &E7, c: Int): Int = c + {i}").unwrap();
    }
    source
}

/// `n` statements calling the prelude's intrinsics and helpers (`standard-library`): the
/// front end looks each name and member up in the shared prelude scope. Not compiled here: the
/// helpers' instances need the library's records, which `prelude_costs_a_lookup_per_file`
/// leaves out too.
fn prelude_calls(n: usize) -> String {
    format!(
        "fun main() {{\n    val xs = listOf(1, 2, 3)\n{}}}\n",
        join(n, "", |i| format!(
            "    println(xs.map {{ x -> x + {} }}.sumOf {{ x -> x }} + xs.size)\n",
            i
        )),
    )
}

/// A small workspace file of the shape of `prelude_costs_a_lookup_per_file`: calls of
/// `println`, `listOf` and two helpers.
fn prelude_file(i: usize) -> String {
    format!(
        "fun f{i}(): Int {{\n    val xs = listOf({i}, 2)\n    println(xs.size)\n    return xs.map {{ x -> x * 2 }}.count {{ x -> x > {i} }}\n}}\n"
    )
}

fn shapes() -> Vec<Shape> {
    let shape = |name, compiled, sizes, generate| Shape {
        name,
        compiled,
        sizes,
        generate,
    };
    vec![
        shape("arguments-plain", true, SIZES, arguments_plain),
        shape("arguments-clone", true, SIZES, arguments_clone),
        shape("arguments-borrow", true, SIZES, arguments_borrow),
        shape("many-values", true, SIZES, many_values),
        shape("many-statements", true, SIZES, many_statements),
        shape("sequential-ifs", true, SIZES, sequential_ifs),
        shape("when-branches", true, SIZES, when_branches),
        shape("nested-blocks", true, DEPTHS, nested_blocks),
        shape("nested-closures", true, DEPTHS, nested_closures),
        shape("many-closures", true, SIZES, many_closures),
        shape(
            "implicit-parameter-closures",
            true,
            SIZES,
            implicit_parameter_closures,
        ),
        shape(
            "nested-implicit-parameter-closures",
            true,
            DEPTHS,
            nested_implicit_parameter_closures,
        ),
        shape(
            "nested-trailing-lambdas",
            true,
            CALL_DEPTHS,
            nested_trailing_lambdas,
        ),
        shape(
            "nested-lambdas-with-parameters",
            true,
            CALL_DEPTHS,
            nested_lambdas_with_parameters,
        ),
        shape(
            "nested-generic-lambdas",
            true,
            CALL_DEPTHS,
            nested_generic_lambdas,
        ),
        shape(
            "nested-generic-implicit-lambdas",
            true,
            CALL_DEPTHS,
            nested_generic_implicit_lambdas,
        ),
        shape("nested-compose-ui", true, CALL_DEPTHS, nested_compose_ui),
        shape("composable-calls", true, SIZES, composable_calls),
        shape("composable-branches", true, SIZES, composable_branches),
        shape(
            "nested-composable-branches",
            true,
            DEPTHS,
            nested_composable_branches,
        ),
        shape("many-composables", true, SIZES, many_composables),
        shape("composable-lambdas", true, SIZES, composable_lambdas),
        shape("many-captures", true, SIZES, many_captures),
        shape("many-adapters", true, SIZES, many_adapters),
        shape("generic-closures", true, SIZES, generic_closures),
        shape("escaping-handlers", true, SIZES, escaping_handlers),
        shape("returned-closures", true, SIZES, returned_closures),
        shape(
            "closures-kept-past-blocks",
            true,
            SIZES,
            closures_kept_past_blocks,
        ),
        shape("handlers-kept-to-end", true, SIZES, handlers_kept_to_end),
        shape("live-handlers", true, SIZES, live_handlers),
        shape("many-functions", true, SIZES, many_functions),
        shape("public-functions", true, SIZES, public_functions),
        shape("struct-fields", true, SIZES, struct_fields),
        shape("enum-variants", true, SIZES, enum_variants),
        shape("when-patterns", true, SIZES, when_patterns),
        shape("string-concatenation", true, SIZES, string_concatenation),
        shape("live-borrows", true, SIZES, live_borrows),
        shape("moves-and-drops", true, SIZES, moves_and_drops),
        shape("template-parts", true, SIZES, template_parts),
        shape(
            "template-computed-parts",
            true,
            SIZES,
            template_computed_parts,
        ),
        shape("template-long-text", true, SIZES, template_long_text),
        shape("many-templates", true, SIZES, many_templates),
        shape("nested-templates", true, DEPTHS, nested_templates),
        shape("generic-call-sites", true, SIZES, generic_call_sites),
        shape("many-instances", true, SIZES, many_instances),
        shape("generic-call-chain", true, SIZES, generic_call_chain),
        shape("nested-generic-calls", true, DEPTHS, nested_generic_calls),
        shape("nested-generic-types", true, SIZES, nested_generic_types),
        shape("list-literal", true, SIZES, list_literal),
        shape("list-operations", true, SIZES, list_operations),
        shape("range-loops", true, SIZES, range_loops),
        shape("list-loops", true, SIZES, list_loops),
        shape("list-element-types", true, SIZES, list_element_types),
        shape("nested-lists", true, DEPTHS, nested_lists),
        shape("contract-fingerprints", true, SIZES, contract_fingerprints),
        shape("prelude-calls", false, SIZES, prelude_calls),
        shape("receiver-lambdas", true, SIZES, receiver_lambdas),
        shape(
            "nested-receiver-lambdas",
            true,
            CALL_DEPTHS,
            nested_receiver_lambdas,
        ),
        shape("context-overloads", true, SIZES, context_overloads),
    ]
}

const STAGES: [&str; 10] = [
    "lex", "parse", "abi", "type", "mir", "borrow", "compose", "inst", "lower", "clif",
];

/// The host ISA with the backend's settings, then `extra`, overridden by
/// `PRISM_TIMING_CRANELIFT`.
fn isa(extra: &[(&str, &str)]) -> cranelift_codegen::isa::OwnedTargetIsa {
    let mut flags = settings::builder();
    for (name, value) in prism_codegen::isa::settings()
        .into_iter()
        .chain(extra.iter().copied())
    {
        flags.set(name, value).unwrap();
    }
    if let Ok(overrides) = std::env::var("PRISM_TIMING_CRANELIFT") {
        for setting in overrides.split(',').filter(|setting| !setting.is_empty()) {
            let (name, value) = setting
                .split_once('=')
                .unwrap_or_else(|| panic!("not a name=value setting: {}", setting));
            flags.set(name, value).unwrap();
        }
    }
    cranelift_native::builder()
        .unwrap()
        .finish(settings::Flags::new(flags))
        .unwrap()
}

/// Runs `f`, returning its result and how long it took.
fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let result = f();
    (result, start.elapsed())
}

/// The time of each stage of `program`, `None` for the stages it does not reach.
fn compile(program: &Generated, compiled: bool) -> [Option<Duration>; 10] {
    let mut times = [None; 10];
    let source = program.source.as_str();
    let (tokens, time) = timed(|| prism_lexer::Lexer::new(source).tokenize_all());
    times[0] = Some(time);
    let (file, time) = timed(|| prism_parser::Parser::new(tokens).parse_source_file());
    times[1] = Some(time);
    let file = file.unwrap_or_else(|error| panic!("parse error: {:?}", error));
    let (abi, time) = timed(|| prism_abi::FileAbi::extract(&file, "bench", &|_| None));
    times[2] = Some(time);
    abi.unwrap_or_else(|error| panic!("ABI error: {:?}", error));
    let package = file.package.as_ref().map(|p| p.package_string());
    let ((tree, errors), time) = timed(|| {
        let mut checker = prism_typechecker::TypeChecker::new(package, "bench")
            .with_prelude(prism_typechecker::test_prelude());
        let tree = checker.check_module(&file);
        (tree, checker.errors)
    });
    times[3] = Some(time);
    assert!(errors.is_empty(), "type errors: {:?}", errors);
    let (mir, time) = timed(|| prism_mir::lower_file(&tree));
    times[4] = Some(time);
    let (elaborated, time) = timed(|| prism_borrowck::check_file(&mir));
    times[5] = Some(time);
    let elaborated = elaborated.unwrap_or_else(|errors| panic!("borrow errors: {:?}", errors));
    let (composed, time) = timed(|| prism_cli::compose_file(&elaborated));
    times[6] = Some(time);
    if !compiled {
        return times;
    }
    let files = [composed];
    let (instances, time) = timed(|| {
        let instances = prism_codegen::collect_instances(&[&files[0]])
            .unwrap_or_else(|errors| panic!("instances rejected: {:?}", errors));
        prism_codegen::check_instances(&instances)
            .unwrap_or_else(|errors| panic!("unsupported instances: {:?}", errors));
        instances
    });
    times[7] = Some(time);
    let mut module = JITModule::new(JITBuilder::with_isa(isa(&[]), default_libcall_names()));
    // The ISA the backend compiles large functions with (`prism_codegen::isa::is_large`).
    let large = isa(&[("regalloc_algorithm", "single_pass")]);
    let (units, time) = timed(|| {
        prism_codegen::check_file(&files[0])
            .unwrap_or_else(|errors| panic!("unsupported: {:?}", errors));
        let mut units = vec![prism_codegen::CodeUnit::lower(&files[0], &mut module).unwrap()];
        for instance in &instances {
            units.push(
                prism_codegen::CodeUnit::lower_named(&instance.file, &instance.symbol, &mut module)
                    .unwrap(),
            );
        }
        units
    });
    times[8] = Some(time);
    let ((), time) = timed(|| {
        let mut context = module.make_context();
        for unit in units {
            for function in unit.functions.into_iter().chain(unit.glue) {
                context.func = function.function;
                prism_codegen::link::define_function(
                    &mut module,
                    &*large,
                    function.id,
                    &mut context,
                )
                .unwrap();
                module.clear_context(&mut context);
            }
        }
    });
    times[9] = Some(time);
    times
}

/// The fastest time of each stage over five runs, or of one run when it takes over a second.
fn best_of(program: &Generated, compiled: bool) -> [Option<Duration>; 10] {
    let mut best = compile(program, compiled);
    let total: Duration = best.iter().flatten().sum();
    if total < Duration::from_secs(1) {
        for _ in 0..4 {
            let times = compile(program, compiled);
            for (best, time) in best.iter_mut().zip(times) {
                *best = best.zip(time).map(|(a, b)| a.min(b));
            }
        }
    }
    best
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn run_benchmark() {
    let filter: Option<Vec<String>> = std::env::var("PRISM_TIMING_SHAPES")
        .ok()
        .map(|names| names.split(',').map(str::to_string).collect());
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!("compile time of each stage in ms, {} build:", build);
    let mut header = format!("  {:<28}", "program");
    for stage in STAGES {
        write!(header, " {:>9}", stage).unwrap();
    }
    println!("{}   growth", header);
    let mut too_fast_growth = Vec::new();
    for shape in shapes() {
        if let Some(names) = &filter {
            if !names.iter().any(|name| shape.name.contains(name.as_str())) {
                continue;
            }
        }
        let mut previous: Option<(usize, [Option<Duration>; 10])> = None;
        // The size and `borrow` time of each program.
        let mut borrow_times: Vec<(usize, Duration)> = Vec::new();
        // The size and stage times of each program.
        let mut all_times: Vec<(usize, [Option<Duration>; 10])> = Vec::new();
        for &size in shape.sizes {
            let program = Generated {
                size,
                source: (shape.generate)(size),
            };
            let times = best_of(&program, shape.compiled);
            let mut line = format!("  {:<28}", format!("{}-{}", shape.name, size));
            for time in times {
                match time {
                    Some(time) => write!(line, " {:>9.1}", milliseconds(time)).unwrap(),
                    None => write!(line, " {:>9}", "-").unwrap(),
                }
            }
            if let Some((previous_size, previous_times)) = previous {
                let size_factor = program.size as f64 / previous_size as f64;
                // The growth of the stage that grew the most, among those taking at least 5 ms.
                let worst = STAGES
                    .iter()
                    .zip(times.iter().zip(previous_times))
                    .filter_map(|(stage, (time, previous))| {
                        let (time, previous) = (time.as_ref()?, previous?);
                        (milliseconds(*time) >= 5.0)
                            .then(|| (stage, time.as_secs_f64() / previous.as_secs_f64()))
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((stage, factor)) = worst {
                    write!(line, "   {} x{:.1} for x{:.1}", stage, factor, size_factor).unwrap();
                    if factor > 2.0 * size_factor {
                        too_fast_growth.push(format!("{}-{} ({})", shape.name, size, stage));
                    }
                }
            }
            println!("{}", line);
            borrow_times.extend(times[5].map(|time| (program.size, time)));
            all_times.push((program.size, times));
            let total: Duration = times.iter().flatten().sum();
            previous = Some((program.size, times));
            if total > Duration::from_secs(20) {
                println!("  {:<28} larger sizes skipped", shape.name);
                break;
            }
        }
        if let (true, [(first_size, first), .., (last_size, last)]) =
            (LINEAR_BORROW.contains(&shape.name), &borrow_times[..])
        {
            // The average growth per x4 of the size: the geometric mean of each step's.
            let steps = (*last_size as f64 / *first_size as f64).log(4.0);
            let growth = (last.as_secs_f64() / first.as_secs_f64()).powf(1.0 / steps);
            println!(
                "  {:<28} borrow x{:.2} per x4 on average",
                shape.name, growth
            );
            if growth > BORROW_GROWTH && milliseconds(*last) >= 5.0 {
                too_fast_growth.push(format!("{} (borrow x{:.2} per x4)", shape.name, growth));
            }
        }
        if let (true, [(first_size, first), .., (last_size, last)]) =
            (LINEAR_DEPTH.contains(&shape.name), &all_times[..])
        {
            // The average growth per doubling of the depth of each stage: the geometric mean
            // of each step's.
            let steps = (*last_size as f64 / *first_size as f64).log2();
            for (stage, (first, last)) in STAGES.iter().zip(first.iter().zip(last)) {
                let (Some(first), Some(last)) = (first, last) else {
                    continue;
                };
                let growth = (last.as_secs_f64() / first.as_secs_f64()).powf(1.0 / steps);
                if milliseconds(*last) >= 5.0 {
                    println!(
                        "  {:<28} {} x{:.2} per x2 on average",
                        shape.name, stage, growth
                    );
                    if growth > DEPTH_GROWTH {
                        too_fast_growth
                            .push(format!("{} ({} x{:.2} per x2)", shape.name, stage, growth));
                    }
                }
            }
        }
    }
    if std::env::var_os("PRISM_ASSERT_TIMING").is_some_and(|value| value == "1") {
        assert!(
            too_fast_growth.is_empty(),
            "compilation grows super-linearly for {:?}",
            too_fast_growth
        );
    }
}

#[test]
#[ignore = "a benchmark: run it with --ignored --nocapture"]
fn every_stage_compiles_in_linear_time() {
    // The stack of `prism`'s main thread, which the recursive passes run on.
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(run_benchmark)
        .unwrap()
        .join()
        .unwrap();
}

/// D4 of `standard-library`: Phase 2 of `n` small workspace files, each calling `println`,
/// `listOf` and two helpers, with the prelude scope built once and shared, and, for comparison,
/// with the prelude modules' ABIs ingested into each file's checker instead. The time per file
/// of the shared scope must stay flat as `n` grows (within x2 of the smallest size's, under
/// `PRISM_ASSERT_TIMING=1`): the prelude costs each file a pointer copy and a lookup per name,
/// never a check of the library.
fn prelude_benchmark() {
    let library = prism_library::Library::bundled();
    let prelude_abis: Vec<prism_abi::FileAbi> = library
        .prelude()
        .iter()
        .map(|module| {
            let file = prism_parser::parse_source(&library.module(module).unwrap().source).unwrap();
            let no_definition = |_: &str, _: &str| None;
            prism_abi::FileAbi::extract_with_origin(
                &file,
                module,
                &prism_abi::no_imports,
                &no_definition,
                true,
            )
            .unwrap()
        })
        .collect();
    let (scope, build_time) = timed(|| {
        std::sync::Arc::new(prism_typechecker::PreludeScope::from_abis(
            &prelude_abis.iter().collect::<Vec<_>>(),
        ))
    });
    println!(
        "prelude scope built once in {:.3} ms",
        milliseconds(build_time)
    );
    println!(
        "  {:<12} {:>14} {:>14} {:>18}",
        "files", "shared ms", "us per file", "ingested us/file"
    );
    let mut per_file = Vec::new();
    for files in [256, 1024, 4096] {
        let parsed: Vec<_> = (0..files)
            .map(|i| prism_parser::parse_source(&prelude_file(i)).unwrap())
            .collect();
        let check_all = |ingest: bool| {
            let start = Instant::now();
            for (i, file) in parsed.iter().enumerate() {
                let mut checker = prism_typechecker::TypeChecker::new(None, format!("f{}", i));
                if ingest {
                    for abi in &prelude_abis {
                        checker.ingest_file_abi(abi);
                    }
                } else {
                    checker = checker.with_prelude(std::sync::Arc::clone(&scope));
                }
                checker.check_module(file);
                assert!(checker.errors.is_empty(), "{:?}", checker.errors);
            }
            start.elapsed()
        };
        let shared = (0..3).map(|_| check_all(false)).min().unwrap();
        let ingested = (0..3).map(|_| check_all(true)).min().unwrap();
        let micros = shared.as_secs_f64() * 1e6 / files as f64;
        println!(
            "  {:<12} {:>14.1} {:>14.2} {:>18.2}",
            files,
            milliseconds(shared),
            micros,
            ingested.as_secs_f64() * 1e6 / files as f64
        );
        per_file.push(micros);
    }
    if std::env::var_os("PRISM_ASSERT_TIMING").is_some_and(|value| value == "1") {
        let (first, last) = (per_file[0], per_file[per_file.len() - 1]);
        assert!(
            last <= 2.0 * first,
            "the prelude's cost per file grows: {:.2} us at 256 files, {:.2} us at 4096",
            first,
            last
        );
    }
}

#[test]
#[ignore = "a benchmark: run it with --ignored --nocapture"]
fn prelude_costs_a_lookup_per_file() {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(prelude_benchmark)
        .unwrap()
        .join()
        .unwrap();
}
