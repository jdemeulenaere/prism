//! The example apps in real windows, driven without a person (D12 of
//! `reactive-app-verification`): the counter, the todo app, its 1,000-item screen and the
//! toolkit tour of `examples/` run as `prism run --check-allocations --frame-stats` runs them,
//! through `prism_cli::windows`, while a driver thread clicks, types, resizes and closes their
//! windows with `WindowSystem::deliver`, which takes the same input mapping and closing path as
//! the platform's events (D13 of `desktop-window`). Two more scenarios run the counter and the
//! todo app under `prism run --watch` and save the hot-reload edits of their sessions on disk,
//! in a copy of the apps, while the windows keep their state.
//!
//! Each scenario checks the app's final display list (what `--screenshot` writes), the exit
//! status and the allocation check, and prints the app's `frame stats:` lines. The 1,000-item
//! scenario toggles 20 visible items and checks that the `input to present` 99th percentile is
//! under 16 ms, failing on it only with `PRISM_ASSERT_TIMING=1`, and runs the same toggles
//! headless in the same process as a control (`large-headless`); the counter and todo scenarios also leave their windows idle for 10 s and check
//! that the process used no CPU meanwhile.
//!
//! It opens windows, so it has its own `main` (`harness = false`) and runs only with
//! `PRISM_WINDOW_TESTS=1` on a machine with a display; otherwise it prints why it is skipped
//! and passes (`PRISM_APP_WINDOWS_ONLY=<scenario>` runs one scenario):
//!
//! ```text
//! PRISM_WINDOW_TESTS=1 cargo test --release -p prism-cli --test app_windows
//! ```
//!
//! With `PRISM_UPDATE_SCREENSHOTS=1`, the final frames of the counter, the todo app and the tour
//! are copied to `docs/images/counter.png`, `todo.png` and `tour.png`, which `README.md` shows. What it
//! cannot check, the platform's own mouse and keyboard events, key repeat as the keyboard sends
//! it, sharpness at a Retina scale and moving between displays, is left to a person (the change
//! design's "Manual verification").

use prism_desktop::input::{ElementState, KeyValue as Key, MouseButton, NamedKey, WindowInput};
use prism_desktop::{MainThreadServer, WindowSystem};
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// Counts the program thread's allocations for `--frame-stats`, as the `prism` binary does.
#[global_allocator]
static ALLOCATOR: prism_runtime::allocation_counter::CountingAllocator =
    prism_runtime::allocation_counter::CountingAllocator;

const SKIPPED: &str =
    "app window test skipped: set PRISM_WINDOW_TESTS=1 on a machine with a display";

