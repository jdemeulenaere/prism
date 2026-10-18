//! The compile-time benchmark of the whole pipeline: how long each stage spends on generated
//! programs of growing size, so that a super-linear step shows up as a growth factor well above
//! the size factor.
//!
//! Each shape is generated at three sizes four times apart (256, 1024 and 4096 of the thing it
//! repeats; nesting shapes at depths 30, 60 and 120, since the parser rejects nesting deeper
//! than 256 with `E2002`). One file is compiled stage by stage, as `prism run` compiles it:
//!
//! - `lex` and `parse`: the lexer's tokens, then the parser's AST;
//! - `abi`: Phase 1, the file's ABI;
//! - `type`: Phase 2's type checker, which builds the typed tree;
//! - `mir`: MIR lowering;
//! - `borrow`: the borrow checker, with drop elaboration and capture inference;
//! - `lower`: the backend's support check and lowering to Cranelift IR;
//! - `clif`: Cranelift's compilation of every function and glue function.
//!
//! Closures are not compiled by the Tier 1 backend (`E6001`), so the shapes that use them stop
//! after `borrow`.
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
//! and fails when any stage of at least 5 ms grows more than twice as fast as the size, only
//! when `PRISM_ASSERT_TIMING=1` is set.

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
    sizes: [usize; 3],
    generate: fn(usize) -> String,
}

const SIZES: [usize; 3] = [256, 1024, 4096];
const DEPTHS: [usize; 3] = [30, 60, 120];

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
        shape("nested-closures", false, DEPTHS, nested_closures),
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
    ]
}

const STAGES: [&str; 8] = [
    "lex", "parse", "abi", "type", "mir", "borrow", "lower", "clif",
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
fn compile(program: &Generated, compiled: bool) -> [Option<Duration>; 8] {
    let mut times = [None; 8];
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
        let mut checker = prism_typechecker::TypeChecker::new(package, "bench");
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
    if !compiled {
        return times;
    }
    let mut module = JITModule::new(JITBuilder::with_isa(isa(&[]), default_libcall_names()));
    // The ISA the backend compiles large functions with (`prism_codegen::isa::is_large`).
    let large = isa(&[("regalloc_algorithm", "single_pass")]);
    let (unit, time) = timed(|| {
        prism_codegen::check_file(&elaborated)
            .unwrap_or_else(|errors| panic!("unsupported: {:?}", errors));
        prism_codegen::CodeUnit::lower(&elaborated, &mut module).unwrap()
    });
    times[6] = Some(time);
    let ((), time) = timed(|| {
        let mut context = module.make_context();
        for function in unit.functions.into_iter().chain(unit.glue) {
            context.func = function.function;
            prism_codegen::link::define_function(&mut module, &*large, function.id, &mut context)
                .unwrap();
            module.clear_context(&mut context);
        }
    });
    times[7] = Some(time);
    times
}

/// The fastest time of each stage over three runs, or of one run when it takes over a second.
fn best_of(program: &Generated, compiled: bool) -> [Option<Duration>; 8] {
    let mut best = compile(program, compiled);
    let total: Duration = best.iter().flatten().sum();
    if total < Duration::from_secs(1) {
        for _ in 0..2 {
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
        let mut previous: Option<(usize, [Option<Duration>; 8])> = None;
        for size in shape.sizes {
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
            let total: Duration = times.iter().flatten().sum();
            previous = Some((program.size, times));
            if total > Duration::from_secs(20) {
                println!("  {:<28} larger sizes skipped", shape.name);
                break;
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
