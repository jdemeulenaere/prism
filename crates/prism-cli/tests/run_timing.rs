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
//! filled, so no run compiles anything, D9 of `instance-cache`), and fails on the cold time only
//! when `PRISM_ASSERT_TIMING=1` is set.

use std::path::Path;
use std::process::Command;

const RUNS: usize = 20;
const TARGET_MILLISECONDS: f64 = 15.0;
const STAGES: [&str; 4] = ["front end", "code generation", "linking", "total to main"];

/// The milliseconds of each stage of one `prism run --timings` run with `PRISM_CACHE` set to
/// `cache` (`off` for a cold run).
fn run_once(root: &Path, cache: &std::ffi::OsStr) -> Vec<f64> {
    let output = Command::new(env!("CARGO_BIN_EXE_prism"))
        .env("PRISM_CACHE", cache)
        .args(["run", "--timings", "examples/run/main.prism"])
        .current_dir(root)
        .output()
        .expect("failed to run prism");
    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    STAGES
        .iter()
        .map(|stage| {
            stderr
                .lines()
                .find_map(|line| line.strip_prefix(&format!("timing: {} ", stage)))
                .and_then(|rest| rest.strip_suffix(" ms"))
                .and_then(|value| value.parse().ok())
                .unwrap_or_else(|| panic!("no '{}' timing in\n{}", stage, stderr))
        })
        .collect()
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
        // A first run warms the file system cache and the dynamic loader, and fills the build
        // cache for the warm runs.
        run_once(&root, cache);
        let runs: Vec<Vec<f64>> = (0..RUNS).map(|_| run_once(&root, cache)).collect();
        println!(
            "prism run --timings examples/run/main.prism, {} {} runs, {} build:",
            RUNS, label, build
        );
        for (index, stage) in STAGES.iter().enumerate() {
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