fn display_available() -> bool {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return true;
    }
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// --- The child: an app in a window, and the driver

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let start = Instant::now();
    while !condition() {
        if start.elapsed() > Duration::from_secs(20) {
            fail(&format!("timed out waiting until {}", what));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn fail(message: &str) -> ! {
    eprintln!("app window driver: {}", message);
    std::process::exit(2);
}

struct Driver {
    windows: WindowSystem,
    scale: f64,
}

impl Driver {
    fn statistics(&self) -> Arc<prism_desktop::statistics::WindowStatistics> {
        self.windows.statistics()
    }

    /// Waits until the `n`-th window is open and has presented its first frame, and checks its
    /// title and size.
    fn opened(&mut self, n: u64, title: &str, size: (f64, f64)) {
        let statistics = self.statistics();
        wait_until(&format!("window {} opens", n), || {
            statistics.windows_opened() >= n
        });
        wait_until(&format!("window {} presents a frame", n), || {
            statistics.new_frames_presented() > 0
        });
        let window = statistics.last_window().expect("a window report");
        if window.title != title || window.logical_size != size {
            fail(&format!(
                "window {} is {:?} of {:?}, not {:?} of {:?}",
                n, window.title, window.logical_size, title, size
            ));
        }
        self.scale = window.scale_factor;
        eprintln!(
            "app window driver: {} opened at scale {}",
            title, window.scale_factor
        );
        self.settle();
    }

    /// Waits until the window has drawn every frame asked for so far.
    fn settle(&self) {
        let statistics = self.statistics();
        let counts = || {
            (
                statistics.frames_received(),
                statistics.frames_presented(),
                statistics.windows_closed(),
            )
        };
        let start = Instant::now();
        loop {
            let before = counts();
            std::thread::sleep(Duration::from_millis(100));
            if counts() == before {
                return;
            }
            if start.elapsed() > Duration::from_secs(20) {
                fail("the window keeps drawing");
            }
        }
    }

    fn deliver(&self, input: WindowInput) {
        if !self.windows.deliver(input.clone()) {
            fail(&format!("no window is open for {:?}", input));
        }
    }

    fn button(&self, at: (f64, f64), state: ElementState) {
        self.deliver(WindowInput::CursorMoved {
            x: at.0 * self.scale,
            y: at.1 * self.scale,
        });
        self.deliver(WindowInput::MouseButton {
            button: MouseButton::Left,
            state,
        });
    }

    /// A click at logical `at`, then the frame it causes.
    fn click(&self, at: (f64, f64)) {
        self.button(at, ElementState::Pressed);
        self.button(at, ElementState::Released);
        self.settle();
    }

    fn type_text(&self, text: &str) {
        for character in text.chars() {
            let text = character.to_string();
            self.deliver(WindowInput::KeyPressed {
                key: Key::Character(text.as_str().into()),
                text: Some(text.as_str().into()),
            });
        }
        self.settle();
    }

    fn backspace(&self) {
        self.deliver(WindowInput::KeyPressed {
            key: Key::Named(NamedKey::Backspace),
            text: Some("\u{8}".into()),
        });
        self.settle();
    }

    fn resize(&self, width: u32, height: u32) {
        let statistics = self.statistics();
        let received = statistics.frames_received();
        if !self.windows.request_inner_size(width, height) {
            fail("no window to resize");
        }
        wait_until("the resized app presents a frame", || {
            statistics.frames_received() > received
        });
        self.settle();
    }

    /// Leaves the window idle for 10 s, between two lines the parent measures the process's
    /// CPU time at.
    fn idle(&self) {
        eprintln!("app window driver: idle start");
        std::thread::sleep(Duration::from_secs(10));
        eprintln!("app window driver: idle end");
    }

    fn close(&self) {
        let statistics = self.statistics();
        let closed = statistics.windows_closed();
        self.deliver(WindowInput::CloseRequested);
        wait_until("the window closes", || statistics.windows_closed() > closed);
    }
}

/// Points of the apps' windows, in logical pixels, with the window's text metrics: the
/// counter's buttons are at 16,16 and 16,64, the todo app's field at 16,18 with `Add` at its
/// right, its filters at y 64 to 104 and its rows from y 112, 48 apart, each a checkbox at its
/// start and `Remove` at its end.
const COUNT_BUTTON: (f64, f64) = (40.0, 36.0);
const RESET_BUTTON: (f64, f64) = (40.0, 84.0);
const FIELD: (f64, f64) = (100.0, 36.0);
const ADD: (f64, f64) = (440.0, 36.0);
const ALL: (f64, f64) = (36.0, 84.0);
const ACTIVE: (f64, f64) = (130.0, 84.0);
const DONE: (f64, f64) = (230.0, 84.0);

fn checkbox(row: usize) -> (f64, f64) {
    (26.0, 132.0 + 48.0 * row as f64)
}

fn remove(row: usize, width: f64) -> (f64, f64) {
    (width - 40.0, 132.0 + 48.0 * row as f64)
}

/// Adds `titles` to the todo app through its field and `Add`.
fn add(driver: &Driver, titles: &[&str]) {
    for title in titles {
        driver.click(FIELD);
        driver.type_text(title);
        driver.click(ADD);
    }
}

/// What each scenario runs and what its driver does.
fn scenario(name: &str) -> (&'static str, Box<dyn FnOnce(Driver) + Send>) {
    match name {
        // 8.1: clicks count, a press released outside does not, Reset resets and is disabled
        // at zero; three clicks are left for the screenshot.
        "counter" => (
            "examples/apps/counter/main.prism",
            Box::new(|mut driver: Driver| {
                driver.opened(1, "Counter", (320.0, 200.0));
                driver.click(RESET_BUTTON);
                driver.click(COUNT_BUTTON);
                driver.click(COUNT_BUTTON);
                driver.button(COUNT_BUTTON, ElementState::Pressed);
                driver.button((300.0, 180.0), ElementState::Released);
                driver.settle();
                driver.click(RESET_BUTTON);
                for _ in 0..3 {
                    driver.click(COUNT_BUTTON);
                }
                driver.idle();
                driver.close();
            }),
        ),
        // 8.2: typing with Backspace, adding, toggling, removing, filtering and a live resize;
        // three items with the second checked are left for the screenshot.
        "todo" => (
            "examples/apps/todo/main.prism",
            Box::new(|mut driver: Driver| {
                driver.opened(1, "Todo", (480.0, 640.0));
                driver.click(FIELD);
                driver.type_text("Milx");
                driver.backspace();
                driver.type_text("k");
                driver.click(ADD);
                add(&driver, &["Eggs", "Jam", "Bread"]);
                driver.click(remove(2, 480.0));
                driver.click(checkbox(1));
                driver.click(ACTIVE);
                driver.click(DONE);
                driver.click(ALL);
                driver.resize(640, 480);
                driver.resize(480, 640);
                driver.idle();
                driver.close();
            }),
        ),
        // 8.4: toggling 20 visible items of the 1,000-item list in an 800 × 600 window.
        "large" => (
            "examples/apps/todo/large.prism",
            Box::new(|mut driver: Driver| {
                driver.opened(1, "Todo: 1,000 items", (800.0, 600.0));
                for row in 0..10 {
                    driver.click(checkbox(row));
                    driver.click(checkbox(row));
                }
                driver.close();
            }),
        ),
        // 8.5: the tour, its counter, checkbox, field and toggleable text used once.
        "tour" => (
            "examples/demo/ui/ui.prism",
            Box::new(|mut driver: Driver| {
                driver.opened(1, "Prism UI tour", (640.0, 800.0));
                driver.click((80.0, 84.0));
                driver.click((26.0, 126.0));
                driver.click((200.0, 158.0));
                driver.type_text("Prism");
                driver.click((40.0, 222.0));
                driver.close();
            }),
        ),
        _ => fail(&format!("no scenario {}", name)),
    }
}

/// The watch scenarios (8.3): the app runs from a copy of `examples/` under `prism run
/// --watch`; after some input the driver saves the edit of the app's hot-reload session on
/// disk, waits for the reloaded frame, which arrives without input, and closes the window. The
/// parent ends the session once the driver is done.
fn watch_scenario(name: &str, workspace: &Path) -> (PathBuf, Box<dyn FnOnce(Driver) + Send>) {
    let examples = repository().join("examples");
    let copy = |relative: &str| {
        let target = workspace.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(examples.join(relative), &target).unwrap();
    };
    let sources = examples.clone();
    let edit = move |driver: &Driver, target: PathBuf, replacement: &str| {
        let statistics = driver.statistics();
        let received = statistics.frames_received();
        std::fs::write(
            &target,
            std::fs::read_to_string(sources.join(replacement)).unwrap(),
        )
        .unwrap();
        wait_until("the reloaded app presents a frame without input", || {
            statistics.frames_received() > received
        });
        driver.settle();
    };
    match name {
        "watch-counter" => {
            copy("apps/counter/main.prism");
            let target = workspace.join("apps/counter/main.prism");
            (
                target.clone(),
                Box::new(move |mut driver: Driver| {
                    driver.opened(1, "Counter", (320.0, 200.0));
                    for _ in 0..3 {
                        driver.click(COUNT_BUTTON);
                    }
                    edit(
                        &driver,
                        target,
                        "apps/counter/sessions/relabelling-keeps-the-count.main.prism.v2",
                    );
                    driver.close();
                    eprintln!("app window driver: done");
                }),
            )
        }
        "watch-todo" => {
            for file in ["model", "screen", "main"] {
                copy(&format!("apps/todo/{}.prism", file));
            }
            let target = workspace.join("apps/todo/screen.prism");
            (
                workspace.join("apps/todo/main.prism"),
                Box::new(move |mut driver: Driver| {
                    driver.opened(1, "Todo", (480.0, 640.0));
                    add(&driver, &["Milk", "Eggs", "Bread"]);
                    driver.click(checkbox(1));
                    driver.click(FIELD);
                    driver.type_text("Jam");
                    edit(
                        &driver,
                        target,
                        "apps/todo/sessions/editing-the-row-layout-keeps-the-list.screen.prism.v2",
                    );
                    driver.type_text("s");
                    driver.close();
                    eprintln!("app window driver: done");
                }),
            )
        }
        _ => fail(&format!("no watch scenario {}", name)),
    }
}

fn child(name: &str, directory: &Path) -> i32 {
    prism_runtime::allocation_counter::enable();
    let (server, windows) = MainThreadServer::new();
    let driver = Driver {
        windows: windows.clone(),
        scale: 1.0,
    };
    // The final frame, as `--screenshot` writes it, and its display list's dump.
    let screenshot = directory.join(format!("{}.png", name));
    let dump = directory.join(format!("{}.draw", name));
    prism_ui::host::set_final_frame_sink(Some(Box::new(move |list, size, scale| {
        prism_desktop::screenshot::write(&screenshot, list, size, scale).unwrap();
        let mut text = String::new();
        prism_ui::dump::dump_display_list(list, &mut text);
        std::fs::write(&dump, text).unwrap();
    })));
    // The 1,000-item list is timed as task 8.4 runs it, without the allocation check, whose
    // map would dominate its frames (D7).
    let options = prism_cli::RunOptions {
        check_allocations: !name.starts_with("large"),
        frame_stats: true,
        ..prism_cli::RunOptions::default()
    };
    if name.starts_with("watch-") {
        let (entry, drive) = watch_scenario(name, &directory.join("workspace"));
        std::thread::spawn(move || drive(driver));
        return prism_cli::windows::watch_with_windows(&entry, options, None, server, windows);
    }
    // The control of `large`: the same toggles headless in the same process, so that the window's
    // numbers can be told apart from the machine's.
    if name == "large-headless" {
        let commands =
            prism_ui::script::parse_script(&"click checkbox 2\nframe\n".repeat(20)).unwrap();
        let options = prism_cli::RunOptions {
            headless: Some(prism_cli::HeadlessOptions {
                size: None,
                commands,
            }),
            ..options
        };
        return prism_cli::run_file(
            &repository().join("examples/apps/todo/large.prism"),
            &options,
        )
        .unwrap();
    }
    let (entry, drive) = scenario(name);
    std::thread::spawn(move || drive(driver));
    prism_cli::windows::run_with_windows(&repository().join(entry), &options, server, windows)
}

// --- The parent: runs each scenario in a child and checks what it did

struct Outcome {
    status: Option<i32>,
    stdout: String,
    stderr: Vec<String>,
    /// The process's CPU seconds over its idle window, if it had one.
    idle_cpu: Option<f64>,
    draw: String,
}

/// The CPU time `ps` reports for `pid`, in seconds.
fn cpu_seconds(pid: u32) -> f64 {
    let output = Command::new("ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
        .expect("ps runs");
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // [[dd-]hh:]mm:ss.cc
    text.split(':').fold(0.0, |total, part| {
        total * 60.0 + part.parse::<f64>().unwrap_or(0.0)
    })
}

fn run(name: &str) -> Outcome {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("prism_app_windows_{}_{}", name, nanos));
    std::fs::create_dir_all(&directory).unwrap();
    let mut child: Child = Command::new(std::env::current_exe().unwrap())
        .env("PRISM_APP_WINDOWS_SCENARIO", name)
        .env("PRISM_APP_WINDOWS_DIRECTORY", &directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the scenario's process starts");
    let pid = child.id();
    let (sender, lines) = mpsc::channel();
    let stderr = child.stderr.take().unwrap();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr).lines() {
            let Ok(line) = line else { return };
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    let watching = name.starts_with("watch-");
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut stderr = Vec::new();
    let mut idle_start = None;
    let mut idle_cpu = None;
    let status = loop {
        while let Ok(line) = lines.recv_timeout(Duration::from_millis(20)) {
            if line == "app window driver: idle start" {
                idle_start = Some(cpu_seconds(pid));
            } else if line == "app window driver: idle end" {
                idle_cpu = idle_start.map(|start| cpu_seconds(pid) - start);
            }
            let done = watching && line == "app window driver: done";
            stderr.push(line);
            if done {
                // The run has ended; the session waits for changes until it is killed.
                std::thread::sleep(Duration::from_millis(500));
                let _ = child.kill();
            }
        }
        if let Some(status) = child.try_wait().unwrap() {
            break status.code();
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            break None;
        }
    };
    while let Ok(line) = lines.recv_timeout(Duration::from_millis(200)) {
        stderr.push(line);
    }
    let mut stdout = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    let draw =
        std::fs::read_to_string(directory.join(format!("{}.draw", name))).unwrap_or_default();
    if std::env::var_os("PRISM_UPDATE_SCREENSHOTS").is_some_and(|value| value == "1")
        && matches!(name, "counter" | "todo" | "tour")
    {
        let images = repository().join("docs/images");
        std::fs::create_dir_all(&images).unwrap();
        std::fs::copy(
            directory.join(format!("{}.png", name)),
            images.join(format!("{}.png", name)),
        )
        .unwrap();
    }
    let _ = std::fs::remove_dir_all(&directory);
    Outcome {
        status,
        stdout,
        stderr,
        idle_cpu,
        draw,
    }
}

/// Checks `condition`, collecting a failure.
fn check(failures: &mut Vec<String>, scenario: &str, condition: bool, message: String) {
    if !condition {
        failures.push(format!("{}: {}", scenario, message));
    }
}

/// The 99th percentile of the `input to present` line, in milliseconds.
fn input_to_present_p99(stderr: &[String]) -> Option<f64> {
    let line = stderr
        .iter()
        .find(|line| line.starts_with("frame stats: input to present median "))?;
    let p99 = line.split("p99 ").nth(1)?.split(" ms").next()?;
    p99.parse().ok()
}

fn parent() -> Result<(), String> {
    let mut failures = Vec::new();
    let only = std::env::var("PRISM_APP_WINDOWS_ONLY").ok();
    for name in [
        "counter",
        "todo",
        "large",
        "tour",
        "watch-counter",
        "watch-todo",
        "large-headless",
    ] {
        if only.as_deref().is_some_and(|only| only != name) {
            continue;
        }
        let outcome = run(name);
        println!("{}: exit status {:?}", name, outcome.status);
        for line in &outcome.stderr {
            println!("  {}", line);
        }
        if let Some(cpu) = outcome.idle_cpu {
            println!("  CPU time over 10 s idle: {:.2} s", cpu);
        }
        let watching = name.starts_with("watch-");
        if !watching {
            check(
                &mut failures,
                name,
                outcome.status == Some(0),
                format!("exit status {:?}", outcome.status),
            );
        }
        check(
            &mut failures,
            name,
            !outcome
                .stderr
                .iter()
                .any(|line| line.starts_with("allocation check") || line.starts_with("panic")),
            "the allocation check or a runtime error reported".to_string(),
        );
        check(
            &mut failures,
            name,
            name == "large-headless"
                || outcome
                    .stderr
                    .iter()
                    .any(|line| line.starts_with("frame stats: raster median ")),
            "no window lines in the frame statistics".to_string(),
        );
        check(
            &mut failures,
            name,
            outcome.stdout.is_empty(),
            format!("stdout {:?}", outcome.stdout),
        );
        if let Some(cpu) = outcome.idle_cpu {
            check(
                &mut failures,
                name,
                cpu < 0.05,
                format!("{:.2} s of CPU time over 10 s idle", cpu),
            );
        }
        let shows = |text: &str| outcome.draw.contains(&format!("\"{}\"", text));
        match name {
            "counter" => check(
                &mut failures,
                name,
                shows("Count: 3") && shows("Reset"),
                format!("the final frame:\n{}", outcome.draw),
            ),
            "todo" => check(
                &mut failures,
                name,
                shows("Milk")
                    && shows("Eggs")
                    && shows("Bread")
                    && !shows("Jam")
                    && shows("2 left"),
                format!("the final frame:\n{}", outcome.draw),
            ),
            "large" => {
                // A time, so it fails the test only under `PRISM_ASSERT_TIMING=1`, as the
                // benchmarks do: a window that is not visible, behind a locked screen for
                // instance, is throttled by the system.
                let p99 = input_to_present_p99(&outcome.stderr);
                let met = p99.is_some_and(|p99| p99 < 16.0);
                println!(
                    "  target: input to present p99 under 16 ms: {}",
                    if met { "met" } else { "MISSED" }
                );
                check(
                    &mut failures,
                    name,
                    met || std::env::var_os("PRISM_ASSERT_TIMING").is_none_or(|value| value != "1"),
                    format!("input to present p99 {:?}, target under 16 ms", p99),
                );
                check(
                    &mut failures,
                    name,
                    shows("667 left"),
                    "the toggles did not cancel out".to_string(),
                );
            }
            "tour" => check(
                &mut failures,
                name,
                shows("Prism UI tour"),
                format!("the final frame:\n{}", outcome.draw),
            ),
            "watch-counter" => check(
                &mut failures,
                name,
                shows("Clicks: 3")
                    && outcome
                        .stderr
                        .iter()
                        .any(|line| line == "reload: applied apps/counter/main.prism"),
                format!("the final frame:\n{}", outcome.draw),
            ),
            "watch-todo" => check(
                &mut failures,
                name,
                shows("Milk") && shows("Eggs") && shows("Bread") && shows("Jams"),
                format!("the final frame:\n{}", outcome.draw),
            ),
            _ => {}
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn main() {
    if let Ok(name) = std::env::var("PRISM_APP_WINDOWS_SCENARIO") {
        let directory = PathBuf::from(std::env::var("PRISM_APP_WINDOWS_DIRECTORY").unwrap());
        std::process::exit(child(&name, &directory));
    }
    if std::env::var("PRISM_WINDOW_TESTS").as_deref() != Ok("1") || !display_available() {
        println!("{}", SKIPPED);
        return;
    }
    if let Err(message) = parent() {
        eprintln!("app window test failed:\n{}", message);
        std::process::exit(1);
    }
    println!("app window test passed");
}
