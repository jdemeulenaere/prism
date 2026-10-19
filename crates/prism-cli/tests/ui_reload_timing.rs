//! The UI reload benchmark of `prism run --watch` (the `ui` spec, "UI Reload Latency"; D13 of
//! `compose-hot-reload`): how long a one-line edit of the counter app's composable takes to
//! reach the frame that shows it, against the 50 ms target, from the write on disk to the dump
//! of that frame, file-system events and the debounce included.
//!
//! It is ignored by default, since a debug build or a busy machine says nothing about the
//! target. Run it on a release build with
//!
//! ```text
//! cargo test --release -p prism-cli --test ui_reload_timing -- --ignored --nocapture
//! ```
//!
//! It writes `ui-toolkit`'s counter app to a temporary workspace and runs `prism run --watch
//! --timings --headless --events events` on a temporary build cache, with a script of three
//! clicks and a dump, then [`EDITS`] rounds of `wait reload`, `frame` and `dump`. `wait reload`
//! writes out the dump before it, so each dump reaches the benchmark as the app starts waiting
//! again. Each round rewrites `main.prism` with a new label constant and measures from the write
//! to the dump showing it. It reports the median and 90th percentile, with the session's
//! reload stages and the reload pass of the frame loop, and fails on the median only when
//! `PRISM_ASSERT_TIMING=1` is set. A window adds rasterization, which this does not measure.

use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const EDITS: usize = 20;
const TARGET_MILLISECONDS: f64 = 50.0;

/// The counter app, whose text shows `label` before its count.
fn counter(label: &str) -> String {
    format!(
        "import /prism/compose/ui\nimport /prism/compose/foundation\nimport /prism/compose/layout\nimport /prism/compose/material3\n\
         compose fun Counter() {{\n    val c = remember {{ mutableStateOf(0) }}\n    \
         Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {{\n        \
         Text(\"{}: ${{c.value}}\")\n        Button(onClick = {{ c.value += 1 }}) {{ Text(\"+\") }}\n    }}\n}}\n\
         fun main() {{ runApp(width = 400, height = 300) {{ Counter() }} }}\n",
        label
    )
}

/// The value at `fraction` of the sorted `values`.
fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let index = ((values.len() - 1) as f64 * fraction).round() as usize;
    values[index]
}

/// The milliseconds a `timing: reload pass <ms> (<n> scopes)` line or a `timing: reload <ms>
/// (...)` line starts with.
fn milliseconds(line: &str, prefix: &str) -> f64 {
    let rest = line.strip_prefix(prefix).expect("a timing line");
    rest.split_once(" ms")
        .expect("milliseconds")
        .0
        .parse()
        .unwrap()
}

fn workspace() -> PathBuf {
    let root = std::env::temp_dir().join(format!("prism_ui_reload_timing_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.prism"), counter("Count")).unwrap();
    let mut events = String::from("click \"+\"\nclick \"+\"\nclick \"+\"\ndump\n");
    for _ in 0..EDITS {
        events.push_str("wait reload\nframe\ndump\n");
    }
    std::fs::write(root.join("events"), events).unwrap();
    root
}

/// Sends each line of `pipe`, with the instant it was read, to a new channel.
fn lines_of(pipe: impl std::io::Read + Send + 'static) -> mpsc::Receiver<(Instant, String)> {
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(pipe).lines() {
            let Ok(line) = line else { return };
            if sender.send((Instant::now(), line)).is_err() {
                return;
            }
        }
    });
    lines
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
/// leaves no session running.
struct KilledOnDrop(std::process::Child);

impl Drop for KilledOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "a benchmark: run it on a release build with --ignored --nocapture"]
fn ui_reload_latency() {
    let root = workspace();
    let cache = root.join(".cache");
    let mut child = KilledOnDrop(
        Command::new(env!("CARGO_BIN_EXE_prism"))
            .env("PRISM_CACHE", &cache)
            .args([
                "run",
                "--watch",
                "--timings",
                "--headless",
                "--events",
                "events",
                "main.prism",
            ])
            .current_dir(&root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to run prism"),
    );
    let output = lines_of(child.0.stdout.take().unwrap());
    let errors = lines_of(child.0.stderr.take().unwrap());
    // The first dump is written when the app starts waiting for the first reload.
    wait_for(&output, |line| line.contains("\"Count: 3\""));
    let mut totals = Vec::new();
    let mut reloads = Vec::new();
    let mut passes = Vec::new();
    for edit in 0..EDITS {
        let label = format!("Label{}", edit);
        let shown = format!("\"{}: 3\"", label);
        let written = Instant::now();
        std::fs::write(root.join("main.prism"), counter(&label)).unwrap();
        let (dumped, _) = wait_for(&output, |line| line.contains(&shown));
        totals.push(dumped.duration_since(written).as_secs_f64() * 1000.0);
        let (_, reload) = wait_for(&errors, |line| {
            line.starts_with("timing: reload ") && line.contains("detect")
        });
        reloads.push(milliseconds(&reload, "timing: reload "));
        let (_, pass) = wait_for(&errors, |line| line.starts_with("timing: reload pass "));
        passes.push(milliseconds(&pass, "timing: reload pass "));
    }
    drop(child);
    let _ = std::fs::remove_dir_all(&root);

    let median = percentile(&mut totals, 0.5);
    println!(
        "UI reload of the counter app, release build, {} edits: write to frame median {:.3} ms, p90 {:.3} ms",
        EDITS,
        median,
        percentile(&mut totals, 0.9)
    );
    println!(
        "  session reload (event to applied) median {:>8.3} ms  p90 {:>8.3} ms",
        percentile(&mut reloads, 0.5),
        percentile(&mut reloads, 0.9)
    );
    println!(
        "  reload pass and layout          median {:>8.3} ms  p90 {:>8.3} ms",
        percentile(&mut passes, 0.5),
        percentile(&mut passes, 0.9)
    );
    if std::env::var("PRISM_ASSERT_TIMING").as_deref() == Ok("1") {
        assert!(
            median < TARGET_MILLISECONDS,
            "the median UI reload took {:.3} ms, over the {} ms target",
            median,
            TARGET_MILLISECONDS
        );
    }
}
