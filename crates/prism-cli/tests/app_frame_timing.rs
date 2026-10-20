//! The frame benchmark of the example apps (D9 and D10 of `reactive-app-verification`): the
//! frame, recomposition and allocation numbers of `prism run --frame-stats` on the real apps of
//! `examples/apps`, driven headless by generated event scripts, and the time from `prism`'s
//! start to the todo app's first frame, without the build cache and with a warm one.
//!
//! It is ignored by default, since a debug build or a busy machine says nothing about the
//! targets. Run it on a release build with
//!
//! ```text
//! cargo test --release -p prism-cli --test app_frame_timing -- --ignored --nocapture
//! ```
//!
//! Each measurement runs five times and reports the median of the runs' medians and the largest
//! of their 99th percentiles; the first frame runs twenty times each way and reports the
//! median. It fails on the targets only with `PRISM_ASSERT_TIMING=1`:
//!
//! | App and action | Measure | Target |
//! |---|---|---|
//! | counter, `click button 1` then `frame` | `frame` median, p99 | under 1 ms, under 2 ms |
//! | `large.prism`, `click checkbox 2`, `frame`, `pause 100` | `recomposition` median | under 1 ms |
//! | the same | `frame` p99 | under 4 ms |
//! | `large.prism`, `type "a"`, `frame`, `pause 100` | `recomposition` median | under 2 ms |
//! | the same | `frame` p99 | under 4 ms |
//! | both, a click on empty space then `frame` | allocations of those frames | zero |
//! | counter clicks and todo toggles | live allocations after each frame | constant |
//! | todo app, `main.prism` | first frame after start, cold and warm | under 50 ms, under 20 ms |
//! | `large.prism`, a first `frame`, then `resize 801 600`, `frame`, `resize 800 600`, `frame` | `layout` p99 | under 4 ms |
//! | the same | allocations of the frames after the first | zero |
//!
//! The resize session lays out the 1,000-item app in full at every frame, about 7,000 nodes,
//! every container through its measure policy written in Prism (`layout-primitive` D18), and
//! prints its `layout` median against the native layout's, measured before that change on the
//! machine that set the budget ([`NATIVE_RESIZE_LAYOUT`]). Since `layout-in-prism`, every
//! layout modifier is a measurement step written in Prism too, so the session also prints its
//! median against the one measured just before that change ([`POLICIES_RESIZE_LAYOUT`]), which
//! it should stay within 1.5 times of (`layout-in-prism` D9); the 4 ms p99 target is unchanged.
//!
//! The todo inputs are paced 100 ms apart, as a person's (D15 and D16 of `strong-skipping`): the
//! same work takes longer on a core that clocked down while the program slept, which is what a
//! person's input meets. Back-to-back toggles are still reported, without a target, to show it.

use std::path::{Path, PathBuf};
use std::process::Command;

const RUNS: usize = 5;
const FIRST_FRAME_RUNS: usize = 20;

/// The `layout` median of the resize session with the native layout pass, in milliseconds, on
/// the M4 Pro that measured `layout-primitive`'s baseline (D18).
const NATIVE_RESIZE_LAYOUT: f64 = 0.177;

/// The `layout` median of the resize session with the policies in Prism and the layout
/// modifiers native, in milliseconds, on the same machine, before `layout-in-prism` (D9).
const POLICIES_RESIZE_LAYOUT: f64 = 0.528;

/// The `examples/` workspace.
fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// The numbers of one `frame stats:` report.
#[derive(Debug, Default, Clone)]
struct Report {
    frames: u64,
    /// Median, 99th percentile and maximum of a stage, in milliseconds, by its name.
    stages: Vec<(String, [f64; 3])>,
    allocations_median: u64,
    allocations_max: u64,
    frames_without_allocations: u64,
    live: (u64, u64),
    first_frame_after_start: f64,
}

impl Report {
    fn stage(&self, name: &str) -> [f64; 3] {
        self.stages
            .iter()
            .find(|(stage, _)| stage == name)
            .unwrap_or_else(|| panic!("no {} line", name))
            .1
    }
}

/// The milliseconds of `"<number> ms"` at the start of `text`.
fn milliseconds(text: &str) -> f64 {
    text.trim()
        .trim_end_matches(',')
        .trim_end_matches(" ms")
        .parse()
        .unwrap_or_else(|_| panic!("not milliseconds: {:?}", text))
}

