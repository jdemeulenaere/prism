//! The run-time benchmark of a headless app (D19 of `ui-toolkit`): how long one click of a
//! counter takes, from the event to the recomposition and relayout it causes, in an app of
//! 1,000 `Text`s.
//!
//! It is ignored by default, since a debug build or a busy machine says nothing about the
//! target. Run it on a release build with
//!
//! ```text
//! cargo test --release -p prism-cli --test ui_run_timing -- --ignored --nocapture
//! ```
//!
//! Each run is a `prism run --headless` of the generated app from a warm build cache; the time
//! per click is the difference between a run of 100 clicks and a run of none, divided by 100,
//! so that compiling, linking and the first pass are left out. It reports the median over 10
//! pairs of runs. On a release build, the median must be under 1 ms (D9 of
//! `reactive-app-verification`); the test fails on it only when `PRISM_ASSERT_TIMING=1` is
//! set.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

const RUNS: usize = 10;
const CLICKS: usize = 100;
const TEXTS: usize = 1000;
const TARGET_MILLISECONDS: f64 = 1.0;

/// The app: a column of a counter's label and button, then `TEXTS` texts.
fn app() -> String {
    let mut texts = String::new();
    for index in 0..TEXTS {
        texts.push_str(&format!("        Text(\"row {}\")\n", index));
    }
    format!(
        "import /prism/compose/ui\n\
         import /prism/compose/layout\n\
         import /prism/compose/material3\n\
         compose fun Screen() {{\n\
         \x20   val c = remember {{ mutableStateOf(0) }}\n\
         \x20   Column {{\n\
         \x20       Text(\"Count: ${{c.value}}\")\n\
         \x20       Button(onClick = {{ c.value += 1 }}) {{ Text(\"+\") }}\n\
         {texts}\
         \x20   }}\n\
         }}\n\
         fun main() {{ runApp(width = 800, height = 30000) {{ Screen() }} }}\n"
    )
}

/// The seconds one headless run of `main.prism` in `directory` with `script` takes.
fn run_once(directory: &Path, cache: &Path, script: &str) -> f64 {
    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_prism"))
        .env("PRISM_CACHE", cache)
        .args(["run", "--headless", "--events", script, "main.prism"])
        .current_dir(directory)
        .output()
        .expect("failed to run prism");
    let elapsed = started.elapsed().as_secs_f64();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    elapsed
}

#[test]
#[ignore = "a benchmark: run it on a release build with --ignored --nocapture"]
fn a_click_in_an_app_of_a_thousand_texts() {
    let directory =
        std::env::temp_dir().join(format!("prism_ui_run_timing_{}", std::process::id()));
    let cache = directory.join("cache");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("main.prism"), app()).unwrap();
    std::fs::write(directory.join("none.txt"), "frame\n").unwrap();
    std::fs::write(
        directory.join("clicks.txt"),
        format!("frame\n{}", "click \"+\"\n".repeat(CLICKS)),
    )
    .unwrap();
    // The first run fills the build cache.
    run_once(&directory, &cache, "none.txt");
    let mut per_click: Vec<f64> = (0..RUNS)
        .map(|_| {
            let none = run_once(&directory, &cache, "none.txt");
            let clicks = run_once(&directory, &cache, "clicks.txt");
            (clicks - none) / CLICKS as f64
        })
        .collect();
    per_click.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "a click in an app of {} texts, {} build: median {:.3} ms over {} pairs of runs",
        TEXTS,
        build,
        per_click[RUNS / 2] * 1000.0,
        RUNS
    );
    std::fs::remove_dir_all(&directory).ok();
    let median = per_click[RUNS / 2] * 1000.0;
    let met = median < TARGET_MILLISECONDS;
    println!(
        "  target: median per click under {} ms: {}",
        TARGET_MILLISECONDS,
        if met { "met" } else { "MISSED" }
    );
    if std::env::var_os("PRISM_ASSERT_TIMING").is_some_and(|value| value == "1") {
        assert!(
            met,
            "a click took {:.3} ms, over the {} ms target",
            median, TARGET_MILLISECONDS
        );
    }
}
