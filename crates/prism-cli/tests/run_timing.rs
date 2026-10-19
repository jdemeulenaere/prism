//! The startup benchmark of `prism run` (the `execution` spec, "Tier 1 Instant Execution"; D22 of
//! `codegen-core`): how long `prism run` takes to reach `main` on the reference program
//! `examples/run/main.prism`, against the 15 ms target.
//!
//! It is ignored by default, since a debug build or a busy machine says nothing about the
//! target. Run it on a release build with
//!
//! ```text
//! cargo test --release -p prism-cli --test run_timing -- --ignored --nocapture
//! ```
//!
//! It reports the median and 90th percentile of each stage over 20 runs, cold (without the
//! build cache, so every run compiles the program) and warm (from a temporary cache the first run
//! filled, so no run compiles anything, D9 of `instance-cache`), with the wall time of the whole
//! process, which also counts what happens before `main` of `prism` itself, such as the dynamic
//! loader mapping the window crate's frameworks (D4 of `desktop-window`), and fails on the cold
//! time only when `PRISM_ASSERT_TIMING=1` is set.

use std::path::Path;
use std::process::Command;

const RUNS: usize = 20;
const TARGET_MILLISECONDS: f64 = 15.0;
const STAGES: [&str; 4] = ["front end", "code generation", "linking", "total to main"];
/// The label of the process's wall time, reported after the stages.
const WALL: &str = "wall time";

/// The milliseconds of each stage of one `prism run --timings` run with `PRISM_CACHE` set to
/// `cache` (`off` for a cold run), then the process's wall time.
fn run_once(root: &Path, cache: &std::ffi::OsStr) -> Vec<f64> {
    let started = std::time::Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_prism"))
        .env("PRISM_CACHE", cache)
        .args(["run", "--timings", "examples/run/main.prism"])
        .current_dir(root)
        .output()
        .expect("failed to run prism");
    let wall = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut times: Vec<f64> = STAGES
        .iter()
        .map(|stage| {
            stderr
                .lines()
                .find_map(|line| line.strip_prefix(&format!("timing: {} ", stage)))
                .and_then(|rest| rest.strip_suffix(" ms"))
                .and_then(|value| value.parse().ok())
                .unwrap_or_else(|| panic!("no '{}' timing in\n{}", stage, stderr))
        })
        .collect();
    times.push(wall);
    times
}

/// Waits until `prism`'s executable is older than the build cache's settle time (2 s, D16 of
/// `instance-cache`), so that the warm runs read its hash from the cache, as a user's warm runs
/// do, rather than hash an executable cargo has just linked on every run.
fn wait_for_settled_executable() {
    let settle = std::time::Duration::from_millis(2500);
    let age = std::fs::metadata(env!("CARGO_BIN_EXE_prism"))
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok());
    if let Some(age) = age {
        if age < settle {
            std::thread::sleep(settle - age);
        }
    }
}

/// The value at `fraction` of the sorted `values`.
fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let index = ((values.len() - 1) as f64 * fraction).round() as usize;
    values[index]
}

#[test]
#[ignore = "a benchmark: run it on a release build with --ignored --nocapture"]
fn tier_1_instant_script_execution() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cache = std::env::temp_dir().join(format!("prism_run_timing_cache_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let mut total_median = 0.0;
    for (label, cache) in [
        ("cold", std::ffi::OsStr::new("off")),
        ("warm", cache.as_os_str()),
    ] {
        if label == "warm" {
            wait_for_settled_executable();
        }
        // A first run warms the file system cache and the dynamic loader, and fills the build
        // cache for the warm runs.
        run_once(&root, cache);
        let runs: Vec<Vec<f64>> = (0..RUNS).map(|_| run_once(&root, cache)).collect();
        println!(
            "prism run --timings examples/run/main.prism, {} {} runs, {} build:",
            RUNS, label, build
        );
        for (index, stage) in STAGES.iter().chain([&WALL]).enumerate() {
            let mut values: Vec<f64> = runs.iter().map(|run| run[index]).collect();
            let median = percentile(&mut values, 0.5);
            let p90 = percentile(&mut values, 0.9);
            println!(
                "  {:<16} median {:>7.3} ms   p90 {:>7.3} ms",
                stage, median, p90
            );
            if *stage == "total to main" && label == "cold" {
                total_median = median;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&cache);
    println!(
        "  target: total to main under {} ms: {}",
        TARGET_MILLISECONDS,
        if total_median < TARGET_MILLISECONDS {
            "met"
        } else {
            "MISSED"
        }
    );
    if std::env::var_os("PRISM_ASSERT_TIMING").is_some_and(|value| value == "1") {
        assert!(
            total_median < TARGET_MILLISECONDS,
            "the median time to main is {:.3} ms, above the {} ms target",
            total_median,
            TARGET_MILLISECONDS
        );
    }
}
