//! Warm-run timing of `prism run` with the build cache (group 7 of `instance-cache`).
//!
//! Generates workspaces of three shapes in three sizes four times apart and compiles each through
//! `compile_program_with` on a temporary cache: cold, warm with no edit, warm after a non-generic
//! body edit and warm after a generic body edit. It reports the front end (resolution, Phase 1,
//! and Phase 2 or reading cached verdicts), code generation (checks, planning, instantiation and
//! compiling what missed) and linking (defining every artifact, fresh or loaded). Under
//! `PRISM_ASSERT_TIMING=1` it asserts that each stage grows near-linearly, at most 1.5 times as
//! fast as the size, and that the unchanged warm run compiles no function.
//!
//! `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test warm_run_timing -- --ignored --nocapture`

use prism_abi::ContentHash;
use prism_cli::{compile_program_with, BuildCache, RunOptions};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SIZES: [usize; 3] = [64, 256, 1024];

/// The number of times each workspace is measured, keeping each stage's fastest time.
const TRIALS: usize = 3;

/// A shape's name and its generator for a size.
type Shape = (&'static str, fn(usize) -> Generated);
const RUNS: [&str; 4] = ["cold", "warm", "body edit", "generic edit"];

/// A generated workspace: its files, the entry, and the two edits, each `(path, new source)`.
struct Generated {
    files: Vec<(String, String)>,
    body_edit: (String, String),
    generic_edit: (String, String),
}

/// `n` files in a chain, each importing the previous one, with a few functions and a call of a
/// generic of the first file.
fn many_files(n: usize) -> Generated {
    let generic = |extra: &str| {
        format!(
            "package lib\npublic fun <T> keep(x: T): T {{ {}return x }}\n",
            extra
        )
    };
    let file = |i: usize, body: &str| {
        let import = if i == 0 {
            "import /lib/generic\n".to_string()
        } else {
            format!("import f{}\n", i - 1)
        };
        let previous = if i == 0 {
            "keep(1)".to_string()
        } else {
            format!("f{}a(1)", i - 1)
        };
        format!(
            "package app\n{}public fun f{i}a(n: Int): Int = n + {body}\n\
             public fun f{i}b(n: Int): Int = f{i}a(n) * 2\n\
             public fun f{i}c(): Int = {previous}\n",
            import
        )
    };
    let mut files = vec![("lib/generic.prism".to_string(), generic(""))];
    for i in 0..n {
        files.push((format!("app/f{}.prism", i), file(i, "1")));
    }
    files.push((
        "app/main.prism".to_string(),
        format!(
            "package app\nimport f{}\nfun main() {{ println(f{}c()) }}\n",
            n - 1,
            n - 1
        ),
    ));
    Generated {
        body_edit: (format!("app/f{}.prism", n / 2), file(n / 2, "2")),
        generic_edit: ("lib/generic.prism".to_string(), generic("val y = 1; ")),
        files,
    }
}

/// One generic instantiated with `n` distinct structs.
fn many_instances(n: usize) -> Generated {
    let generic = |extra: &str| {
        format!(
            "package lib\npublic fun <T> keep(x: T): T {{ {}return x }}\npublic fun plain(n: Int): Int = n\n",
            extra
        )
    };
    let main = |body: &str| {
        let mut source = String::from("package app\nimport /lib/box\n");
        for i in 0..n {
            source.push_str(&format!("struct S{i}(val v: Int)\n"));
        }
        source.push_str("fun main() {\n    var total = 0\n");
        for i in 0..n {
            source.push_str(&format!("    total += keep(S{i}({i})).v\n"));
        }
        source.push_str(&format!("    println(total + {})\n}}\n", body));
        source
    };
    Generated {
        files: vec![
            ("lib/box.prism".to_string(), generic("")),
            ("app/main.prism".to_string(), main("0")),
        ],
        body_edit: ("app/main.prism".to_string(), main("1")),
        generic_edit: ("lib/box.prism".to_string(), generic("val y = 1; ")),
    }
}

/// `n` files, each declaring a generic that calls the previous file's generic.
fn instance_chains(n: usize) -> Generated {
    let file = |i: usize, extra: &str| {
        if i == 0 {
            format!("package lib\npublic fun <T> g0(x: T): T {{ {}return x }}\npublic fun h0(): Int = 1\n", extra)
        } else {
            format!(
                "package lib\nimport g{}\npublic fun <T> g{i}(x: T): T {{ {}return g{}(x) }}\npublic fun h{i}(): Int = {i}\n",
                i - 1,
                extra,
                i - 1
            )
        }
    };
    let mut files: Vec<(String, String)> = (0..n)
        .map(|i| (format!("lib/g{}.prism", i), file(i, "")))
        .collect();
    files.push((
        "app/main.prism".to_string(),
        format!(
            "package app\nimport /lib/g{}\nfun main() {{ println(g{}(1)); println(g{}(true)) }}\n",
            n - 1,
            n - 1,
            n - 1
        ),
    ));
    Generated {
        files,
        body_edit: (
            format!("lib/g{}.prism", n / 2),
            file(n / 2, "").replace(
                &format!("Int = {}\n", n / 2),
                &format!("Int = {}\n", n / 2 + 1),
            ),
        ),
        generic_edit: (format!("lib/g{}.prism", n / 2), file(n / 2, "val y = 1; ")),
    }
}

fn temporary(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("prism_warm_timing_{}_{}", name, nanos));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write(root: &Path, (path, source): &(String, String)) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

/// The stages of the four runs over `generated`, and the functions each run compiled.
fn measure(name: &str, generated: &Generated) -> Vec<([Duration; 3], usize)> {
    let base = temporary(name);
    let root = base.join("workspace");
    for file in &generated.files {
        write(&root, file);
    }
    let entry = root.join("app/main.prism");
    let options = RunOptions {
        cache: Some(BuildCache::open(
            base.join("cache"),
            prism_cli::cache::identity_of("timing", &ContentHash([1; 32])),
        )),
        ..RunOptions::default()
    };
    let mut results = Vec::new();
    for run in RUNS {
        match run {
            "body edit" => write(&root, &generated.body_edit),
            "generic edit" => write(&root, &generated.generic_edit),
            _ => {}
        }
        let compiled = compile_program_with(&entry, &options)
            .unwrap_or_else(|error| panic!("{} {}: {}", name, run, error.render()));
        let timings = compiled.timings;
        results.push((
            [timings.front_end, timings.code_generation, timings.linking],
            compiled.compiled_functions,
        ));
    }
    fs::remove_dir_all(&base).ok();
    results
}

#[test]
#[ignore = "a benchmark: run it on a release build with --ignored --nocapture"]
fn warm_runs_take_linear_time() {
    let assert = std::env::var_os("PRISM_ASSERT_TIMING").is_some();
    let shapes: [Shape; 3] = [
        ("many_files", many_files),
        ("many_instances", many_instances),
        ("instance_chains", instance_chains),
    ];
    let milliseconds = |duration: Duration| duration.as_secs_f64() * 1000.0;
    let mut failures = Vec::new();
    for (shape, generate) in shapes {
        let mut by_size = Vec::new();
        for size in SIZES {
            let generated = generate(size);
            // The fastest of a few trials, each on a fresh workspace and cache, since file
            // system activity after a cold run's writes makes single runs noisy.
            let mut results = measure(&format!("{}_{}", shape, size), &generated);
            for trial in 1..TRIALS {
                let again = measure(&format!("{}_{}_{}", shape, size, trial), &generated);
                for ((stages, _), (other, _)) in results.iter_mut().zip(&again) {
                    for (stage, other) in stages.iter_mut().zip(other) {
                        *stage = (*stage).min(*other);
                    }
                }
            }
            for (run, (stages, compiled)) in RUNS.iter().zip(&results) {
                println!(
                    "{:16} {:5} {:13} front end {:9.2} ms  code generation {:9.2} ms  linking {:8.2} ms  compiled {:6}",
                    shape,
                    size,
                    run,
                    milliseconds(stages[0]),
                    milliseconds(stages[1]),
                    milliseconds(stages[2]),
                    compiled
                );
            }
            if results[1].1 != 0 {
                failures.push(format!(
                    "{} {}: the unchanged warm run compiled {} functions",
                    shape, size, results[1].1
                ));
            }
            by_size.push(results);
        }
        // Each stage of each run grows near-linearly: at most 1.5 times as fast as the size, so
        // 6x for 4x the files (a quadratic step would show 16x), with a floor of one
        // millisecond under which noise dominates. Reading every import's directory once per
        // import made the warm front end of `many_files` grow 6.6x before the resolver read each
        // directory once.
        for pair in by_size.windows(2) {
            for (run_index, run) in RUNS.iter().enumerate() {
                for (stage_index, stage) in ["front end", "code generation", "linking"]
                    .iter()
                    .enumerate()
                {
                    let small = milliseconds(pair[0][run_index].0[stage_index]).max(1.0);
                    let large = milliseconds(pair[1][run_index].0[stage_index]);
                    if large > small * 6.0 {
                        failures.push(format!(
                            "{} {} {}: {:.2} ms to {:.2} ms for 4x the size",
                            shape, run, stage, small, large
                        ));
                    }
                }
            }
        }
    }
    if assert {
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    } else if !failures.is_empty() {
        println!("not asserted:\n{}", failures.join("\n"));
    }
}
