//! `prism run`, `prism dump-clif` and `prism dump-layout` on the binary: the scenarios of the `execution`,
//! `code-generation` and `compilation-and-abi` specs that look at the command line, timings or
//! generated code rather than at a program's output alone (which the executable fixtures cover).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_directory(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("prism_run_{}_{}", test_name, nanos));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Runs `prism` with a build cache shared by these tests in the temporary directory, so that they
/// exercise the cache without touching the user's (`cache_tests.rs` tests the cache itself).
fn prism(arguments: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_prism"))
        .env(
            "PRISM_CACHE",
            std::env::temp_dir().join("prism_run_cli_build_cache"),
        )
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("failed to run prism")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// A workspace whose `main.prism` holds `source`.
fn single_file(test_name: &str, source: &str) -> PathBuf {
    let directory = temporary_directory(test_name);
    write(&directory.join("main.prism"), source);
    directory
}

/// A workspace with `lib/math.prism` holding `math` and `app/main.prism` importing it.
fn two_files(test_name: &str, math: &str, main: &str) -> PathBuf {
    let directory = temporary_directory(test_name);
    write(
        &directory.join("lib/math.prism"),
        &format!("package lib\n{}", math),
    );
    write(
        &directory.join("app/main.prism"),
        &format!("package app\nimport /lib/math\n{}", main),
    );
    directory
}

/// The IR of function `symbol` in the `prism dump-clif` output `dump`.
fn function_ir<'a>(dump: &'a str, symbol: &str) -> &'a str {
    let header = format!("function {}\n", symbol);
    let start = dump
        .find(&header)
        .unwrap_or_else(|| panic!("no {} in\n{}", symbol, dump));
    let rest = &dump[start + header.len()..];
    let end = rest.find("\n}\n").map(|end| end + 3).unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn arguments_after_the_separator() {
    let directory = single_file("arguments", "fun main() { println(1) }\n");
    let output = prism(&["run", "main.prism", "--", "--timings", "x"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn reporting_timings() {
    let directory = single_file("timings", "fun main() { println(1) }\n");
    let output = prism(&["run", "--timings", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1\n");
    let lines: Vec<String> = stderr(&output).lines().map(str::to_string).collect();
    let stages = ["front end", "code generation", "linking", "total to main"];
    assert_eq!(lines.len(), stages.len(), "{:?}", lines);
    for (line, stage) in lines.iter().zip(stages) {
        let value = line
            .strip_prefix(&format!("timing: {} ", stage))
            .and_then(|rest| rest.strip_suffix(" ms"))
            .unwrap_or_else(|| panic!("unexpected timing line {:?}", line));
        let milliseconds: f64 = value.parse().unwrap();
        assert!(milliseconds >= 0.0);
    }
}

/// execution: "Cache options before the file".
#[test]
fn cache_options_before_the_file() {
    let directory = single_file("cache_options", "fun main() { println(1) }\n");
    let output = prism(
        &["run", "--no-cache", "--explain-cache", "main.prism"],
        &directory,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1\n");
    let stderr = stderr(&output);
    assert!(
        stderr.lines().any(|line| line == "Cache: off"),
        "{}",
        stderr
    );
    assert!(
        stderr.lines().any(|line| line == "  checked main.prism"),
        "{}",
        stderr
    );
}

#[test]
fn usage_errors_exit_with_status_1() {
    let directory = single_file("usage", "fun main() { println(1) }\n");
    for arguments in [
        &["run"][..],
        &["run", "--timings"],
        &["run", "--fast", "main.prism"],
        &["run", "main.prism", "extra"],
        &["dump-clif"],
        &["dump-layout"],
    ] {
        let output = prism(arguments, &directory);
        assert_eq!(output.status.code(), Some(1), "{:?}", arguments);
        assert_eq!(stdout(&output), "", "{:?}", arguments);
    }
}

#[test]
fn dumping_a_file_s_ir() {
    let directory = single_file(
        "dump_twice",
        "val LIMIT: Int = 10\nfun twice(n: Int): Int = n * 2\nfun main() { println(twice(LIMIT)) }\n",
    );
    let first = prism(&["dump-clif", "main.prism"], &directory);
    let second = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    assert_eq!(second.status.code(), Some(0));
    assert!(!stdout(&first).is_empty());
    assert_eq!(stdout(&first), stdout(&second));
    assert_eq!(stderr(&first), "");
}

#[test]
fn dumping_a_file_the_backend_rejects() {
    let directory = single_file("dump_rejected", "fun first(w: Widget): Int = 1\n");
    let output = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).starts_with(
            "error[E6001]: the Tier 1 backend does not support values of unknown type 'Widget' yet\n  --> main.prism:1:1\n"
        ),
        "{}",
        stderr(&output)
    );
}

#[test]
fn calls_within_and_across_files_are_indirect() {
    let directory = two_files(
        "indirect",
        "public fun twice(n: Int): Int = n * 2\n",
        "fun helper(): Int = 1\nfun main() { println(twice(helper())) }\n",
    );
    let dump = prism(&["dump-clif", "app/main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let main = function_ir(&dump, "app/main::main");
    for slot in ["app/main::helper#slot", "lib/math::twice#slot"] {
        // The slot's address, a load from it, then an indirect call of what was loaded.
        let global = main
            .lines()
            .find(|line| line.contains(&format!("symbol %\"{}\"", slot)))
            .and_then(|line| line.trim().split(' ').next())
            .unwrap_or_else(|| panic!("no global for {} in\n{}", slot, main));
        let address = main
            .lines()
            .find(|line| line.contains(&format!("symbol_value.i64 {}", global)))
            .and_then(|line| line.trim().split(' ').next())
            .unwrap_or_else(|| panic!("{} is not addressed in\n{}", global, main));
        let loaded = main
            .lines()
            .find(|line| line.contains("load.i64") && line.ends_with(&format!(" {}", address)))
            .and_then(|line| line.trim().split(' ').next())
            .unwrap_or_else(|| panic!("{} is not loaded in\n{}", address, main));
        assert!(
            main.lines()
                .any(|line| line.contains("call_indirect")
                    && line.contains(&format!(", {}(", loaded))),
            "{} is not called indirectly in\n{}",
            loaded,
            main
        );
    }
    let run = prism(&["run", "app/main.prism"], &directory);
    assert_eq!(run.status.code(), Some(0), "{}", stderr(&run));
    assert_eq!(stdout(&run), "2\n");
}

#[test]
fn signatures_of_scalar_functions() {
    let directory = single_file(
        "signatures",
        "fun f(n: Int, b: Bool, c: Char, d: Double, r: &Int): Double = d\nfun g(n: Int) {}\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    assert!(
        function_ir(&dump, "main::f")
            .starts_with("function %\"main::f\"(i64, i8 uext, i32, f64, i64) -> f64 "),
        "{}",
        dump
    );
    let g = function_ir(&dump, "main::g");
    assert!(g.starts_with("function %\"main::g\"(i64) "), "{}", g);
    assert!(!g.lines().next().unwrap().contains("->"), "{}", g);
}

#[test]
fn symbols_of_a_file() {
    let directory = temporary_directory("symbols");
    write(
        &directory.join("lib/math.prism"),
        "package lib\npublic fun Int.double(): Int = this * 2\npublic fun scale(a: Int, b: Int = 2): Int = a * b\npublic val BASE: Int = 3\n",
    );
    let dump = prism(&["dump-clif", "lib/math.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let headers: Vec<String> = stdout(&dump)
        .lines()
        .filter_map(|line| {
            line.strip_prefix("function ")
                .filter(|rest| !rest.starts_with('%'))
        })
        .map(str::to_string)
        .collect();
    assert_eq!(
        headers,
        [
            "lib/math::Int.double",
            "lib/math::scale",
            "lib/math::scale#default(b)",
            "lib/math::BASE#init",
            "lib/math::BASE#get",
        ]
    );
}

#[test]
fn symbols_of_extensions_differing_by_context() {
    let directory = temporary_directory("context-symbols");
    write(
        &directory.join("lib/layout.prism"),
        "package lib\npublic struct Modifier : Copy { val fraction: Double }\npublic struct RowScope : Copy {}\npublic struct ColumnScope : Copy {}\ncontext(scope: RowScope) public fun Modifier.weight(w: Double, fill: Bool = true): Modifier = Modifier(w)\ncontext(scope: ColumnScope) public fun Modifier.weight(w: Double): Modifier = Modifier(w)\n",
    );
    let dump = prism(&["dump-clif", "lib/layout.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let headers: Vec<String> = stdout(&dump)
        .lines()
        .filter_map(|line| {
            line.strip_prefix("function ")
                .filter(|rest| !rest.starts_with('%'))
        })
        .map(str::to_string)
        .collect();
    // The receiver and the context are written as the ABI writes their type constructors.
    assert_eq!(
        headers,
        [
            "lib/layout::lib/layout.Modifier.weight#context(lib/layout.RowScope)",
            "lib/layout::lib/layout.Modifier.weight#context(lib/layout.RowScope)#default(fill)",
            "lib/layout::lib/layout.Modifier.weight#context(lib/layout.ColumnScope)",
        ]
    );
}

#[test]
fn compiling_independent_source_files() {
    let directory = two_files(
        "independent",
        "public fun twice(n: Int): Int = n * 2\n",
        "fun main() { println(twice(2)) }\n",
    );
    let dump = prism(&["dump-clif", "app/main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    // Only the entry's own functions are generated; `lib/math` appears only as the symbol of a
    // slot.
    let headers: Vec<&str> = dump
        .lines()
        .filter_map(|line| line.strip_prefix("function "))
        .filter(|rest| !rest.starts_with('%'))
        .collect();
    assert_eq!(headers, ["app/main::main"]);
    let mentions: Vec<&str> = dump
        .lines()
        .filter(|line| line.contains("lib/math"))
        .collect();
    assert_eq!(mentions.len(), 1, "{:?}", mentions);
    assert!(mentions[0].contains("symbol %\"lib/math::twice#slot\""));
}

#[test]
fn code_generation_independent_of_dependency_bodies() {
    let directory = two_files(
        "independent_bodies",
        "public fun twice(n: Int): Int = n * 2\n",
        "fun main() { println(twice(2)) }\n",
    );
    let before = prism(&["dump-clif", "app/main.prism"], &directory);
    let run_before = prism(&["run", "app/main.prism"], &directory);
    write(
        &directory.join("lib/math.prism"),
        "package lib\npublic fun twice(n: Int): Int = n + n\n",
    );
    let after = prism(&["dump-clif", "app/main.prism"], &directory);
    let run_after = prism(&["run", "app/main.prism"], &directory);
    assert_eq!(before.status.code(), Some(0), "{}", stderr(&before));
    assert_eq!(stdout(&before), stdout(&after));
    for run in [run_before, run_after] {
        assert_eq!(run.status.code(), Some(0), "{}", stderr(&run));
        assert_eq!(stdout(&run), "4\n");
    }
}

#[test]
fn a_dependency_with_errors_stops_the_run() {
    let directory = temporary_directory("dependency_errors");
    write(
        &directory.join("lib/text.prism"),
        "package lib\npublic fun shout(n: Int): Int = nope\n",
    );
    write(
        &directory.join("app/main.prism"),
        "package app\nimport /lib/text\nfun main() { println(shout(1)) }\n",
    );
    let dump = prism(&["dump-clif", "app/main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    assert!(stdout(&dump).contains("function app/main::main\n"));
    let run = prism(&["run", "app/main.prism"], &directory);
    assert_eq!(run.status.code(), Some(1));
    assert_eq!(stdout(&run), "");
    assert!(
        stderr(&run).starts_with("error[E4001]: unresolved symbol 'nope'\n  --> lib/text.prism:2:"),
        "{}",
        stderr(&run)
    );
}

/// The reference program `examples/run/main.prism`, which the timing benchmark also runs.
#[test]
fn the_reference_program_runs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = prism(
        &["run", "--check-allocations", "examples/run/main.prism"],
        &root,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "2 3 5 7 11 13 17 19 23 29 31 37 41 43 47 \n15\n12\n5050\n735\n0.3333333333333333\n-3\ntrue\nfalse\n-2\n0\nquadrant 4\n-24\n15 primes up to 50\n15 primes up to 50, 18 characters\n20\n10\ntrue\nprimes: 15\n5\n-1\n60\n18\n-20\n134\nok\nlimit = 50\n5 removed, 4 left, total 154, true\ntrue\n(6, -3)\n202, 20, 34, 52\n"
    );
    assert_eq!(stderr(&output), "");
}

/// code-generation: "Signature of a function over structs" and "A field added keeps the
/// signature".
#[test]
fn signatures_of_functions_over_structs() {
    let source =
        "struct Point : Copy { val x: Int, val y: Int }\nfun f(p: Point, n: Int): Point = p\n";
    let directory = single_file("struct_signatures", source);
    let signature_line = |directory: &Path| {
        let dump = prism(&["dump-clif", "main.prism"], directory);
        assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
        let dump = stdout(&dump);
        function_ir(&dump, "main::f")
            .lines()
            .next()
            .unwrap()
            .to_string()
    };
    let before = signature_line(&directory);
    // The result's address, the address of `p` and the 64-bit `n`; nothing is returned.
    assert!(
        before.starts_with("function %\"main::f\"(i64, i64, i64) "),
        "{}",
        before
    );
    assert!(!before.contains("->"), "{}", before);
    write(
        &directory.join("main.prism"),
        &source.replace("val y: Int }", "val y: Int, val z: Int }"),
    );
    assert_eq!(signature_line(&directory), before);
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Dumping a file's layouts".
#[test]
fn dumping_a_file_s_layouts() {
    let directory = single_file(
        "dump_layout",
        "struct Point : Copy { val x: Double, val y: Double }\n\
         enum Color { Red, Green }\n\
         fun f(n: Int?): Int = n ?: 0\n",
    );
    let first = prism(&["dump-layout", "main.prism"], &directory);
    let second = prism(&["dump-layout", "main.prism"], &directory);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    assert_eq!(stdout(&first), stdout(&second));
    assert_eq!(
        stdout(&first),
        "struct main::Point : Copy (size 16, align 8)\n  x: Float at 0\n  y: Float at 8\n\
         enum main::Color (size 1, align 1, tag u8 at 0)\n  Red\n  Green\n\
         nullable Int? (size 16, align 8, tag u8 at 0)\n  payload Int at 8\n"
    );
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Dumping a file with errors".
#[test]
fn dumping_the_layouts_of_a_file_with_errors() {
    let directory = single_file(
        "dump_layout_errors",
        "struct P { val x: Int }\nfun f(): Int = nope\n",
    );
    let output = prism(&["dump-layout", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("error[E4001]"),
        "{}",
        stderr(&output)
    );
    fs::remove_dir_all(&directory).ok();
}

/// The headers of the functions of the `prism dump-clif` output `dump`, in order.
fn function_headers(dump: &str) -> Vec<&str> {
    dump.lines()
        .filter_map(|line| line.strip_prefix("function "))
        .filter(|rest| !rest.starts_with('%'))
        .collect()
}

/// Whether the IR `ir` calls `symbol` directly: imports it as a function and calls that.
fn calls_directly(ir: &str, symbol: &str) -> bool {
    let import = format!(" = %\"{}\" ", symbol);
    ir.lines()
        .find_map(|line| {
            let (name, _) = line.trim().split_once(&import)?;
            Some(name.to_string())
        })
        .is_some_and(|name| ir.contains(&format!("call {}(", name)))
}

/// code-generation: "Drop glue in the IR dump".
#[test]
fn drop_glue_in_the_ir_dump() {
    let directory = single_file(
        "drop_glue",
        "struct User { val name: String }\nfun f(u: User) {}\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    assert_eq!(
        function_headers(&dump),
        ["main::f", "main#drop(main::User)"]
    );
    assert!(
        calls_directly(function_ir(&dump, "main::f"), "main#drop(main::User)"),
        "{}",
        dump
    );
    // The glue drops the string field through the runtime, and has no slot.
    let glue = function_ir(&dump, "main#drop(main::User)");
    assert!(calls_directly(glue, "prism_rt_string_drop"), "{}", glue);
    assert!(!dump.contains("main#drop(main::User)#slot"), "{}", dump);
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Clone glue in the IR dump".
#[test]
fn clone_glue_in_the_ir_dump() {
    let directory = single_file(
        "clone_glue",
        "struct User : Clone { val name: String, val id: Int }\nfun f(u: &User): User = u.clone()\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    assert_eq!(
        function_headers(&dump),
        ["main::f", "main#clone(main::User)"]
    );
    assert!(
        calls_directly(function_ir(&dump, "main::f"), "main#clone(main::User)"),
        "{}",
        dump
    );
    let glue = function_ir(&dump, "main#clone(main::User)");
    assert!(calls_directly(glue, "prism_rt_string_clone"), "{}", glue);
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Glue of one type in two files".
#[test]
fn glue_of_one_type_in_two_files() {
    let directory = temporary_directory("glue_two_files");
    write(
        &directory.join("lib/user.prism"),
        "package lib\npublic struct User { val name: String }\npublic fun drop(u: User): Unit {}\n",
    );
    write(
        &directory.join("app/main.prism"),
        "package app\nimport /lib/user\nfun keep(u: User) {}\nfun main() { drop(User(\"a\")); keep(User(\"b\")) }\n",
    );
    for (file, glue) in [
        ("lib/user.prism", "lib/user#drop(lib/user::User)"),
        ("app/main.prism", "app/main#drop(lib/user::User)"),
    ] {
        let dump = prism(&["dump-clif", file], &directory);
        assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
        let dump = stdout(&dump);
        assert_eq!(function_headers(&dump).last(), Some(&glue), "{}", dump);
    }
    let run = prism(
        &["run", "--check-allocations", "app/main.prism"],
        &directory,
    );
    assert_eq!(run.status.code(), Some(0), "{}", stderr(&run));
    assert_eq!(stderr(&run), "");
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Signature of a function over strings".
#[test]
fn signature_of_a_function_over_strings() {
    let directory = single_file(
        "string_signature",
        "fun greet(name: String, n: Int): String = name\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let signature = function_ir(&dump, "main::greet").lines().next().unwrap();
    // The result's address, the address of `name` and the 64-bit `n`; nothing is returned.
    assert!(
        signature.starts_with("function %\"main::greet\"(i64, i64, i64) "),
        "{}",
        signature
    );
    assert!(!signature.contains("->"), "{}", signature);
    fs::remove_dir_all(&directory).ok();
}

/// execution: "A program that frees what it allocates".
#[test]
fn a_program_that_frees_what_it_allocates() {
    let directory = single_file(
        "frees",
        "fun main() { val s = \"a\" + \"b\"; println(s) }\n",
    );
    let output = prism(&["run", "--check-allocations", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "ab\n");
    assert_eq!(stderr(&output), "");
    fs::remove_dir_all(&directory).ok();
}

/// execution: "A runtime error is not checked".
#[test]
fn a_runtime_error_is_not_checked() {
    let directory = single_file(
        "runtime_error_unchecked",
        "fun zero(): Int = 0\nfun main() { val s = \"a\" + \"b\"; println(1 / zero()) }\n",
    );
    let output = prism(&["run", "--check-allocations", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "panic: division by zero\n  at main.prism:2:41\n"
    );
    fs::remove_dir_all(&directory).ok();
}

/// `--check-allocations` combines with `--timings` and with the program's arguments.
#[test]
fn the_allocation_check_combines_with_the_other_options() {
    let directory = single_file(
        "check_and_timings",
        "fun main() { println(1.toString()) }\n",
    );
    let output = prism(
        &[
            "run",
            "--timings",
            "--check-allocations",
            "main.prism",
            "--",
            "--check-allocations",
        ],
        &directory,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1\n");
    assert!(
        stderr(&output).contains("timing: total to main"),
        "{}",
        stderr(&output)
    );
    let output = prism(&["run", "main.prism", "--check-allocations"], &directory);
    assert_eq!(output.status.code(), Some(1), "an option after the file");
    fs::remove_dir_all(&directory).ok();
}

/// The name the IR `ir` gives the function it imports as `symbol`, if it imports it.
fn imported_name<'a>(ir: &'a str, symbol: &str) -> Option<&'a str> {
    let import = format!(" = %\"{}\" ", symbol);
    ir.lines()
        .find_map(|line| Some(line.trim().split_once(&import)?.0))
}

/// The lines of the IR `ir` that call the function it imports as `symbol`.
fn calls_of<'a>(ir: &'a str, symbol: &str) -> Vec<&'a str> {
    let Some(name) = imported_name(ir, symbol) else {
        return Vec::new();
    };
    let call = format!("call {}(", name);
    ir.lines().filter(|line| line.contains(&call)).collect()
}

/// code-generation: "Constructing a box allocates its contents' size".
#[test]
fn constructing_a_box_allocates_its_contents_size() {
    let directory = single_file(
        "box_allocation",
        "struct Point : Copy { val x: Int, val y: Int }\nfun f(p: Point): Heap<Point> = Heap(p)\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let ir = function_ir(&dump, "main::f");
    let calls = calls_of(ir, "prism_rt_allocate");
    assert_eq!(calls.len(), 1, "{}", ir);
    // The size, then the alignment, as the constants Cranelift annotates the call with.
    let (_, constants) = calls[0].split_once(';').expect("annotated constants");
    let values: Vec<&str> = constants
        .split(',')
        .map(|constant| constant.trim().rsplit(' ').next().unwrap_or(""))
        .collect();
    assert_eq!(values, ["16", "8"], "{}", calls[0]);
    // The point is written into the allocation after the call.
    let after = &ir[ir.find(calls[0]).unwrap()..];
    assert!(after.contains("store notrap aligned"), "{}", ir);
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Box glue in the IR dump".
#[test]
fn box_glue_in_the_ir_dump() {
    let directory = single_file(
        "box_glue",
        "enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }\nfun f(e: Expr) {}\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    assert_eq!(
        function_headers(&dump),
        [
            "main::f",
            "main#drop(Heap<main::Expr>)",
            "main#drop(main::Expr)"
        ]
    );
    let enum_glue = function_ir(&dump, "main#drop(main::Expr)");
    assert!(
        calls_directly(enum_glue, "main#drop(Heap<main::Expr>)"),
        "{}",
        enum_glue
    );
    let box_glue = function_ir(&dump, "main#drop(Heap<main::Expr>)");
    let contents = calls_of(box_glue, "main#drop(main::Expr)");
    let frees = calls_of(box_glue, "prism_rt_free");
    assert_eq!((contents.len(), frees.len()), (1, 1), "{}", box_glue);
    assert!(
        box_glue.find(contents[0]) < box_glue.find(frees[0]),
        "the contents are dropped before the allocation is freed:\n{}",
        box_glue
    );
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Box clone glue in the IR dump".
#[test]
fn box_clone_glue_in_the_ir_dump() {
    let directory = single_file(
        "box_clone_glue",
        "fun f(h: &Heap<Int>): Heap<Int> = h.clone()\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    assert!(
        calls_directly(function_ir(&dump, "main::f"), "main#clone(Heap<Int>)"),
        "{}",
        dump
    );
    let glue = function_ir(&dump, "main#clone(Heap<Int>)");
    assert!(calls_directly(glue, "prism_rt_allocate"), "{}", glue);
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Equality glue in the IR dump".
#[test]
fn equality_glue_in_the_ir_dump() {
    let directory = single_file(
        "equality_glue",
        "enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }\nfun same(a: &Expr, b: &Expr): Bool = a == b\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let glue = function_ir(&dump, "main#equals(main::Expr)");
    assert!(calls_directly(glue, "main#equals(main::Expr)"), "{}", glue);
    let same = function_ir(&dump, "main::same");
    assert!(calls_directly(same, "main#equals(main::Expr)"), "{}", same);
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "The bounds check in the IR dump" (D8 of `collections`).
#[test]
fn the_bounds_check_in_the_ir_dump() {
    let directory = single_file(
        "list_bounds_check",
        "fun get(xs: &List<Int>, i: Int): Int = xs[i]\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let get = function_ir(&dump, "main::get");
    // The size is the word at offset 8, compared unsigned with the index.
    assert!(get.contains("load.i64 notrap aligned "), "{}", get);
    assert!(get.contains("+8\n"), "{}", get);
    assert!(get.contains("icmp.i64 uge"), "{}", get);
    assert!(
        calls_directly(get, "prism_rt_list_index_out_of_range"),
        "{}",
        get
    );
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "List glue in the IR dump" (D15 of `collections`).
#[test]
fn list_glue_in_the_ir_dump() {
    let directory = single_file("list_glue", "fun f(xs: List<String>) {}\n");
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    assert_eq!(
        function_headers(&dump),
        ["main::f", "main#drop(List<String>)"]
    );
    assert!(
        calls_directly(function_ir(&dump, "main::f"), "main#drop(List<String>)"),
        "{}",
        dump
    );
    let glue = function_ir(&dump, "main#drop(List<String>)");
    assert!(calls_directly(glue, "prism_rt_string_drop"), "{}", glue);
    assert!(calls_directly(glue, "prism_rt_list_free"), "{}", glue);
    // Each element is dropped before the buffer is freed.
    let drop = glue.find("call fn1(").expect("the string drop");
    let free = glue.find("call fn2(").expect("the free");
    assert!(drop < free, "{}", glue);
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "List equality glue in the IR dump" (D15 of `collections`).
#[test]
fn list_equality_glue_in_the_ir_dump() {
    let directory = single_file(
        "list_equality_glue",
        "fun same(a: &List<Int>, b: &List<Int>): Bool = a == b\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    assert!(
        function_headers(&dump).contains(&"main#equals(List<Int>)"),
        "{}",
        dump
    );
    let same = function_ir(&dump, "main::same");
    assert!(calls_directly(same, "main#equals(List<Int>)"), "{}", same);
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "A large struct is compared by equality glue".
#[test]
fn a_large_struct_is_compared_by_equality_glue() {
    let directory = single_file(
        "large_equality_glue",
        "struct P : Copy { val a: Int, val b: Int, val c: Int, val d: Int }\n\
         struct Q : Copy { val a: P, val b: P, val c: P, val d: P }\n\
         struct R : Copy { val a: Q, val b: Q, val c: Q }\n\
         fun same(x: R, y: R): Bool = x == y\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let same = function_ir(&dump, "main::same");
    assert!(calls_directly(same, "main#equals(main::R)"), "{}", same);
    let large = function_ir(&dump, "main#equals(main::R)");
    // `a` and `b` use the whole budget of 32 comparisons: 16 pairs of `Int`s are loaded from
    // each value and only `c` is compared by a call.
    assert_eq!(
        calls_of(large, "main#equals(main::Q)").len(),
        1,
        "{}",
        large
    );
    assert_eq!(
        large.matches("load.i64 notrap aligned v0").count(),
        32,
        "{}",
        large
    );
    let part = function_ir(&dump, "main#equals(main::Q)");
    assert!(!part.contains("= %\"main#equals("), "{}", part);
    assert_eq!(
        part.matches("load.i64 notrap aligned v0").count(),
        16,
        "{}",
        part
    );
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Glue of an imported recursive type".
#[test]
fn glue_of_an_imported_recursive_type() {
    let directory = temporary_directory("glue_imported_recursive");
    write(
        &directory.join("lib/tree.prism"),
        "package lib\npublic enum Tree { Leaf, Node(Heap<Tree>, Int, Heap<Tree>) }\npublic fun leaf(): Tree = Tree.Leaf\n",
    );
    write(
        &directory.join("app/main.prism"),
        "package app\nimport /lib/tree\nfun main() { val t = Tree.Node(Heap(leaf()), 1, Heap(leaf())); println(t == t) }\n",
    );
    let run = prism(
        &["run", "--check-allocations", "app/main.prism"],
        &directory,
    );
    assert_eq!(run.status.code(), Some(0), "{}", stderr(&run));
    assert_eq!(stdout(&run), "true\n");
    let dump = prism(&["dump-clif", "app/main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let headers = function_headers(&dump);
    for glue in [
        "app/main#drop(lib/tree::Tree)",
        "app/main#drop(Heap<lib/tree::Tree>)",
        "app/main#equals(lib/tree::Tree)",
    ] {
        assert!(headers.contains(&glue), "no {} in {:?}", glue, headers);
    }
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "A template calls the runtime once" (`string-templates`).
#[test]
fn a_template_calls_the_runtime_once() {
    let directory = single_file(
        "template_clif",
        "fun label(n: Int, d: Double, s: &String): String = \"Count: $n of $s at $d\"\n",
    );
    let dump = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let label = function_ir(&dump, "main::label");
    // The callee `fnN` of each call, besides the stack check's panic.
    let callee = |name: &str| {
        label
            .lines()
            .find(|line| line.contains(&format!("= %\"{}\"", name)))
            .and_then(|line| line.split_whitespace().next())
            .map(str::to_string)
    };
    let panic = callee("prism_rt_panic");
    let calls: Vec<&str> = label
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("call "))
        .filter(|line| {
            panic
                .as_ref()
                .is_none_or(|panic| !line.starts_with(&format!("call {}(", panic)))
        })
        .collect();
    let template = callee("prism_rt_string_template").expect("the template function is imported");
    assert_eq!(calls.len(), 1, "{}", label);
    assert!(
        calls[0].starts_with(&format!("call {}(", template)),
        "{}",
        label
    );
    for other in [
        "prism_rt_string_concatenate",
        "prism_rt_int_to_string",
        "prism_rt_double_to_string",
    ] {
        assert!(!label.contains(other), "{}", label);
    }
}

// Instances of generics (the `monomorphization` change).

/// Whether `ir` loads the address in the slot `slot` and calls what it loaded indirectly.
fn calls_through_slot(ir: &str, slot: &str) -> bool {
    let Some(global) = ir
        .lines()
        .find(|line| line.contains(&format!("symbol %\"{}\"", slot)))
        .and_then(|line| line.trim().split(' ').next())
    else {
        return false;
    };
    let Some(address) = ir
        .lines()
        .find(|line| line.contains(&format!("symbol_value.i64 {}", global)))
        .and_then(|line| line.trim().split(' ').next())
    else {
        return false;
    };
    let Some(loaded) = ir
        .lines()
        .find(|line| line.contains("load.i64") && line.ends_with(&format!(" {}", address)))
        .and_then(|line| line.trim().split(' ').next())
    else {
        return false;
    };
    ir.lines()
        .any(|line| line.contains("call_indirect") && line.contains(&format!(", {}(", loaded)))
}

/// A workspace with `lib/box.prism` holding `box_source` and `app/main.prism` importing it.
fn box_workspace(test_name: &str, box_source: &str, main: &str) -> PathBuf {
    let directory = temporary_directory(test_name);
    write(
        &directory.join("lib/box.prism"),
        &format!("package lib\n{}", box_source),
    );
    write(
        &directory.join("app/main.prism"),
        &format!("package app\nimport /lib/box\n{}", main),
    );
    directory
}

/// code-generation: "Calls of an instance go through its slot".
#[test]
fn calls_of_an_instance_go_through_its_slot() {
    let directory = box_workspace(
        "instance_slot",
        "public fun <T> keep(x: T): T = x\n",
        "fun main() { println(keep(1)) }\n",
    );
    let dump = prism(&["dump-clif", "app/main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let main = function_ir(&dump, "app/main::main");
    assert!(
        calls_through_slot(main, "lib/box::keep<Int>#slot"),
        "{}",
        main
    );
    // The file's own dump holds no instance.
    assert_eq!(function_headers(&dump), ["app/main::main"]);
    let run = prism(&["run", "app/main.prism"], &directory);
    assert_eq!(run.status.code(), Some(0), "{}", stderr(&run));
    assert_eq!(stdout(&run), "1\n");
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "One instance for every caller".
#[test]
fn one_instance_for_every_caller() {
    let directory = box_workspace(
        "one_instance",
        "public fun <T> keep(x: T): T = x\n",
        "import /app/other\nfun main() { println(keep(1)); println(other()) }\n",
    );
    write(
        &directory.join("app/other.prism"),
        "package app\nimport /lib/box\npublic fun other(): Int = keep(2)\n",
    );
    let dump = prism(&["dump-clif", "--instances", "app/main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    assert_eq!(function_headers(&stdout(&dump)), ["lib/box::keep<Int>"]);
    let run = prism(&["run", "app/main.prism"], &directory);
    assert_eq!(stdout(&run), "1\n2\n", "{}", stderr(&run));
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Dumping a program's instances".
#[test]
fn dumping_a_program_s_instances() {
    let directory = single_file(
        "instance_dump",
        "fun <T> keep(x: T): T = x\nfun main() { println(keep(1)); println(keep(\"a\")) }\n",
    );
    let first = prism(&["dump-clif", "--instances", "main.prism"], &directory);
    let second = prism(&["dump-clif", "--instances", "main.prism"], &directory);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    assert_eq!(stdout(&first), stdout(&second));
    assert_eq!(
        function_headers(&stdout(&first)),
        ["main::keep<Int>", "main::keep<String>"]
    );
    fs::remove_dir_all(&directory).ok();
}

/// mir: "Instance MIR Dump" and code-generation: "Instance Code Units Are Dumped on Request",
/// for a program with errors.
#[test]
fn instance_dumps_of_a_program_with_errors() {
    let directory = box_workspace(
        "instance_dump_errors",
        "public fun <T> keep(x: T): T = nope\n",
        "fun main() { println(keep(1)) }\n",
    );
    for flag in [["dump-mir", "--instances"], ["dump-clif", "--instances"]] {
        let output = prism(&[flag[0], flag[1], "app/main.prism"], &directory);
        assert_eq!(output.status.code(), Some(1), "{}", flag[0]);
        assert_eq!(stdout(&output), "", "{}", flag[0]);
        assert!(stderr(&output).contains("E4001"), "{}", stderr(&output));
    }
    fs::remove_dir_all(&directory).ok();
}

/// mir: "Dumping the instances of a program", on the binary.
#[test]
fn dumping_the_instances_of_a_program() {
    let directory = box_workspace(
        "instance_mir",
        "public fun <T> keep(x: T): T = x\n",
        "fun main() { println(keep(1)); println(keep(true)) }\n",
    );
    let first = prism(&["dump-mir", "--instances", "app/main.prism"], &directory);
    let second = prism(&["dump-mir", "--instances", "app/main.prism"], &directory);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    let text = stdout(&first);
    assert_eq!(text, stdout(&second));
    let headers: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("fun "))
        .collect();
    assert_eq!(
        headers,
        [
            "fun lib/box::keep<Bool>(_1: Bool): Bool @2:1",
            "fun lib/box::keep<Int>(_1: Int): Int @2:1"
        ]
    );
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Symbols of instances".
#[test]
fn symbols_of_instances() {
    let directory = box_workspace(
        "instance_symbols",
        "public struct Box<T>(val v: T)\n\
         public fun <T> keep(x: T): T = x\n\
         public fun <T> Box<T>.get(): &T = &v\n\
         public fun <T : Copy> pad(x: T, y: T = x): T = y\n",
        "import /lib/user\n\
         fun main() { val u = keep(User(\"a\")); println(Box(1).get()); println(pad(2)) }\n",
    );
    write(
        &directory.join("lib/user.prism"),
        "package lib\npublic struct User { val name: String }\n",
    );
    let dump = prism(&["dump-clif", "--instances", "app/main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let headers = function_headers(&dump);
    for symbol in [
        "lib/box::keep<lib/user::User>",
        "lib/box::lib/box.Box.get<Int>",
        "lib/box::pad<Int>",
        "lib/box::pad<Int>#default(y)",
    ] {
        assert!(headers.contains(&symbol), "no {} in {:?}", symbol, headers);
    }
    let run = prism(
        &["run", "--check-allocations", "app/main.prism"],
        &directory,
    );
    assert_eq!(stdout(&run), "1\n2\n", "{}", stderr(&run));
    fs::remove_dir_all(&directory).ok();
}

/// code-generation: "Glue of an instance".
#[test]
fn glue_of_an_instance() {
    let directory = single_file(
        "instance_glue",
        "struct User : Clone { val name: String }\n\
         fun <T> discard(x: T) {}\n\
         fun <T : Clone> copy(x: &T): T = x.clone()\n\
         fun main() { discard(User(\"a\")); val u = copy(&User(\"b\")); println(1) }\n",
    );
    let dump = prism(&["dump-clif", "--instances", "main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let headers = function_headers(&dump);
    let position = |symbol: &str| {
        headers
            .iter()
            .position(|header| *header == symbol)
            .unwrap_or_else(|| panic!("no {} in {:?}", symbol, headers))
    };
    assert!(
        position("main::discard<main::User>")
            < position("main::discard<main::User>#drop(main::User)")
    );
    assert!(
        position("main::copy<main::User>") < position("main::copy<main::User>#clone(main::User)")
    );
    assert!(calls_directly(
        function_ir(&dump, "main::discard<main::User>"),
        "main::discard<main::User>#drop(main::User)"
    ));
    let run = prism(&["run", "--check-allocations", "main.prism"], &directory);
    assert_eq!(run.status.code(), Some(0), "{}", stderr(&run));
    assert_eq!(stdout(&run), "1\n");
    fs::remove_dir_all(&directory).ok();
}

/// compilation-and-abi: "A generic body change leaves its callers' code unchanged".
#[test]
fn a_generic_body_change_leaves_its_callers_code_unchanged() {
    let directory = box_workspace(
        "generic_body_change",
        "public fun <T> pick(a: T, b: T): T = a\n",
        "fun main() { println(pick(1, 2)) }\n",
    );
    let before = prism(&["dump-clif", "app/main.prism"], &directory);
    let run_before = prism(&["run", "app/main.prism"], &directory);
    write(
        &directory.join("lib/box.prism"),
        "package lib\npublic fun <T> pick(a: T, b: T): T = b\n",
    );
    let after = prism(&["dump-clif", "app/main.prism"], &directory);
    let run_after = prism(&["run", "app/main.prism"], &directory);
    assert_eq!(before.status.code(), Some(0), "{}", stderr(&before));
    assert_eq!(stdout(&before), stdout(&after));
    assert_eq!(stdout(&run_before), "1\n", "{}", stderr(&run_before));
    assert_eq!(stdout(&run_after), "2\n", "{}", stderr(&run_after));
    fs::remove_dir_all(&directory).ok();
}

/// compilation-and-abi: "A generic body calls private helpers": the instance calls the helper
/// and the other instance through their slots.
#[test]
fn a_generic_body_calls_private_helpers() {
    let directory = box_workspace(
        "private_helpers",
        "fun helper(n: Int): Int = n + 1\n\
         fun <T> inner(x: T): T = x\n\
         public fun <T> outer(x: T, n: Int): Int { val y = inner(x); return helper(n) }\n",
        "fun main() { println(outer(\"a\", 1)) }\n",
    );
    let dump = prism(&["dump-clif", "--instances", "app/main.prism"], &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    let dump = stdout(&dump);
    let outer = function_ir(&dump, "lib/box::outer<String>");
    assert!(
        calls_through_slot(outer, "lib/box::helper#slot"),
        "{}",
        outer
    );
    assert!(
        calls_through_slot(outer, "lib/box::inner<String>#slot"),
        "{}",
        outer
    );
    let run = prism(
        &["run", "--check-allocations", "app/main.prism"],
        &directory,
    );
    assert_eq!(stdout(&run), "2\n", "{}", stderr(&run));
    fs::remove_dir_all(&directory).ok();
}

/// The `prism dump-clif` output of `source`, which must succeed.
fn clif_of(test_name: &str, arguments: &[&str], source: &str) -> String {
    let directory = single_file(test_name, source);
    let mut all = vec!["dump-clif"];
    all.extend_from_slice(arguments);
    all.push("main.prism");
    let dump = prism(&all, &directory);
    assert_eq!(dump.status.code(), Some(0), "{}", stderr(&dump));
    fs::remove_dir_all(&directory).ok();
    stdout(&dump)
}

/// The machine signature line of function `symbol` in `dump`.
fn signature_line<'a>(dump: &'a str, symbol: &str) -> &'a str {
    let ir = function_ir(dump, symbol);
    ir.lines().next().unwrap_or_default()
}

/// code-generation: "Signature of a function taking a function value" (D2 of `closures`).
#[test]
fn signature_of_a_function_taking_a_function_value() {
    let dump = clif_of(
        "function_value_signature",
        &[],
        "fun call(f: () -> Int): Int = f()\n",
    );
    let line = signature_line(&dump, "main::call");
    assert!(line.contains("\"main::call\"(i64) -> i64"), "{}", line);
}

/// code-generation: "Building a closure names its descriptor" and "Closure functions in the IR
/// dump" (D3 and D4 of `closures`).
#[test]
fn closure_functions_and_descriptors_in_the_ir_dump() {
    let dump = clif_of(
        "closure_functions",
        &[],
        "fun run(f: () -> Int): Int = f()\nfun main() { val a = 1; println(run { a + 1 }) }\n",
    );
    assert_eq!(
        function_headers(&dump),
        [
            "main::run",
            "main::main",
            "main::main#closure0",
            "main#drop(() -> Int)"
        ]
    );
    let main = function_ir(&dump, "main::main");
    assert!(
        main.contains("symbol %\"main::main#closure0#descriptor\""),
        "{}",
        main
    );
    // The closure takes its environment and returns an `Int`.
    let line = signature_line(&dump, "main::main#closure0");
    assert!(
        line.contains("\"main::main#closure0\"(i64) -> i64"),
        "{}",
        line
    );
    // Descriptors are data, not printed.
    assert!(!dump.contains("function main::main#closure0#descriptor"));
}

/// code-generation: "A call of a function value in the IR dump" (D6 of `closures`): three
/// dependent loads, then an indirect call passing the environment.
#[test]
fn a_call_of_a_function_value_in_the_ir_dump() {
    let dump = clif_of(
        "function_value_call",
        &[],
        "fun call(f: () -> Int): Int = f()\n",
    );
    let ir = function_ir(&dump, "main::call");
    let lines: Vec<&str> = ir.lines().map(str::trim).collect();
    let call = lines
        .iter()
        .position(|line| line.contains("call_indirect"))
        .unwrap_or_else(|| panic!("no indirect call in\n{}", ir));
    // `v1 = load f`, `v2 = load f+8`, `v3 = load v1`, `v4 = load v3`, `call_indirect v4(v2)`.
    let value = |line: &str| line.split(" = ").next().unwrap_or_default().to_string();
    let loads = &lines[call - 4..call];
    assert!(loads.iter().all(|line| line.contains("load.i64")), "{}", ir);
    let (descriptor, environment, slot, code) = (
        value(loads[0]),
        value(loads[1]),
        value(loads[2]),
        value(loads[3]),
    );
    assert!(loads[1].ends_with("+8"), "{}", ir);
    assert!(loads[2].ends_with(&format!(" {}", descriptor)), "{}", ir);
    assert!(loads[3].ends_with(&format!(" {}", slot)), "{}", ir);
    assert!(
        lines[call].contains(&format!("{}({})", code, environment)),
        "{}",
        ir
    );
}

/// code-generation: "A writing closure allocates nothing" (D5 of `closures`).
#[test]
fn a_writing_closure_allocates_nothing() {
    let dump = clif_of(
        "writing_closure",
        &[],
        "fun repeat(n: Int, f: () -> Unit) { var i = 0; while (i < n) { f(); i += 1 } }\n\
         fun main() { var count = 0; repeat(3) { count += 1 }; println(count) }\n",
    );
    assert!(!dump.contains("prism_rt_allocate"), "{}", dump);
}

/// code-generation: "An owning closure allocates once" (D8 of `escaping-closures`).
#[test]
fn an_owning_closure_allocates_once() {
    let dump = clif_of(
        "owning_closure",
        &[],
        "fun make(s: String): () -> Int = { s.length }\n\
         fun main() { val g = make(\"abc\"); println(g() + g()) }\n",
    );
    let make = function_ir(&dump, "main::make");
    assert_eq!(calls_of(make, "prism_rt_allocate").len(), 1, "{}", make);
    let closure = function_ir(&dump, "main::make#closure0");
    assert!(
        calls_of(closure, "prism_rt_allocate").is_empty(),
        "{}",
        closure
    );
}

/// code-generation: "The environment's drop function in the IR dump" (D8 of
/// `escaping-closures`).
#[test]
fn the_environment_s_drop_function_in_the_ir_dump() {
    let dump = clif_of(
        "environment_drop",
        &[],
        "fun make(s: String, n: Int): () -> Int = { s.length + n }\n",
    );
    let headers = function_headers(&dump);
    assert!(
        headers.contains(&"main::make#closure0#drop-environment"),
        "{:?}",
        headers
    );
    let drop = function_ir(&dump, "main::make#closure0#drop-environment");
    assert!(calls_directly(drop, "prism_rt_string_drop"), "{}", drop);
    assert!(calls_directly(drop, "prism_rt_free"), "{}", drop);
    // The descriptor is data: building the closure names it, and its drop entry is the
    // function above, which the run's allocation check exercises.
    let make = function_ir(&dump, "main::make");
    assert!(
        make.contains("symbol %\"main::make#closure0#descriptor\""),
        "{}",
        make
    );
}

/// code-generation: "A handler capturing one handle allocates nothing" (D9 of
/// `escaping-closures`).
#[test]
fn a_handler_capturing_one_handle_allocates_nothing() {
    let dump = clif_of(
        "inline_environment",
        &[],
        "struct Handle : Copy { val id: Int }\n\
         struct Button { val onClick: () -> Int }\n\
         fun button(h: Handle): Button = Button({ h.id * 2 })\n\
         fun main() { val b = button(Handle(21)); println(b.onClick()) }\n",
    );
    assert!(!dump.contains("prism_rt_allocate"), "{}", dump);
    assert!(!dump.contains("drop-environment"), "{}", dump);
}

/// code-generation: "An escaping parameter is represented alike" (D3 of `escaping-closures`).
#[test]
fn an_escaping_parameter_is_represented_alike() {
    let dump = clif_of(
        "escaping_signature",
        &[],
        "fun call(f: escaping () -> Int): Int = f()\n",
    );
    let line = signature_line(&dump, "main::call");
    assert!(line.contains("\"main::call\"(i64) -> i64"), "{}", line);
}

/// code-generation: "Symbols of nested and default closures" (D4 of `closures`).
#[test]
fn symbols_of_nested_and_default_closures() {
    let dump = clif_of(
        "closure_symbols",
        &[],
        "fun run(f: () -> Int): Int = f()\n\
         fun pick(n: Int, f: () -> Int = { 3 }): Int = n + f()\n\
         fun main() { println(run { run { 1 } }); println(pick(1)) }\n",
    );
    let headers = function_headers(&dump);
    for symbol in [
        "main::main#closure0",
        "main::main#closure0#closure0",
        "main::pick#default(f)#closure0",
    ] {
        assert!(headers.contains(&symbol), "{:?}", headers);
    }
}

/// code-generation: "Drop glue of a function type in the IR dump" (D9 of `closures`).
#[test]
fn drop_glue_of_a_function_type_in_the_ir_dump() {
    let dump = clif_of(
        "function_drop_glue",
        &[],
        "struct Task { val run: () -> Int, val name: String }\nfun f(t: Task) {}\n",
    );
    assert_eq!(
        function_headers(&dump),
        ["main::f", "main#drop(() -> Int)", "main#drop(main::Task)"]
    );
    let task = function_ir(&dump, "main#drop(main::Task)");
    assert!(calls_directly(task, "main#drop(() -> Int)"), "{}", task);
    // The function type's glue calls the drop entry only when it is not null.
    let glue = function_ir(&dump, "main#drop(() -> Int)");
    assert!(
        glue.contains("brif") && glue.contains("call_indirect"),
        "{}",
        glue
    );
}

/// code-generation: "A closure in an instance" (D11 of `closures`).
#[test]
fn a_closure_in_an_instance() {
    let dump = clif_of(
        "instance_closure",
        &["--instances"],
        "fun run(g: () -> Int): Int = g()\n\
         fun <T : Copy> count(x: T, f: (T) -> Int): Int = run { f(x) + 1 }\n\
         fun main() { println(count(2) { n -> n * 10 }) }\n",
    );
    let headers = function_headers(&dump);
    let instance = headers.iter().position(|h| *h == "main::count<Int>");
    let closure = headers
        .iter()
        .position(|h| *h == "main::count<Int>#closure0");
    assert!(instance.is_some() && closure > instance, "{:?}", headers);
}

// --- `prism run --watch` (the `hot-reload` spec)

/// execution: "Edits without watching".
#[test]
fn edits_without_watching() {
    let directory = single_file("edits_without_watching", "fun main() { println(1) }\n");
    write(
        &directory.join("edits"),
        "after exit: edit main.prism main.prism\n",
    );
    let output = prism(&["run", "--edits", "edits", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "", "nothing runs");
    assert!(
        stderr(&output).contains("usage: prism run"),
        "{}",
        stderr(&output)
    );
}

/// hot-reload: "Timing a reload". The times differ from run to run, so the lines are checked by
/// their shape.
#[test]
fn timing_a_reload() {
    let directory = single_file(
        "timing_a_reload",
        "fun step(i: Int): Int = i\nfun main() { println(step(1)); println(step(2)) }\n",
    );
    write(
        &directory.join("main.prism.v2"),
        "fun step(i: Int): Int = i * 10\nfun main() { println(step(1)); println(step(2)) }\n",
    );
    write(
        &directory.join("edits"),
        "after line 1: edit main.prism main.prism.v2\n",
    );
    let output = prism(
        &[
            "run",
            "--watch",
            "--timings",
            "--edits",
            "edits",
            "main.prism",
        ],
        &directory,
    );
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1\n20\n");
    let stderr = stderr(&output);
    let reloads: Vec<&str> = stderr
        .lines()
        .filter(|line| line.starts_with("timing: reload "))
        .collect();
    assert_eq!(reloads.len(), 1, "{}", stderr);
    let stages = [
        "detect",
        "front end",
        "code generation",
        "linking",
        "applied after",
    ];
    let rest = reloads[0].strip_prefix("timing: reload ").unwrap();
    let (total, stages_text) = rest.split_once(" ms (").expect("a total and its stages");
    assert!(total.parse::<f64>().unwrap() >= 0.0);
    let stages_text = stages_text.strip_suffix(')').expect("closed stages");
    let parsed: Vec<(&str, f64)> = stages_text
        .split(", ")
        .map(|stage| {
            let (name, value) = stage
                .strip_suffix(" ms")
                .and_then(|stage| stage.rsplit_once(' '))
                .unwrap_or_else(|| panic!("unexpected stage {:?}", stage));
            (name, value.parse::<f64>().unwrap())
        })
        .collect();
    let names: Vec<&str> = parsed.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, stages, "{}", reloads[0]);
    assert!(parsed.iter().all(|(_, value)| *value >= 0.0));
    assert!(
        stderr.lines().any(|line| line == "timing: live modules 2"),
        "{}",
        stderr
    );
}

/// A `prism run --watch` session on a workspace, with its stderr read line by line.
struct WatchedProgram {
    child: std::process::Child,
    lines: std::sync::mpsc::Receiver<String>,
    seen: Vec<String>,
}

impl WatchedProgram {
    fn start(directory: &Path, entry: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_prism"))
            .env(
                "PRISM_CACHE",
                std::env::temp_dir().join("prism_run_cli_build_cache"),
            )
            .args(["run", "--watch", entry])
            .current_dir(directory)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("failed to run prism");
        let (sender, lines) = std::sync::mpsc::channel();
        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stderr).lines() {
                let Ok(line) = line else { return };
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            lines,
            seen: Vec::new(),
        }
    }

    /// Waits up to 20 seconds for a stderr line satisfying `wanted`.
    fn wait_for(&mut self, wanted: impl Fn(&str) -> bool) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    self.seen.push(line.clone());
                    if wanted(&line) {
                        return line;
                    }
                }
                Err(_) => panic!("timed out; stderr so far: {:#?}", self.seen),
            }
        }
    }

    /// Collects the stderr lines written within `duration`.
    fn drain_for(&mut self, duration: std::time::Duration) {
        let deadline = std::time::Instant::now() + duration;
        while let Ok(line) = self
            .lines
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        {
            self.seen.push(line);
        }
    }

    /// Ends the session and gives its stdout.
    fn stop(mut self) -> (String, Vec<String>) {
        let _ = self.child.kill();
        let output = self.child.wait_with_output().unwrap();
        (stdout(&output), self.seen)
    }
}

/// A program whose `main` loops calling `value` until it returns something else than 1.
const LOOPING: &str =
    "fun value(): Int = 1\nfun main() { var n = 0; while (value() == 1) { n += 1 }; println(value()) }\n";

/// hot-reload: "Saving an edited body while the program runs".
#[test]
fn saving_an_edited_body_while_the_program_runs() {
    let directory = single_file("watch_saving", LOOPING);
    let mut session = WatchedProgram::start(&directory, "main.prism");
    // The program runs once its first load is done; give the watcher time to register too.
    std::thread::sleep(std::time::Duration::from_millis(300));
    write(
        &directory.join("main.prism"),
        &LOOPING.replace("Int = 1\n", "Int = 2\n"),
    );
    session.wait_for(|line| line == "reload: applied main.prism");
    session.wait_for(|line| line == "watch: main returned with status 0; waiting for changes");
    let (stdout, _) = session.stop();
    assert_eq!(stdout, "2\n");
}

/// hot-reload: "Saving without changes does nothing": writing the same content reloads
/// nothing, and a real edit afterwards reloads once.
#[test]
fn saving_without_changes_does_nothing() {
    let directory = single_file("watch_unchanged", LOOPING);
    let mut session = WatchedProgram::start(&directory, "main.prism");
    std::thread::sleep(std::time::Duration::from_millis(300));
    write(&directory.join("main.prism"), LOOPING);
    session.drain_for(std::time::Duration::from_millis(500));
    write(
        &directory.join("main.prism"),
        &LOOPING.replace("Int = 1\n", "Int = 2\n"),
    );
    session.wait_for(|line| line == "watch: main returned with status 0; waiting for changes");
    let (_, seen) = session.stop();
    let reloads = seen
        .iter()
        .filter(|line| line.starts_with("reload:"))
        .count();
    assert_eq!(reloads, 1, "{:#?}", seen);
}

/// hot-reload: "An atomic save is seen": an editor writes a temporary file and renames it over
/// the original.
#[test]
fn an_atomic_save_is_seen() {
    let directory = single_file("watch_atomic", LOOPING);
    let mut session = WatchedProgram::start(&directory, "main.prism");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let temporary = directory.join("main.prism.tmp");
    write(&temporary, &LOOPING.replace("Int = 1\n", "Int = 2\n"));
    fs::rename(&temporary, directory.join("main.prism")).unwrap();
    session.wait_for(|line| line == "watch: main returned with status 0; waiting for changes");
    let (stdout, seen) = session.stop();
    assert_eq!(stdout, "2\n");
    let reloads = seen
        .iter()
        .filter(|line| line.starts_with("reload: applied"))
        .count();
    assert_eq!(reloads, 1, "{:#?}", seen);
}

/// hot-reload: "A syntax error in an imported file at start": the session watches the file in
/// error although no build succeeded, so saving it fixed runs the program.
#[test]
fn a_syntax_error_in_an_imported_file_at_start() {
    let directory = two_files(
        "watch_syntax_error_at_start",
        "public fun twice(n: Int): Int = n *\n",
        "fun main() { println(twice(2)) }\n",
    );
    let mut session = WatchedProgram::start(&directory, "app/main.prism");
    session.wait_for(|line| line == "watch: waiting for changes");
    std::thread::sleep(std::time::Duration::from_millis(300));
    write(
        &directory.join("lib/math.prism"),
        "package lib\npublic fun twice(n: Int): Int = n * 2\n",
    );
    session.wait_for(|line| line == "watch: main returned with status 0; waiting for changes");
    let (stdout, _) = session.stop();
    assert_eq!(stdout, "4\n");
}

/// The IR `prism dump-clif` prints for the function `symbol` of the file in `directory`.
fn clif_function(directory: &Path, symbol: &str) -> String {
    let output = prism(&["dump-clif", "main.prism"], directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = stdout(&output);
    let header = format!("function {}\n", symbol);
    let start = dump
        .find(&header)
        .unwrap_or_else(|| panic!("no function {} in\n{}", symbol, dump));
    let rest = &dump[start + header.len()..];
    let end = rest.find("\nfunction ").unwrap_or(rest.len());
    rest[..end].to_string()
}

/// code-generation: "Signature of a composable" and "A call of a composable function value"
/// (`compose-transform`): a composable takes the composer and a change word after its
/// parameters, and a call of a composable function value passes them after the environment.
#[test]
fn composable_signatures_and_calls() {
    let directory = single_file(
        "composable_signatures",
        "compose fun Label(n: Int) {}\ncompose fun Column(content: compose () -> Unit) { content() }\n",
    );
    let label = clif_function(&directory, "main::Label");
    assert!(
        label.contains("function %\"main::Label\"(i64, i64, i64)"),
        "{}",
        label
    );
    let column = clif_function(&directory, "main::Column");
    // The content's environment, then the composer and the change word.
    assert!(
        column.contains("call_indirect sig") && column.contains("(i64, i64, i64) "),
        "{}",
        column
    );
}

/// code-generation: "A restart group in the IR dump" and "Records of a unit"
/// (`compose-transform`).
#[test]
fn restart_groups_and_slot_type_records_in_the_ir_dump() {
    let directory = single_file(
        "restart_group_ir",
        "struct Point : Copy { val x: Int }\n\
         compose fun Badge(n: Int) { println(n) }\n\
         compose fun P(n: Int) { val p = remember { Point(n) } }\n",
    );
    let badge = clif_function(&directory, "main::Badge");
    let start = badge
        .find("call fn2(")
        .or_else(|| badge.find("prism_compose_start_restart_group"))
        .expect("the start of the restart group");
    assert!(
        badge.contains("prism_compose_start_restart_group"),
        "{}",
        badge
    );
    assert!(
        badge.contains("prism_compose_end_restart_group"),
        "{}",
        badge
    );
    assert!(start > 0);
    assert!(
        badge.contains("symbol %\"main#slot-type(Int)\""),
        "{}",
        badge
    );
    let p = clif_function(&directory, "main::P");
    assert!(
        p.contains("symbol %\"main#slot-type(main::Point)\""),
        "{}",
        p
    );
}

/// code-generation: "Entries of closures" (`compose-transform`): a closure owning only `Copy`
/// captures in an allocated environment has an environment equality and duplicate function,
/// and one owning a `String` has neither.
#[test]
fn entries_of_closures() {
    let directory = single_file(
        "closure_entries",
        "fun keep(f: escaping () -> Unit) {}\n\
         fun main() { val a = 1; val b = 2; keep { println(a + b) }; val s = \"x\" + \"y\"; keep { println(s) } }\n",
    );
    let output = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = stdout(&output);
    assert!(dump.contains("function main::main#closure0#equals-environment\n"));
    assert!(dump.contains("function main::main#closure0#duplicate-environment\n"));
    assert!(!dump.contains("main::main#closure1#equals-environment"));
    assert!(!dump.contains("main::main#closure1#duplicate-environment"));
}