/// Parses the `frame stats:` lines of `stderr`.
fn parse(stderr: &str) -> Report {
    let mut report = Report::default();
    for line in stderr.lines() {
        let Some(line) = line.strip_prefix("frame stats: ") else {
            continue;
        };
        if let Some(rest) = line.strip_prefix("frames ") {
            report.frames = rest.split(',').next().unwrap().parse().unwrap();
            let after = rest.split(" ms after runApp, ").nth(1).unwrap();
            report.first_frame_after_start = milliseconds(after.trim_end_matches(" after start"));
        } else if let Some(rest) = line.strip_prefix("allocations per frame median ") {
            let mut words = rest.split(' ');
            report.allocations_median =
                words.next().unwrap().trim_end_matches(',').parse().unwrap();
            report.allocations_max = rest
                .split("max ")
                .nth(1)
                .unwrap()
                .split(',')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            report.frames_without_allocations = rest
                .split("none in ")
                .nth(1)
                .unwrap()
                .split(' ')
                .next()
                .unwrap()
                .parse()
                .unwrap();
        } else if let Some(rest) = line.strip_prefix("live allocations after each frame from ") {
            let (low, high) = rest.split_once(" to ").unwrap();
            report.live = (low.parse().unwrap(), high.parse().unwrap());
        } else if let Some((name, rest)) = line.split_once(" median ") {
            let p99 = rest
                .split("p99 ")
                .nth(1)
                .unwrap()
                .split(',')
                .next()
                .unwrap();
            let max = rest
                .split("max ")
                .nth(1)
                .unwrap()
                .split(" (")
                .next()
                .unwrap();
            let median = rest.split(',').next().unwrap();
            report.stages.push((
                name.to_string(),
                [milliseconds(median), milliseconds(p99), milliseconds(max)],
            ));
        }
    }
    report
}

/// Runs `prism run --frame-stats --headless [--events <script>] <entry>` from `examples/`,
/// with `cache` as the build cache, and gives its report.
fn run(entry: &str, script: Option<&Path>, cache: &std::ffi::OsStr) -> Report {
    let mut command = Command::new(env!("CARGO_BIN_EXE_prism"));
    command
        .env("PRISM_CACHE", cache)
        .args(["run", "--frame-stats", "--headless"]);
    if let Some(script) = script {
        command.arg("--events").arg(script);
    }
    let output = command
        .arg(entry)
        .current_dir(examples())
        .output()
        .expect("failed to run prism");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{}", stderr);
    parse(&stderr)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    values[values.len() / 2]
}

/// What `RUNS` runs of one script measured: the median of the medians and the largest p99 of
/// `stages`, and the runs' reports.
struct Measurement {
    reports: Vec<Report>,
}

impl Measurement {
    fn take(entry: &str, script: &Path, cache: &std::ffi::OsStr) -> Measurement {
        Measurement {
            reports: (0..RUNS).map(|_| run(entry, Some(script), cache)).collect(),
        }
    }

    fn median_of_medians(&self, stage: &str) -> f64 {
        let mut medians: Vec<f64> = self.reports.iter().map(|r| r.stage(stage)[0]).collect();
        median(&mut medians)
    }

    fn largest_p99(&self, stage: &str) -> f64 {
        self.reports
            .iter()
            .map(|r| r.stage(stage)[1])
            .fold(0.0, f64::max)
    }

    fn largest_max(&self, stage: &str) -> f64 {
        self.reports
            .iter()
            .map(|r| r.stage(stage)[2])
            .fold(0.0, f64::max)
    }

    fn print(&self, title: &str) {
        let first = &self.reports[0];
        println!("{} ({} frames, {} runs):", title, first.frames, RUNS);
        for (stage, _) in &first.stages {
            println!(
                "  {:<14} median {:>7.3} ms   p99 {:>7.3} ms   max {:>7.3} ms",
                stage,
                self.median_of_medians(stage),
                self.largest_p99(stage),
                self.largest_max(stage)
            );
        }
        println!(
            "  allocations per frame median {}, max {}, none in {} of {} frames; live after each frame from {} to {}",
            first.allocations_median,
            first.allocations_max,
            first.frames_without_allocations,
            first.frames,
            first.live.0,
            first.live.1
        );
    }

    /// Whether every run's live allocations stayed equal.
    fn constant_live(&self) -> bool {
        self.reports.iter().all(|r| r.live.0 == r.live.1)
    }
}

