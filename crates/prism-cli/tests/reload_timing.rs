//! The reload benchmark of `prism run --watch` (the `hot-reload` spec, "Reload Latency"; D17 of
//! `hot-reload`): how long a one-line body edit of one file of a 100-file program takes to reach
//! the running program, against the 50 ms target, from the write on disk to the session's
//! `reload: applied` line, file-system events and the debounce included.
//!
//! It is ignored by default, since a debug build or a busy machine says nothing about the
//! target. Run it on a release build with
//!
//! ```text
//! cargo test --release -p prism-cli --test reload_timing -- --ignored --nocapture
//! ```
//!
//! It generates 100 files of 20 functions, each file importing the one before, and a `main`
//! that loops calling into every file. It runs `prism run --watch --timings` on a temporary build
//! cache, rewrites the body of one function of the middle file [`EDITS`] times, each time with a
//! new constant so that every reload compiles, and waits for each `reload: applied` line. It
//! reports the median and 90th percentile of the time from write to line and of each stage the
//! session reports, and fails on the median only when `PRISM_ASSERT_TIMING=1` is set.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const FILES: usize = 100;
const FUNCTIONS: usize = 20;
const EDITS: usize = 20;
const EDITED: usize = FILES / 2;
const TARGET_MILLISECONDS: f64 = 50.0;
const STAGES: [&str; 5] = [
    "detect",
    "front end",
    "code generation",
    "linking",
    "applied after",
];

/// The source of file `index`, whose function `f<index>_5` adds `constant`.
fn library(index: usize, constant: usize) -> String {
    let mut source = String::from("package lib\n");
    if index > 0 {
        source.push_str(&format!("import /lib/m{}\n", index - 1));
    }
    for function in 0..FUNCTIONS {
        let body = match (function, index) {
            (0, 0) => "n + 1".to_string(),
            (0, _) => format!("f{}_1(n) + 1", index - 1),
            (5, _) => format!("n + {}", constant),
            _ => format!("n * {} + {}", function, index),
        };
        source.push_str(&format!(
            "public fun f{index}_{function}(n: Int): Int = {body}\n"
        ));
    }
    source
}

/// The entry file: it imports every file and loops forever calling into each.
fn entry() -> String {
    let mut source = String::from("package app\n");
    for index in 0..FILES {
        source.push_str(&format!("import /lib/m{}\n", index));
    }
    let calls: Vec<String> = (0..FILES).map(|index| format!("f{}_0(1)", index)).collect();
    source.push_str(&format!(
        "fun main() {{ var i = 0; while (i >= 0) {{ i = (i + {}) % 1000 }} }}\n",
        calls.join(" + ")
    ));
    source
}

/// The value at `fraction` of the sorted `values`.
fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let index = ((values.len() - 1) as f64 * fraction).round() as usize;
    values[index]
}

/// The milliseconds of each stage in a `timing: reload ...` line.
fn stages(line: &str) -> Vec<f64> {
    let (_, stages) = line.split_once(" ms (").expect("a total and its stages");
    stages
        .trim_end_matches(')')
        .split(", ")
        .map(|stage| {
            let value = stage.strip_suffix(" ms").unwrap();
            value.rsplit_once(' ').unwrap().1.parse().unwrap()
        })
        .collect()
}

fn workspace() -> PathBuf {
    let root = std::env::temp_dir().join(format!("prism_reload_timing_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::create_dir_all(root.join("app")).unwrap();
    for index in 0..FILES {
        std::fs::write(
            root.join(format!("lib/m{}.prism", index)),
            library(index, 5),
        )
        .unwrap();
    }
    std::fs::write(root.join("app/main.prism"), entry()).unwrap();
    root
}

/// Waits up to 30 seconds for a line of `lines` satisfying `wanted`.
fn wait_for(
    lines: &mpsc::Receiver<(Instant, String)>,
    wanted: impl Fn(&str) -> bool,
) -> (Instant, String) {
    loop {
        match lines.recv_timeout(Duration::from_secs(30)) {
            Ok((at, line)) if wanted(&line) => return (at, line),
            Ok(_) => {}
            Err(_) => panic!("the session stopped reporting"),
        }
    }
}

/// The watch session, killed and reaped when dropped, so that a benchmark that fails or panics
/// leaves no session running its looping program.
struct KilledOnDrop(std::process::Child);

impl Drop for KilledOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "a benchmark: run it on a release build with --ignored --nocapture"]
fn reload_latency() {
    let root = workspace();
    let cache = root.join(".cache");
    let mut child = KilledOnDrop(
        Command::new(env!("CARGO_BIN_EXE_prism"))
            .env("PRISM_CACHE", &cache)
            .args(["run", "--watch", "--timings", "app/main.prism"])
            .current_dir(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to run prism"),
    );
    let (sender, lines) = mpsc::channel();
    let stderr = child.0.stderr.take().unwrap();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines() {
            let Ok(line) = line else { return };
            if sender.send((Instant::now(), line)).is_err() {
                return;
            }
        }
    });
    let (_, total_to_main) = wait_for(&lines, |line| line.starts_with("timing: total to main"));
    println!("first load: {}", total_to_main);
    // Let the watcher register the directories before the first edit.
    std::thread::sleep(Duration::from_millis(500));
    let edited: &Path = &root.join(format!("lib/m{}.prism", EDITED));
    let mut totals = Vec::new();
    let mut per_stage: Vec<Vec<f64>> = vec![Vec::new(); STAGES.len()];
    let mut modules = String::new();
    for edit in 0..EDITS {
        let written = Instant::now();
        std::fs::write(edited, library(EDITED, 100 + edit)).unwrap();
        let (applied, _) = wait_for(&lines, |line| {
            line == format!("reload: applied lib/m{}.prism", EDITED)
        });
        let (_, timing) = wait_for(&lines, |line| line.starts_with("timing: reload "));
        let (_, live) = wait_for(&lines, |line| line.starts_with("timing: live modules"));
        modules = live;
        totals.push(applied.duration_since(written).as_secs_f64() * 1000.0);
        for (values, value) in per_stage.iter_mut().zip(stages(&timing)) {
            values.push(value);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(child);
    let _ = std::fs::remove_dir_all(&root);

    let median = percentile(&mut totals, 0.5);
    println!(
        "reload of one body edit, {} files, release build, {} edits: write to applied median {:.3} ms, p90 {:.3} ms",
        FILES,
        EDITS,
        median,
        percentile(&mut totals, 0.9)
    );
    for (stage, values) in STAGES.iter().zip(per_stage.iter_mut()) {
        println!(
            "  {:<16} median {:>8.3} ms  p90 {:>8.3} ms",
            stage,
            percentile(values, 0.5),
            percentile(values, 0.9)
        );
    }
    println!("  {}", modules);
    if std::env::var_os("PRISM_ASSERT_TIMING").is_some() {
        assert!(
            median < TARGET_MILLISECONDS,
            "the median reload took {:.3} ms, over the {} ms target",
            median,
            TARGET_MILLISECONDS
        );
    }
}
