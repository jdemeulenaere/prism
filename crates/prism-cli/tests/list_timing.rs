//! The list benchmark (the `execution` delta of `collections`, "Adding to a List Takes Amortized
//! Constant Time"; D19 of `collections`): the wall time of `prism run` on a program adding a
//! million `Int`s to an empty list and summing them, startup included, against the 20 ms
//! target.
//!
//! It is ignored by default, since a debug build or a busy machine says nothing about the
//! target. Run it on a release build with
//!
//! ```text
//! cargo test --release -p prism-cli --test list_timing -- --ignored --nocapture
//! ```
//!
//! It reports the median and 90th percentile over 20 runs, and fails on the time only when
//! `PRISM_ASSERT_TIMING=1` is set.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

const RUNS: usize = 20;
const TARGET_MILLISECONDS: f64 = 20.0;
const PROGRAM: &str = "tests/benchmarks/list_adds.prism";

/// The wall time of one `prism run` of the program, in milliseconds.
fn run_once(root: &Path) -> f64 {
    let start = Instant::now();
    // Without the cache, so that every run compiles the program.
    let output = Command::new(env!("CARGO_BIN_EXE_prism"))
        .env("PRISM_CACHE", "off")
        .args(["run", PROGRAM])
        .current_dir(root)
        .output()
        .expect("failed to run prism");
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "499999500000\n");
    elapsed
}

/// The value at `fraction` of the sorted `values`.
fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let index = ((values.len() - 1) as f64 * fraction).round() as usize;
    values[index]
}

#[test]
#[ignore = "a benchmark: run it on a release build with --ignored --nocapture"]
fn a_million_adds() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // A first run warms the file system cache and the dynamic loader.
    run_once(root);
    let mut times: Vec<f64> = (0..RUNS).map(|_| run_once(root)).collect();
    let median = percentile(&mut times, 0.5);
    let p90 = percentile(&mut times, 0.9);
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "prism run {}, {} runs, {} build: median {:.3} ms, p90 {:.3} ms",
        PROGRAM, RUNS, build, median, p90
    );
    println!(
        "  target: under {} ms: {}",
        TARGET_MILLISECONDS,
        if median < TARGET_MILLISECONDS {
            "met"
        } else {
            "MISSED"
        }
    );
    if std::env::var_os("PRISM_ASSERT_TIMING").is_some_and(|value| value == "1") {
        assert!(
            median < TARGET_MILLISECONDS,
            "the median time is {:.3} ms, above the {} ms target",
            median,
            TARGET_MILLISECONDS
        );
    }
}
