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
    let directory = single_file(
        "arguments",
        "import /prism/core\nfun main() { println(1) }\n",
    );
    let output = prism(&["run", "main.prism", "--", "--timings", "x"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "1\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn reporting_timings() {
    let directory = single_file("timings", "import /prism/core\nfun main() { println(1) }\n");
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
    let directory = single_file(
        "cache_options",
        "import /prism/core\nfun main() { println(1) }\n",
    );
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
    let directory = single_file("usage", "import /prism/core\nfun main() { println(1) }\n");
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
        "import /prism/core\nval LIMIT: Int = 10\nfun twice(n: Int): Int = n * 2\nfun main() { println(twice(LIMIT)) }\n",
    );
    let first = prism(&["dump-clif", "main.prism"], &directory);
    let second = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(first.status.code(), Some(0), "{}", stderr(&first));
    assert_eq!(second.status.code(), Some(0));
    assert!(!stdout(&first).is_empty());
    assert_eq!(stdout(&first), stdout(&second));
    assert_eq!(stderr(&first), "");
}

/// code-generation: "A read in the IR dump" (`state-handles`, `owned-state`): a read of a state
/// loads its word through the borrow of the state object, calls its entry with the word and the
/// record of `Int`, tests the result for null, and on null calls `prism_compose_state_error`
/// with the location of `s.value` from a cold block.
#[test]
fn a_read_in_the_ir_dump() {
    let directory = single_file(
        "state_read",
        "import /prism/compose/runtime\nfun f(s: &State<Int>): Int = s.value\n",
    );
    let output = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = stdout(&output);
    let ir = function_ir(&dump, "main::f");
    assert!(ir.contains("symbol %\"main#slot-type(Int)\""), "{}", ir);
    let read = ir
        .lines()
        .find(|line| line.contains("= %\"prism_compose_state_read\""))
        .and_then(|line| line.trim().split(' ').next())
        .unwrap_or_else(|| panic!("no state read in\n{}", ir));
    assert!(ir.contains(&format!("= call {}(", read)), "{}", ir);
    assert!(ir.contains("icmp eq"), "the result is tested: {}", ir);
    let error = ir
        .lines()
        .find(|line| line.contains("= %\"prism_compose_state_error\""))
        .and_then(|line| line.trim().split(' ').next())
        .unwrap_or_else(|| panic!("no state error in\n{}", ir));
    let cold = ir
        .split("cold:")
        .skip(1)
        .find(|block| block.contains(&format!("call {}(", error)))
        .unwrap_or_else(|| panic!("no cold block calling the error in\n{}", ir));
    // The location of `s.value`: line 2, column 30.
    assert!(
        cold.contains("iconst.i32 2\n") && cold.contains("iconst.i32 30\n"),
        "{}",
        cold
    );
}

/// code-generation: "Retiring the states of a list of items" (`owned-state`, D7): a state's
/// drop glue retires it, and the glue of a struct and of a list of them reach it.
#[test]
fn retiring_the_states_of_a_list_of_items() {
    let dump = clif_of(
        "retiring_list",
        &[],
        "import /prism/compose/runtime\nstruct Todo { val title: String, val done: MutableState<Bool> }\nfun f(todos: List<Todo>) { }\n",
    );
    let state_glue = "main#drop(prism/compose/runtime::MutableState<Bool>)";
    assert!(
        calls_directly(function_ir(&dump, "main::f"), "main#drop(List<main::Todo>)"),
        "{}",
        dump
    );
    assert!(
        calls_directly(
            function_ir(&dump, "main#drop(List<main::Todo>)"),
            "main#drop(main::Todo)"
        ),
        "{}",
        dump
    );
    assert!(
        calls_directly(function_ir(&dump, "main#drop(main::Todo)"), state_glue),
        "{}",
        dump
    );
    let glue = function_ir(&dump, state_glue);
    assert!(
        calls_directly(glue, "prism_compose_state_retire"),
        "{}",
        glue
    );
    // The state's word, loaded from the state object's address.
    assert!(glue.contains("= load.i64 notrap aligned v0\n"), "{}", glue);
}

/// code-generation: "A handler with one weak capture allocates nothing" (`owned-state`, D10):
/// the weak handle is the environment word, and the closure has environment equality.
#[test]
fn a_handler_with_one_weak_capture_allocates_nothing() {
    let dump = clif_of(
        "weak_inline_environment",
        &[],
        "import /prism/compose/runtime\nfun keep(f: attached () -> Unit) {}\nfun f(count: &MutableState<Int>) { keep { count.value += 1 } }\n",
    );
    assert!(!dump.contains("prism_rt_allocate"), "{}", dump);
    assert!(!dump.contains("drop-environment"), "{}", dump);
    assert!(
        function_headers(&dump).contains(&"main::f#closure0#equals-environment"),
        "{}",
        dump
    );
}

/// code-generation: "Records of a unit's cells" (`state-handles`): the unit has one slot type
/// record for each cell type, and the equality and drop glue they name.
#[test]
fn records_of_a_unit_s_cells() {
    let directory = single_file(
        "state_records",
        "import /prism/collections\nimport /prism/compose/runtime\nfun main() { val s = mutableStateOf(\"a\"); val l = mutableStateOf(listOf(1)); s.value = \"b\"; l.value = listOf(2) }\n",
    );
    let output = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = stdout(&output);
    let main = function_ir(&dump, "main::main");
    for record in ["main#slot-type(String)", "main#slot-type(List<Int>)"] {
        assert_eq!(
            main.matches(&format!("symbol %\"{}\"", record)).count(),
            1,
            "{}",
            main
        );
    }
    for glue in [
        "main#equals(String)",
        "main#equals(List<Int>)",
        "main#drop(List<Int>)",
    ] {
        assert!(
            dump.contains(&format!("function {}\n", glue)),
            "{} in\n{}",
            glue,
            dump
        );
    }
}