#[test]
#[ignore = "a benchmark: run it on a release build with --ignored --nocapture"]
fn the_example_apps_meet_their_frame_budgets() {
    let directory =
        std::env::temp_dir().join(format!("prism_app_frame_timing_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let cache = directory.join("cache");
    let script = |name: &str, text: String| {
        let path = directory.join(name);
        std::fs::write(&path, text).unwrap();
        path
    };
    let clicks = script("clicks.events", "click button 1\nframe\n".repeat(200));
    let toggles = script("toggles.events", "click checkbox 2\nframe\n".repeat(100));
    let paced_toggles = script(
        "paced-toggles.events",
        "click checkbox 2\nframe\npause 100\n".repeat(100),
    );
    let paced_typing = script(
        "paced-typing.events",
        format!(
            "click textfield\nframe\npause 100\n{}",
            "type \"a\"\nframe\npause 100\n".repeat(100)
        ),
    );
    // A point no node covers in either app: the counter's 320 × 200 window right of its
    // buttons, and the 1,000-item list's window right of the filter row.
    let counter_empty = script(
        "counter-empty.events",
        format!("frame\n{}", "click 300 190\nframe\n".repeat(50)),
    );
    let todo_empty = script(
        "todo-empty.events",
        format!("frame\n{}", "click 600 80\nframe\n".repeat(50)),
    );
    // A full relayout at every frame: a new width gives every row new constraints.
    let resizes = script(
        "resizes.events",
        format!(
            "frame\n{}",
            "resize 801 600\nframe\nresize 800 600\nframe\n".repeat(50)
        ),
    );
    let warm = cache.as_os_str();
    // The first runs fill the build cache.
    run("apps/counter/main.prism", None, warm);
    run("apps/todo/large.prism", None, warm);

    let counter = Measurement::take("apps/counter/main.prism", &clicks, warm);
    counter.print("counter, click button 1 then frame");
    let todo = Measurement::take("apps/todo/large.prism", &paced_toggles, warm);
    todo.print("todo of 1,000 items, click checkbox 2, frame, pause 100");
    let typing = Measurement::take("apps/todo/large.prism", &paced_typing, warm);
    typing.print("todo of 1,000 items, type \"a\", frame, pause 100");
    let back_to_back = Measurement::take("apps/todo/large.prism", &toggles, warm);
    back_to_back
        .print("todo of 1,000 items, click checkbox 2 then frame, back to back (no target)");
    let counter_idle = Measurement::take("apps/counter/main.prism", &counter_empty, warm);
    counter_idle.print("counter, a click on empty space then frame");
    let todo_idle = Measurement::take("apps/todo/large.prism", &todo_empty, warm);
    todo_idle.print("todo of 1,000 items, a click on empty space then frame");
    let resizing = Measurement::take("apps/todo/large.prism", &resizes, warm);
    resizing.print("todo of 1,000 items, resized between 800 and 801 wide, frame after each");
    let resize_layout = resizing.median_of_medians("layout");
    println!(
        "  resize layout median {:.3} ms, {:.2} x the native layout's {:.3} ms, {:.2} x the native \
         layout modifiers' {:.3} ms (budget 1.5 x)",
        resize_layout,
        resize_layout / NATIVE_RESIZE_LAYOUT,
        NATIVE_RESIZE_LAYOUT,
        resize_layout / POLICIES_RESIZE_LAYOUT,
        POLICIES_RESIZE_LAYOUT
    );

    let first_frames = |cache: &std::ffi::OsStr| {
        let mut times: Vec<f64> = (0..FIRST_FRAME_RUNS)
            .map(|_| run("apps/todo/main.prism", None, cache).first_frame_after_start)
            .collect();
        median(&mut times)
    };
    let cold = first_frames(std::ffi::OsStr::new("off"));
    run("apps/todo/main.prism", None, warm);
    let warm_first = first_frames(warm);
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "todo app's first frame after start, {} build, median of {}: {:.3} ms cold, {:.3} ms warm",
        build, FIRST_FRAME_RUNS, cold, warm_first
    );
    let _ = std::fs::remove_dir_all(&directory);

    // Every frame after the first of an empty-space session allocates nothing.
    let idle_frames_allocate_nothing = |measurement: &Measurement| {
        measurement
            .reports
            .iter()
            .all(|r| r.frames_without_allocations + 1 >= r.frames)
    };
    let targets = [
        (
            "counter frame median under 1 ms",
            counter.median_of_medians("frame") < 1.0,
        ),
        (
            "counter frame p99 under 2 ms",
            counter.largest_p99("frame") < 2.0,
        ),
        (
            "todo toggle recomposition median under 1 ms",
            todo.median_of_medians("recomposition") < 1.0,
        ),
        (
            "todo toggle frame p99 under 4 ms",
            todo.largest_p99("frame") < 4.0,
        ),
        (
            "todo typing recomposition median under 2 ms",
            typing.median_of_medians("recomposition") < 2.0,
        ),
        (
            "todo typing frame p99 under 4 ms",
            typing.largest_p99("frame") < 4.0,
        ),
        (
            "frames that change nothing allocate nothing",
            idle_frames_allocate_nothing(&counter_idle) && idle_frames_allocate_nothing(&todo_idle),
        ),
        (
            "live allocations constant over clicks and toggles",
            counter.constant_live() && todo.constant_live() && back_to_back.constant_live(),
        ),
        (
            "todo resize layout p99 under 4 ms",
            resizing.largest_p99("layout") < 4.0,
        ),
        (
            "todo resize frames after the first allocate nothing",
            idle_frames_allocate_nothing(&resizing),
        ),
        ("first frame under 50 ms cold", cold < 50.0),
        ("first frame under 20 ms warm", warm_first < 20.0),
    ];
    for (target, met) in &targets {
        println!(
            "  target: {}: {}",
            target,
            if *met { "met" } else { "MISSED" }
        );
    }
    if std::env::var_os("PRISM_ASSERT_TIMING").is_some_and(|value| value == "1") {
        let missed: Vec<&str> = targets
            .iter()
            .filter(|(_, met)| !met)
            .map(|(target, _)| *target)
            .collect();
        assert!(missed.is_empty(), "missed targets: {:?}", missed);
    }
}
