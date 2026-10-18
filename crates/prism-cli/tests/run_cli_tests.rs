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

fn prism(arguments: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_prism"))
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
    let directory = single_file("dump_rejected", "fun f(h: Heap<Int>): Int = 1\n");
    let output = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).starts_with(
            "error[E6001]: the Tier 1 backend does not support 'Heap' values yet\n  --> main.prism:1:1\n"
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
    let output = prism(&["run", "examples/run/main.prism"], &root);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "2 3 5 7 11 13 17 19 23 29 31 37 41 43 47 \n15\n12\n5050\n735\n0.3333333333333333\n-3\ntrue\nfalse\n-2\n0\n"
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