/// standard-library: "State declarations are imported" (`state-handles`, `explicit-library`):
/// the ABI writes the handle type of an exported signature by name.
#[test]
fn state_declarations_are_imported() {
    let directory = single_file(
        "state_abi",
        "import /prism/compose/runtime\ncompose fun Counter() { val count = remember { mutableStateOf(0) }; SideEffect { } }\npublic fun counter(): MutableState<Int> = mutableStateOf(0)\n",
    );
    let output = prism(&["abi", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("  public fun counter(): MutableState<Int>\n"),
        "{}",
        stdout(&output)
    );
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
        "import /prism/core\nfun helper(): Int = 1\nfun main() { println(twice(helper())) }\n",
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
        "import /prism/core\nfun main() { println(twice(2)) }\n",
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
        "import /prism/core\nfun main() { println(twice(2)) }\n",
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
        "package app\nimport /prism/core\nimport /lib/text\nfun main() { println(shout(1)) }\n",
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
        "2 3 5 7 11 13 17 19 23 29 31 37 41 43 47 \n15\n12\n5050\n735\n0.3333333333333333\n-3\ntrue\nfalse\n-2\n0\nquadrant 4\n-24\n15 primes up to 50\n15 primes up to 50, 18 characters\n20\n10\ntrue\nprimes: 15\n5\n-1\n60\n18\n-20\n134\nok\nlimit = 50\n5 removed, 4 left, total 154, true\ntrue\n(6, -3)\n202, 20, 34, 52\nrow 1: 1 (first 1)\nrow 2: 2 (first 2)\nrow 1: 2 (first 1)\nrow 2: 4 (first 2)\nfalse\ncounter 1\ncount 0\ncount 2\ntrue\n"
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
        "struct main::Point : Copy (size 16, align 8)\n  x: Double at 0\n  y: Double at 8\n\
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
        "import /prism/core\nfun main() { val s = \"a\" + \"b\"; println(s) }\n",
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
        "import /prism/core\nfun zero(): Int = 0\nfun main() { val s = \"a\" + \"b\"; println(1 / zero()) }\n",
    );
    let output = prism(&["run", "--check-allocations", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "panic: division by zero\n  at main.prism:3:41\n"
    );
    fs::remove_dir_all(&directory).ok();
}

/// `--check-allocations` combines with `--timings` and with the program's arguments.
#[test]
fn the_allocation_check_combines_with_the_other_options() {
    let directory = single_file(
        "check_and_timings",
        "import /prism/core\nfun main() { println(1.toString()) }\n",
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
        "package app\nimport /prism/core\nimport /lib/tree\nfun main() { val t = Tree.Node(Heap(leaf()), 1, Heap(leaf())); println(t == t) }\n",
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
        "import /prism/core\nfun main() { println(keep(1)) }\n",
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
        "import /prism/core\nimport /app/other\nfun main() { println(keep(1)); println(other()) }\n",
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
        "import /prism/core\nfun <T> keep(x: T): T = x\nfun main() { println(keep(1)); println(keep(\"a\")) }\n",
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
        "import /prism/core\nfun main() { println(keep(1)) }\n",
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
        "import /prism/core\nfun main() { println(keep(1)); println(keep(true)) }\n",
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
        "import /prism/core\nimport /lib/user\n\
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
        "import /prism/core\nstruct User : Clone { val name: String }\n\
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
        "import /prism/core\nfun main() { println(pick(1, 2)) }\n",
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
        "import /prism/core\nfun main() { println(outer(\"a\", 1)) }\n",
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
        "import /prism/core\nfun run(f: () -> Int): Int = f()\nfun main() { val a = 1; println(run { a + 1 }) }\n",
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
        "import /prism/core\nfun repeat(n: Int, f: () -> Unit) { var i = 0; while (i < n) { f(); i += 1 } }\n\
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
        "import /prism/core\nfun make(s: String): () -> Int = { s.length }\n\
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
        "import /prism/core\nstruct Handle : Copy { val id: Int }\n\
         struct Button { val onClick: () -> Int }\n\
         fun button(h: Handle): Button = Button({ h.id * 2 })\n\
         fun main() { val b = button(Handle(21)); println(b.onClick()) }\n",
    );
    assert!(!dump.contains("prism_rt_allocate"), "{}", dump);
    assert!(!dump.contains("drop-environment"), "{}", dump);
}

/// code-generation: "A handler copying a field through a borrow allocates nothing"
/// (`path-captures`, D8): the handler owns a copy of `t.id`, held in its environment word.
#[test]
fn a_handler_copying_a_field_through_a_borrow_allocates_nothing() {
    let dump = clif_of(
        "inline_path_environment",
        &[],
        "import /prism/core\nstruct Todo { val id: Int, val count: Int }\n\
         struct Button { val onClick: () -> Int }\n\
         fun button(t: &Todo): Button = Button({ t.id * 2 })\n\
         fun main() { val t = Todo(21, 3); val b = button(t); println(b.onClick()) }\n",
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
        "import /prism/core\nfun run(f: () -> Int): Int = f()\n\
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
        "import /prism/core\nfun run(g: () -> Int): Int = g()\n\
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
    let directory = single_file(
        "edits_without_watching",
        "import /prism/core\nfun main() { println(1) }\n",
    );
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
        "import /prism/core\nfun step(i: Int): Int = i\nfun main() { println(step(1)); println(step(2)) }\n",
    );
    write(
        &directory.join("main.prism.v2"),
        "import /prism/core\nfun step(i: Int): Int = i * 10\nfun main() { println(step(1)); println(step(2)) }\n",
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

/// A `prism run --watch` session on a workspace, with its stderr read line by line. Dropping it
/// kills and reaps the process, so a test that fails or panics leaves no session running: most
/// of these programs loop until they are edited.
struct WatchedProgram {
    child: std::process::Child,
    lines: std::sync::mpsc::Receiver<String>,
    seen: Vec<String>,
    /// The program's stdout line by line, when the session was started to read it.
    output: Option<std::sync::mpsc::Receiver<String>>,
    printed: Vec<String>,
}

/// Sends each line `pipe` gives to a new channel, read by a thread of its own.
fn line_channel(pipe: impl std::io::Read + Send + 'static) -> std::sync::mpsc::Receiver<String> {
    let (sender, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(pipe).lines() {
            let Ok(line) = line else { return };
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    lines
}

impl WatchedProgram {
    fn start(directory: &Path, entry: &str) -> Self {
        Self::spawn(directory, &["run", "--watch", entry])
    }

    /// Starts `prism run --watch --timings` on a program that compiles, and waits until its
    /// first build has read its files and `main` is starting: the session prints its load
    /// timings then. A save from then on is seen whatever the machine's load, by its event or,
    /// when the save came before the watcher registered its directory, by the check the
    /// session makes once it has, so no fixed sleep is needed.
    fn start_running(directory: &Path, entry: &str) -> Self {
        let mut session = Self::spawn(directory, &["run", "--watch", "--timings", entry]);
        session.wait_for(|line| line.starts_with("timing: total to main"));
        session
    }

    fn spawn(directory: &Path, arguments: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_prism"))
            .env(
                "PRISM_CACHE",
                std::env::temp_dir().join("prism_run_cli_build_cache"),
            )
            .args(arguments)
            .current_dir(directory)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("failed to run prism");
        let lines = line_channel(child.stderr.take().unwrap());
        Self {
            child,
            lines,
            seen: Vec::new(),
            output: None,
            printed: Vec::new(),
        }
    }

    /// [`WatchedProgram::spawn`], reading the program's stdout line by line as it is written.
    fn spawn_reading_output(directory: &Path, arguments: &[&str]) -> Self {
        let mut session = Self::spawn(directory, arguments);
        session.output = Some(line_channel(session.child.stdout.take().unwrap()));
        session
    }

    /// Waits up to `limit` for a stdout line satisfying `wanted`, of a session started by
    /// [`WatchedProgram::spawn_reading_output`].
    fn wait_for_output(
        &mut self,
        limit: std::time::Duration,
        wanted: impl Fn(&str) -> bool,
    ) -> String {
        let deadline = std::time::Instant::now() + limit;
        let output = self.output.as_ref().expect("the session's stdout is read");
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match output.recv_timeout(left) {
                Ok(line) => {
                    self.printed.push(line.clone());
                    if wanted(&line) {
                        return line;
                    }
                }
                Err(_) => panic!(
                    "timed out; stdout so far: {:#?}\nstderr so far: {:#?}",
                    self.printed, self.seen
                ),
            }
        }
    }

    /// Waits up to 60 seconds, generous for a loaded machine, for a stderr line satisfying
    /// `wanted`.
    fn wait_for(&mut self, wanted: impl Fn(&str) -> bool) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
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

    /// Waits up to 60 seconds for the session to end by itself, and gives its exit status.
    fn wait_for_exit(&mut self) -> Option<i32> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while std::time::Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code();
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the session did not end; stderr so far: {:#?}", self.seen);
    }

    /// Ends the session and gives its stdout and the stderr lines read.
    fn stop(mut self) -> (String, Vec<String>) {
        use std::io::Read;
        let _ = self.child.kill();
        let mut stdout = String::new();
        if let Some(mut pipe) = self.child.stdout.take() {
            let _ = pipe.read_to_string(&mut stdout);
        }
        let _ = self.child.wait();
        (stdout, std::mem::take(&mut self.seen))
    }
}

impl Drop for WatchedProgram {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The `prism` processes running a watch session in `directory`, by their command line and
/// working directory.
fn watch_sessions_in(directory: &Path) -> Vec<u32> {
    let output = Command::new("pgrep")
        .args(["-f", "prism run --watch"])
        .output()
        .expect("pgrep runs");
    let directory = directory.canonicalize().unwrap_or(directory.to_path_buf());
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .filter(|pid| {
            let cwd = Command::new("lsof")
                .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
                .output()
                .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
                .unwrap_or_default();
            cwd.lines()
                .any(|line| line.strip_prefix('n') == Some(&*directory.to_string_lossy()))
        })
        .collect()
}

/// A program whose `main` loops calling `value` until it returns something else than 1.
const LOOPING: &str =
    "import /prism/core\nfun value(): Int = 1\nfun main() { var n = 0; while (value() == 1) { n += 1 }; println(value()) }\n";

/// hot-reload: "Saving an edited body while the program runs".
#[test]
fn saving_an_edited_body_while_the_program_runs() {
    let directory = single_file("watch_saving", LOOPING);
    let mut session = WatchedProgram::start_running(&directory, "main.prism");
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
    let mut session = WatchedProgram::start_running(&directory, "main.prism");
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
    let mut session = WatchedProgram::start_running(&directory, "main.prism");
    let temporary = directory.join("main.prism.tmp");
    write(&temporary, &LOOPING.replace("Int = 1\n", "Int = 2\n"));
    fs::rename(&temporary, directory.join("main.prism")).unwrap();
    session.wait_for(|line| line == "reload: applied main.prism");
    session.wait_for(|line| line == "watch: main returned with status 0; waiting for changes");
    let (stdout, seen) = session.stop();
    assert_eq!(stdout, "2\n");
    let reloads = seen
        .iter()
        .filter(|line| line.starts_with("reload: applied"))
        .count();
    assert_eq!(reloads, 1, "{:#?}", seen);
}

/// hot-reload: "Removing the workspace root ends the session": a session whose program loops
/// until it is edited ends, rather than run forever unseen.
#[test]
fn removing_the_workspace_root_ends_the_session() {
    let directory = single_file("watch_root_removed", LOOPING);
    let mut session = WatchedProgram::start_running(&directory, "main.prism");
    let root = directory.canonicalize().unwrap();
    fs::remove_dir_all(&directory).unwrap();
    session.wait_for(|line| {
        line == format!(
            "watch: the workspace root {} was removed; the session ends",
            root.display()
        )
    });
    assert_eq!(session.wait_for_exit(), Some(1));
    let (stdout, seen) = session.stop();
    assert_eq!(stdout, "");
    assert!(
        !seen.iter().any(|line| line.starts_with("reload:")),
        "{:#?}",
        seen
    );
}

/// A test that fails while its watch session runs leaves no process behind: dropping the
/// session, as unwinding does, kills and reaps it.
#[test]
fn a_failing_watch_test_leaves_no_session_running() {
    let directory = single_file("watch_failing_test", LOOPING);
    let result = std::panic::catch_unwind(|| {
        let _session = WatchedProgram::start_running(&directory, "main.prism");
        panic!("an induced failure");
    });
    assert!(result.is_err());
    assert_eq!(watch_sessions_in(&directory), Vec::<u32>::new());
}

/// hot-reload: "A syntax error in an imported file at start": the session watches the file in
/// error although no build succeeded, so saving it fixed runs the program.
#[test]
fn a_syntax_error_in_an_imported_file_at_start() {
    let directory = two_files(
        "watch_syntax_error_at_start",
        "public fun twice(n: Int): Int = n *\n",
        "import /prism/core\nfun main() { println(twice(2)) }\n",
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

/// code-generation: "Signature of a composable taking a borrow" (`strong-skipping`): one version
/// word after the composer and the change word.
#[test]
fn signature_of_a_composable_taking_a_borrow() {
    let directory = single_file(
        "borrowed_composable_signature",
        "compose fun Show(s: &String, n: Int) {}\n",
    );
    let show = clif_function(&directory, "main::Show");
    assert!(
        show.contains("function %\"main::Show\"(i64, i64, i64, i64, i64)"),
        "{}",
        show
    );
}

/// code-generation: "A version read in the IR dump" (`strong-skipping`): the version of the
/// state whose contents `Show` borrows is read with the record of `List<String>` and passed
/// as the call's last argument.
#[test]
fn a_version_read_in_the_ir_dump() {
    let directory = single_file(
        "version_read_ir",
        "import /prism/compose/runtime\ncompose fun Show(s: &String) {}\n\
         compose fun Screen(items: &State<List<String>>) { Show(items.value[0]) }\n",
    );
    let screen = clif_function(&directory, "main::Screen");
    let entry = screen
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let (name, symbol) = line.split_once(" = %\"")?;
            (symbol.starts_with("prism_compose_state_version\"")).then(|| name.to_string())
        })
        .unwrap_or_else(|| panic!("no version entry in\n{}", screen));
    let record = screen
        .lines()
        .find_map(|line| {
            let (name, _) = line
                .trim()
                .split_once(" = symbol %\"main#slot-type(List<String>)\"")?;
            Some(name.to_string())
        })
        .expect("the record of List<String>");
    let call = screen
        .lines()
        .find(|line| line.contains(&format!("call {}(", entry)))
        .unwrap_or_else(|| panic!("no call of {} in\n{}", entry, screen));
    let (result, arguments) = call.trim().split_once(" = ").expect("a result");
    // The handle, then a value loaded from the record's symbol.
    assert!(arguments.ends_with(")"), "{}", call);
    let record_value = screen
        .lines()
        .filter_map(|line| {
            line.trim()
                .split_once(&format!(" = symbol_value.i64 {}", record))
        })
        .map(|(name, _)| name.to_string())
        .collect::<Vec<_>>();
    assert!(
        record_value
            .iter()
            .any(|value| arguments.contains(&format!(", {})", value))),
        "{} with {:?}",
        call,
        record_value
    );
    // The result, or an alias of it, is the last argument of the call of `Show`'s slot.
    let aliases: Vec<String> = screen
        .lines()
        .filter_map(|line| {
            let (alias, of) = line.trim().split_once(" -> ")?;
            (of == result).then(|| alias.to_string())
        })
        .chain([result.to_string()])
        .collect();
    let show_call = screen
        .lines()
        .find(|line| line.contains("call_indirect"))
        .unwrap_or_else(|| panic!("no call of Show in\n{}", screen));
    let arguments = show_call
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(arguments, _)| arguments)
        .expect("arguments");
    let last = arguments.rsplit(", ").next().unwrap();
    assert!(aliases.iter().any(|alias| alias == last), "{}", show_call);
}

/// code-generation: "The record of a kept borrow" (`strong-skipping`): `&String` has a record
/// whose equality entry is the identity glue, which compares two words and calls no glue of
/// `String`, and no drop glue.
#[test]
fn the_record_of_a_kept_borrow() {
    let directory = single_file("kept_borrow_record", "compose fun Show(s: &String) {}\n");
    let output = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = stdout(&output);
    let show = function_ir(&dump, "main::Show");
    assert!(
        show.contains("symbol %\"main#slot-type(&String)\""),
        "{}",
        show
    );
    let identity = function_ir(&dump, "main#identity(&String)");
    // Each value's two words, at its address and 8 bytes after, and nothing else but the
    // stack check every glue starts with.
    for word in [
        "aligned v0\n",
        "aligned v0+8\n",
        "aligned v1\n",
        "aligned v1+8\n",
    ] {
        assert!(identity.contains(word), "{} in\n{}", word, identity);
    }
    assert!(!identity.contains("equals"), "{}", identity);
    assert!(!dump.contains("main#drop(&String)"), "{}", dump);
    assert!(!dump.contains("prism_rt_string_equals"), "{}", dump);
}

/// code-generation: "A restart group in the IR dump" and "Records of a unit"
/// (`compose-transform`).
#[test]
fn restart_groups_and_slot_type_records_in_the_ir_dump() {
    let directory = single_file(
        "restart_group_ir",
        "import /prism/core\nimport /prism/compose/runtime\nstruct Point : Copy { val x: Int }\n\
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
        "import /prism/core\nfun keep(f: escaping () -> Unit) {}\n\
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

/// execution: "Events without headless mode": `--events` needs `--headless`, and nothing runs.
#[test]
fn events_without_headless_mode() {
    let directory = single_file(
        "events_without_headless",
        "import /prism/core\nfun main() { println(\"ran\") }\n",
    );
    write(&directory.join("script"), "dump\n");
    let output = prism(&["run", "--events", "script", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("usage: prism run") && stderr(&output).contains("--headless"),
        "{}",
        stderr(&output)
    );
    assert_eq!(stdout(&output), "");
    let output = prism(&["run", "--headless=wide", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(1), "a size is two numbers");
    fs::remove_dir_all(&directory).ok();
}

/// The counter screen of `ui-toolkit`, with `label` before its count.
fn counter_app(label: &str) -> String {
    format!(
        "import /prism/compose/runtime\nimport /prism/compose/ui\nimport /prism/compose/foundation\nimport /prism/compose/layout\nimport /prism/compose/material3\n\
         compose fun Counter() {{\n    val c = remember {{ mutableStateOf(0) }}\n    \
         Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {{\n        \
         Text(\"{}${{c.value}}\")\n        Button(onClick = {{ c.value += 1 }}) {{ Text(\"+\") }}\n    }}\n}}\n\
         fun main() {{ runApp(width = 400, height = 300) {{ Counter() }} }}\n",
        label
    )
}

/// compose-hot-reload, ui: "Saving a file under a waiting headless app". The app dumps its
/// tree, which `wait reload` flushes as it starts waiting, so the save comes once the app waits.
#[test]
fn saving_a_file_under_a_waiting_headless_app() {
    let directory = single_file("watch_headless_save", &counter_app("Count: "));
    write(&directory.join("events"), "dump\nwait reload\ndump\n");
    let mut session = WatchedProgram::spawn_reading_output(
        &directory,
        &[
            "run",
            "--watch",
            "--headless",
            "--events",
            "events",
            "main.prism",
        ],
    );
    let limit = std::time::Duration::from_secs(20);
    session.wait_for_output(limit, |line| line.contains("\"Count: 0\""));
    write(&directory.join("main.prism"), &counter_app("Clicks: "));
    let text = session.wait_for_output(limit, |line| line.contains("Clicks: "));
    assert!(text.contains("\"Clicks: 0\""), "{}", text);
    session.wait_for(|line| line == "reload: applied main.prism");
    session.stop();
    fs::remove_dir_all(&directory).ok();
}

/// compose-hot-reload, ui: "Timing a UI reload". The times differ from run to run, so the lines
/// are checked by their shape: a `timing: reload` line, then the reload pass with its scopes.
#[test]
fn timing_a_ui_reload() {
    let directory = single_file("watch_ui_timing", &counter_app("Count: "));
    write(&directory.join("main.prism.v2"), &counter_app("Clicks: "));
    write(
        &directory.join("events"),
        "click \"+\"\nedit main.prism main.prism.v2\ndump\n",
    );
    let mut session = WatchedProgram::spawn(
        &directory,
        &[
            "run",
            "--watch",
            "--timings",
            "--headless",
            "--events",
            "events",
            "main.prism",
        ],
    );
    session.wait_for(|line| line == "watch: main returned with status 0; waiting for changes");
    let (stdout, seen) = session.stop();
    assert!(stdout.contains("\"Clicks: 1\""), "{}", stdout);
    let reload = seen
        .iter()
        .position(|line| line.starts_with("timing: reload ") && line.contains("detect"))
        .unwrap_or_else(|| panic!("no reload timing: {:#?}", seen));
    let pass = seen
        .iter()
        .position(|line| line.starts_with("timing: reload pass "))
        .unwrap_or_else(|| panic!("no reload pass timing: {:#?}", seen));
    assert!(reload < pass, "{:#?}", seen);
    let rest = seen[pass].strip_prefix("timing: reload pass ").unwrap();
    let (milliseconds, scopes) = rest.split_once(" ms (").expect("a time and its scopes");
    assert!(
        milliseconds.parse::<f64>().unwrap() >= 0.0,
        "{}",
        seen[pass]
    );
    let scopes = scopes.strip_suffix(" scopes)").expect("a scope count");
    assert!(scopes.parse::<usize>().unwrap() >= 1, "{}", seen[pass]);
    fs::remove_dir_all(&directory).ok();
}

/// The counter screen of `ui`'s "Counter screen", in a 400 × 300 app.
const COUNTER_SCREEN: &str = "import /prism/compose/runtime\nimport /prism/compose/ui
import /prism/compose/layout
import /prism/compose/material3
compose fun Counter() {
    val c = remember { mutableStateOf(0) }
    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(\"Count: ${c.value}\")
        Button(onClick = { c.value += 1 }) { Text(\"+\") }
    }
}
fun main() { runApp(width = 400, height = 300) { Counter() } }
";

/// Runs the counter screen headless with `--frame-stats` and the event script `script`, and
/// gives its stderr after checking that it succeeded with empty stdout.
fn counter_frame_stats(test_name: &str, script: &str) -> String {
    let directory = single_file(test_name, COUNTER_SCREEN);
    write(&directory.join("events"), script);
    let output = prism(
        &[
            "run",
            "--frame-stats",
            "--headless",
            "--events",
            "events",
            "main.prism",
        ],
        &directory,
    );
    let _ = fs::remove_dir_all(&directory);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    stderr(&output)
}

/// The report's lines, each without its `frame stats: ` prefix.
fn report_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(|line| {
            line.strip_prefix("frame stats: ")
                .unwrap_or_else(|| panic!("not a report line: {}", line))
                .to_string()
        })
        .collect()
}

/// execution: "Frame statistics without an app".
#[test]
fn frame_statistics_without_an_app() {
    let directory = single_file(
        "frame_stats_no_app",
        "import /prism/core\nfun main() { println(1) }\n",
    );
    let output = prism(&["run", "--frame-stats", "main.prism"], &directory);
    let _ = fs::remove_dir_all(&directory);
    assert_eq!(stdout(&output), "1\n");
    assert_eq!(stderr(&output), "");
    assert_eq!(output.status.code(), Some(0));
}

/// execution: "Frame statistics of a headless app".
#[test]
fn frame_statistics_of_a_headless_app() {
    let stderr = counter_frame_stats("frame_stats_headless", "click \"+\"\nframe\n");
    assert!(
        stderr.starts_with("frame stats: frames 1, events 2, passes 2, reloads 0, "),
        "{}",
        stderr
    );
    assert_eq!(report_lines(&stderr).len(), 8, "{}", stderr);
}

/// ui: "Two clicks and one frame".
#[test]
fn two_clicks_and_one_frame() {
    let stderr = counter_frame_stats(
        "frame_stats_two_clicks",
        "click \"+\"\nclick \"+\"\nframe\n",
    );
    assert!(
        stderr.starts_with("frame stats: frames 1, events 4, passes 3, reloads 0, "),
        "{}",
        stderr
    );
}

/// ui: "Paced clicks" (`strong-skipping`): a pause sleeps the program thread outside every
/// frame. The fastest of five runs with the pause is at least 50 ms slower than the fastest of
/// five without it, and no frame holds the pause.
#[test]
fn paced_clicks() {
    let paced = "click \"+\"\nframe\npause 50\nclick \"+\"\nframe\n";
    let unpaced = "click \"+\"\nframe\nclick \"+\"\nframe\n";
    // Interleaved runs, the fastest of five each, so that a busy moment of the machine slows
    // both kinds alike.
    let time = |name: &str, script: &str| {
        let started = std::time::Instant::now();
        let stderr = counter_frame_stats(name, script);
        (started.elapsed(), stderr)
    };
    let mut unpaced_time = std::time::Duration::MAX;
    let mut paced_time = std::time::Duration::MAX;
    let mut stderr = String::new();
    for run in 0..5 {
        unpaced_time = unpaced_time.min(time(&format!("paced_clicks_without_{}", run), unpaced).0);
        let (elapsed, report) = time(&format!("paced_clicks_with_{}", run), paced);
        paced_time = paced_time.min(elapsed);
        stderr = report;
    }
    assert!(
        paced_time >= unpaced_time + std::time::Duration::from_millis(50),
        "{:?} against {:?}",
        paced_time,
        unpaced_time
    );
    assert!(
        stderr.starts_with("frame stats: frames 2, events 4, passes 3, "),
        "{}",
        stderr
    );
    let frame = report_lines(&stderr)
        .into_iter()
        .find(|line| line.starts_with("frame median "))
        .unwrap();
    let maximum: f64 = frame
        .rsplit("max ")
        .next()
        .and_then(|rest| rest.strip_suffix(" ms"))
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("no maximum in {}", frame));
    assert!(maximum < 50.0, "{}", frame);
}

/// ui: "Dumps are not measured".
#[test]
fn dumps_are_not_measured() {
    let dumps = counter_frame_stats("frame_stats_dumps", "frame\ndump\ndump\n");
    let alone = counter_frame_stats("frame_stats_frame_alone", "frame\n");
    for stderr in [&dumps, &alone] {
        assert!(
            stderr.starts_with("frame stats: frames 1, events 0, passes 1, "),
            "{}",
            stderr
        );
    }
    let allocations = |stderr: &str| {
        report_lines(stderr)
            .into_iter()
            .find(|line| line.starts_with("allocations per frame"))
            .unwrap()
    };
    assert_eq!(allocations(&dumps), allocations(&alone));
}

/// ui: "The lines of a report".
#[test]
fn the_lines_of_a_report() {
    let script = "click \"+\"\nframe\n".repeat(10);
    let stderr = counter_frame_stats("frame_stats_lines", &script);
    let lines = report_lines(&stderr);
    let prefixes = [
        "frames 10, ",
        "events median ",
        "recomposition median ",
        "layout median ",
        "display list median ",
        "frame median ",
        "allocations per frame median ",
        "live allocations after each frame from ",
    ];
    assert_eq!(lines.len(), prefixes.len(), "{}", stderr);
    for (line, prefix) in lines.iter().zip(prefixes) {
        assert!(
            line.starts_with(prefix),
            "{:?} does not start with {:?}",
            line,
            prefix
        );
    }
    assert!(
        !lines.iter().any(|line| line.starts_with("reload ")),
        "{}",
        stderr
    );
}

/// ui: "Allocations of a counter".
#[test]
fn allocations_of_a_counter() {
    let stderr = counter_frame_stats("frame_stats_allocations", "frame\nclick \"+\"\nframe\n");
    let lines = report_lines(&stderr);
    let last_two = &lines[lines.len() - 2..];
    assert!(
        last_two[0].starts_with("allocations per frame median "),
        "{}",
        stderr
    );
    assert!(
        last_two[1].starts_with("live allocations after each frame from "),
        "{}",
        stderr
    );
}

/// The live allocations of a report's last line, smallest and largest.
fn live_allocations(stderr: &str) -> (u64, u64) {
    let line = report_lines(stderr).pop().unwrap();
    let rest = line
        .strip_prefix("live allocations after each frame from ")
        .unwrap();
    let (smallest, largest) = rest.split_once(" to ").unwrap();
    (smallest.parse().unwrap(), largest.parse().unwrap())
}

/// Runs the counter app of `examples/` headless with `--frame-stats` and `script`, from the
/// examples' workspace.
fn example_counter_frame_stats(test_name: &str, script: &str) -> String {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let directory = temporary_directory(test_name);
    write(&directory.join("events"), script);
    let events = directory.join("events");
    let output = prism(
        &[
            "run",
            "--frame-stats",
            "--headless",
            "--events",
            events.to_str().unwrap(),
            "apps/counter/main.prism",
        ],
        &examples,
    );
    let _ = fs::remove_dir_all(&directory);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "");
    stderr(&output)
}

/// example-apps: "Repeated clicks keep the live allocations constant".
#[test]
fn repeated_clicks_keep_the_live_allocations_constant() {
    let stderr = example_counter_frame_stats(
        "frame_stats_repeated_clicks",
        &"click button 1\nframe\n".repeat(10),
    );
    let (smallest, largest) = live_allocations(&stderr);
    assert!(smallest > 0, "{}", stderr);
    assert_eq!(smallest, largest, "{}", stderr);
}

/// example-apps: "Clicking empty space".
#[test]
fn clicking_empty_space() {
    let script = format!("frame\n{}", "click 300 190\nframe\n".repeat(10));
    let stderr = example_counter_frame_stats("frame_stats_empty_space", &script);
    assert!(
        stderr.contains("frame stats: allocations per frame median 0, "),
        "{}",
        stderr
    );
    assert!(stderr.contains("none in 10 of 11 frames"), "{}", stderr);
}

/// example-apps: "Typing into the field of a thousand items skips every row" (`strong-skipping`
/// D6): the rows skip without allocating, since the key lambdas borrow the versions they
/// capture and keep their environments in the creating frame. A frame allocates only what the
/// text field's change does, never one environment per row.
#[test]
fn typing_into_the_field_of_a_thousand_items_skips_every_row_without_allocating() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let directory = temporary_directory("typing_allocations");
    let events = directory.join("events");
    write(
        &events,
        &format!(
            "click textfield\nframe\n{}",
            "type \"a\"\nframe\n".repeat(10)
        ),
    );
    let output = prism(
        &[
            "run",
            "--frame-stats",
            "--headless",
            "--events",
            events.to_str().unwrap(),
            "apps/todo/large.prism",
        ],
        &examples,
    );
    let _ = fs::remove_dir_all(&directory);
    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let stderr = stderr(&output);
    let median: u64 = stderr
        .lines()
        .find_map(|line| line.strip_prefix("frame stats: allocations per frame median "))
        .and_then(|rest| rest.split(',').next())
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("no allocations line in {}", stderr));
    assert!(median < 100, "{}", stderr);
}

/// The declarations of the code-generation scenarios of `operator-conventions`: a `User` whose
/// `equals` compares ids, inside a `Team`.
const USER_WITH_EQUALS: &str = "struct User { val id: Int, val name: String }\n\
     operator fun User.equals(other: &User): Bool = id == other.id\n\
     struct Team { val lead: User, val size: Int }\n";

/// code-generation: "Equality expansion calling equals" (`operator-conventions`): `same`
/// compares `size` inline and calls `User.equals` through its slot for `lead`, and compares no
/// string.
#[test]
fn equality_expansion_calling_equals() {
    let directory = single_file(
        "equality_calling_equals",
        &format!(
            "{}fun same(a: &Team, b: &Team): Bool = a == b\nfun main() {{}}\n",
            USER_WITH_EQUALS
        ),
    );
    let output = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = stdout(&output);
    let same = function_ir(&dump, "main::same");
    assert!(
        same.contains("symbol %\"main::main.User.equals#slot\"") && same.contains("call_indirect"),
        "{}",
        same
    );
    assert!(!same.contains("prism_rt_string_equals"), "{}", same);
    assert!(same.contains("icmp eq"), "{}", same);
}

/// code-generation: "List search calling equals" (`operator-conventions`): the index-of glue of
/// `List<User>` calls `User.equals` for each element.
#[test]
fn list_search_calling_equals() {
    let directory = single_file(
        "list_search_calling_equals",
        &format!(
            "{}fun has(xs: &List<User>, u: &User): Bool = xs.contains(u)\nfun main() {{}}\n",
            USER_WITH_EQUALS
        ),
    );
    let output = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = stdout(&output);
    let glue = function_ir(&dump, "main#indexOf(List<main::User>)");
    assert!(
        glue.contains("symbol %\"main::main.User.equals#slot\"") && glue.contains("call_indirect"),
        "{}",
        glue
    );
    assert!(!glue.contains("prism_rt_string_equals"), "{}", glue);
}

// --- Drop and clone hooks (drop-and-clone-hooks)

/// code-generation: "Drop glue calls the hook through its slot".
#[test]
fn drop_glue_calls_the_hook_through_its_slot() {
    let directory = single_file(
        "drop_hook_slot",
        "struct Session { val id: Int, val name: String }\noperator mut fun Session.drop() {}\nfun f(s: Session) {}\n",
    );
    let output = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = stdout(&output);
    let glue = function_ir(&dump, "main#drop(main::Session)");
    let slot = glue
        .find("symbol %\"main::main.Session.drop#slot\"")
        .unwrap_or_else(|| panic!("no hook slot in\n{}", glue));
    let hook = glue
        .find("call_indirect")
        .unwrap_or_else(|| panic!("no hook call in\n{}", glue));
    let string = glue
        .find("call fn2(")
        .unwrap_or_else(|| panic!("no string drop in\n{}", glue));
    assert!(glue.contains("fn2 = %\"prism_rt_string_drop\""), "{}", glue);
    assert!(slot < hook && hook < string, "{}", glue);
    let f = function_ir(&dump, "main::f");
    assert!(f.contains("%\"main#drop(main::Session)\""), "{}", f);
    assert!(!f.contains("call_indirect"), "{}", f);
}

/// code-generation: "An importer's glue calls the hook of an imported type".
#[test]
fn an_importer_s_glue_calls_the_hook_of_an_imported_type() {
    let directory = temporary_directory("importer_hook_glue");
    write(
        &directory.join("lib/net.prism"),
        "package lib\npublic struct Session(val id: Int)\noperator mut fun Session.drop() {}\n",
    );
    write(
        &directory.join("app/main.prism"),
        "package app\nimport /lib/net\nfun f(s: Session) {}\n",
    );
    let output = prism(&["dump-clif", "app/main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = stdout(&output);
    let glue = function_ir(&dump, "app/main#drop(lib/net::Session)");
    assert!(
        glue.contains("symbol %\"lib/net::lib/net.Session.drop#slot\""),
        "{}",
        glue
    );
    assert!(glue.contains("call_indirect"), "{}", glue);
}

/// code-generation: "Clone glue calls a generic type's clone hook instance".
#[test]
fn clone_glue_calls_a_generic_type_s_clone_hook_instance() {
    let directory = single_file(
        "generic_clone_hook_glue",
        "import /prism/core\nstruct Owned<T> : Clone { val v: T }\noperator fun <T : Clone> Owned<T>.clone(): Owned<T> = Owned(v.clone())\nfun f(o: &Owned<String>): Owned<String> = o.clone()\n",
    );
    let output = prism(&["dump-clif", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let dump = stdout(&output);
    let glue = function_ir(&dump, "main#clone(main::Owned<String>)");
    assert!(
        glue.contains("symbol %\"main::main.Owned.clone<String>#slot\""),
        "{}",
        glue
    );
    assert!(glue.contains("call_indirect"), "{}", glue);
    let output = prism(&["dump-clif", "--instances", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    function_ir(&stdout(&output), "main::main.Owned.clone<String>");
}

/// execution: "Without the check a top-level value runs no hook".
#[test]
fn without_the_check_a_top_level_value_runs_no_hook() {
    let directory = single_file(
        "top_level_hook_unchecked",
        "import /prism/core\nstruct Noisy { val name: String }\noperator mut fun Noisy.drop() { println(\"drop \" + name) }\nval GLOBAL: Noisy = Noisy(\"global\")\nfun main() { println(GLOBAL.name) }\n",
    );
    let output = prism(&["run", "main.prism"], &directory);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "global\n");
    assert_eq!(stderr(&output), "");
}

/// compilation-and-abi: "Annotations change the ABI hash" (`compose-library-in-prism` D12):
/// `@Keeper` is part of a library struct's ABI entry, hashed and printed.
#[test]
fn annotations_change_the_abi_hash() {
    let hash_and_text = |name: &str, source: &str| {
        let directory = single_file(name, source);
        let output = prism(&["abi", "main.prism"], &directory);
        assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
        let text = stdout(&output);
        let hash = text
            .lines()
            .find_map(|line| line.strip_prefix("ABI Hash: "))
            .expect("an ABI hash")
            .to_string();
        (hash, text)
    };
    let (keeper_hash, keeper) = hash_and_text(
        "annotated_keeper",
        "package prism\n@Keeper public struct H(private val n: Int)\n",
    );
    let (plain_hash, plain) = hash_and_text(
        "plain_keeper",
        "package prism\npublic struct H(private val n: Int)\n",
    );
    assert_ne!(keeper_hash, plain_hash);
    assert!(keeper.contains("  @Keeper public struct H"), "{}", keeper);
    assert!(plain.contains("  public struct H"), "{}", plain);
    assert!(!plain.contains("@Keeper"), "{}", plain);
    let (driving_hash, driving) = hash_and_text(
        "annotated_driver",
        "package prism\n@DrivesComposition public fun drive(): Unit {}\n",
    );
    let (_, explicit) = hash_and_text(
        "explicit_groups",
        "package prism\n@ExplicitGroupsComposable public compose fun G(): Unit {}\n",
    );
    let (plain_composable_hash, _) = hash_and_text(
        "plain_composable",
        "package prism\npublic compose fun G(): Unit {}\n",
    );
    let (plain_driver_hash, _) = hash_and_text(
        "plain_driver",
        "package prism\npublic fun drive(): Unit {}\n",
    );
    assert_ne!(driving_hash, plain_driver_hash);
    assert!(
        driving.contains("  @DrivesComposition public fun drive(): Unit"),
        "{}",
        driving
    );
    // `@ExplicitGroupsComposable` changes only its own body's lowering (D5).
    let explicit_hash = explicit
        .lines()
        .find_map(|line| line.strip_prefix("ABI Hash: "))
        .unwrap();
    assert_eq!(explicit_hash, plain_composable_hash);
}
