//! Real windows driven without a person (D13, D14 and D17 of `desktop-window`): Prism programs
//! run as `prism run` and `prism run --watch` run them with the `window` feature, through
//! `prism_cli::windows`, while a driver thread delivers clicks, key presses and closes to their
//! window with `WindowSystem::deliver`. The window handles those as it handles the platform's
//! events, through the same input mapping and closing code, and resizes go through the
//! platform. What is left for a person is what only real hardware shows: the platform's own
//! mouse and keyboard events, sharpness on a Retina display and moving between displays (the
//! "Manual verification" section of the change's design).
//!
//! It opens windows, so it has its own `main` (`harness = false`) and runs only with
//! `PRISM_WINDOW_TESTS=1` and, outside macOS and Windows, `DISPLAY` or `WAYLAND_DISPLAY`;
//! otherwise it prints why it is skipped and passes:
//!
//! ```text
//! PRISM_WINDOW_TESTS=1 cargo test -p prism-cli --test window_input
//! ```
//!
//! The `watch-reload` scenario covers `compose-hot-reload`: a saved edit wakes an idle window,
//! which presents the reloaded app's new text without input, and keeps its state.
//!
//! Each scenario runs in a child process (this executable, with `PRISM_WINDOW_INPUT_SCENARIO`
//! set), whose main thread serves the windows and whose stdout, stderr and status the parent
//! checks; a child that hangs is killed after 60 seconds.

use prism_desktop::input::{
    ElementState, KeyValue as Key, ModifiersState, MouseButton, NamedKey, WindowInput,
};
use prism_desktop::screenshot::Pixmap;
use prism_desktop::{MainThreadServer, WindowSystem};
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const SKIPPED: &str =
    "window input test skipped: set PRISM_WINDOW_TESTS=1 on a machine with a display";

const IMPORTS: &str = "import /prism/compose/ui\nimport /prism/compose/foundation\nimport /prism/compose/layout\nimport /prism/compose/material3\n";

/// The `ui` spec's counter screen, printing each count.
const COUNTER: &str = "import /prism/core\nimport /prism/compose/runtime\ncompose fun Counter() {
    val c = remember { mutableStateOf(0) }
    Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(\"Count: ${c.value}\")
        Button(onClick = { c.value += 1; println(\"count ${c.value}\") }) { Text(\"+\") }
    }
}
";

/// A point inside the counter's `+` button, in logical pixels: the button is at 16,43 and at
/// least 58 × 40 with the window's metrics (at 16,44 headless).
const PLUS: (f64, f64) = (40.0, 60.0);

fn display_available() -> bool {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return true;
    }
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

// --- The child: a program with windows, and the driver

/// Waits up to 10 s for `condition`.
fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let start = Instant::now();
    while !condition() {
        if start.elapsed() > Duration::from_secs(10) {
            fail(&format!("timed out waiting until {}", what));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn fail(message: &str) -> ! {
    eprintln!("window input driver: {}", message);
    std::process::exit(2);
}

/// The driver's view of the window system.
struct Driver {
    windows: WindowSystem,
}

impl Driver {
    fn statistics(&self) -> std::sync::Arc<prism_desktop::statistics::WindowStatistics> {
        self.windows.statistics()
    }

    /// Waits until the `n`-th window of the process is open and has presented a frame, checks
    /// its title, and gives its scale factor.
    fn opened(&self, n: u64, title: &str, size: (f64, f64)) -> f64 {
        let statistics = self.statistics();
        wait_until(&format!("window {} opens", n), || {
            statistics.windows_opened() >= n
        });
        let presented = statistics.new_frames_presented();
        wait_until(&format!("window {} presents its first frame", n), || {
            statistics.new_frames_presented() > presented || statistics.frames_presented() > 0
        });
        let window = statistics.last_window().expect("a window report");
        if window.title != title || window.logical_size != size {
            fail(&format!(
                "window {} is {:?} of {:?} logical pixels, not {:?} of {:?}",
                n, window.title, window.logical_size, title, size
            ));
        }
        self.settle();
        window.scale_factor
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
            std::thread::sleep(Duration::from_millis(150));
            if counts() == before {
                return;
            }
            if start.elapsed() > Duration::from_secs(10) {
                fail("the window keeps drawing");
            }
        }
    }

    fn deliver(&self, input: WindowInput) {
        if !self.windows.deliver(input.clone()) {
            fail(&format!("no window is open for {:?}", input));
        }
    }

    /// Moves the cursor to logical `at` and presses the left button there.
    fn press(&self, at: (f64, f64), scale: f64) {
        self.deliver(WindowInput::CursorMoved {
            x: at.0 * scale,
            y: at.1 * scale,
        });
        self.deliver(WindowInput::MouseButton {
            button: MouseButton::Left,
            state: ElementState::Pressed,
        });
    }

    /// Moves the cursor to logical `at` and releases the left button there.
    fn release(&self, at: (f64, f64), scale: f64) {
        self.deliver(WindowInput::CursorMoved {
            x: at.0 * scale,
            y: at.1 * scale,
        });
        self.deliver(WindowInput::MouseButton {
            button: MouseButton::Left,
            state: ElementState::Released,
        });
    }

    fn click(&self, at: (f64, f64), scale: f64) {
        self.press(at, scale);
        self.release(at, scale);
    }

    /// A key press with `modifiers` held, then their release.
    fn key(&self, modifiers: ModifiersState, key: Key, text: Option<&str>) {
        if !modifiers.is_empty() {
            self.deliver(WindowInput::ModifiersChanged(modifiers));
        }
        self.deliver(WindowInput::KeyPressed {
            key,
            text: text.map(Into::into),
        });
        // A key that closes the window (Cmd+W) may leave no window to release the modifiers in,
        // so their release is delivered only if one is still open.
        if !modifiers.is_empty() {
            let _ = self
                .windows
                .deliver(WindowInput::ModifiersChanged(ModifiersState::empty()));
        }
    }

    fn character(&self, modifiers: ModifiersState, character: &str, text: &str) {
        self.key(modifiers, Key::Character(character.into()), Some(text));
    }

    /// Closes the window by its close button, and waits until it is gone.
    fn close(&self) {
        let closed = self.statistics().windows_closed();
        self.deliver(WindowInput::CloseRequested);
        self.closed(closed + 1);
    }

    /// Waits until `n` windows have closed.
    fn closed(&self, n: u64) {
        let statistics = self.statistics();
        wait_until(&format!("{} windows have closed", n), || {
            statistics.windows_closed() >= n
        });
    }
}

/// The scenarios, each a program and what the driver does with its window.
fn scenario(name: &str) -> (String, Box<dyn FnOnce(Driver) + Send>) {
    let counter_app = |after: &str| {
        format!(
            "import /prism/core\n{}{}fun main() {{ runApp(title = \"Counter\", width = 400, height = 300) {{ Counter() }}; println(\"closed\"){} }}\n",
            IMPORTS, COUNTER, after
        )
    };
    match name {
        // 8.1 and 8.2: clicks count, a press released elsewhere does not, a live resize relays
        // out, and Cmd+W (the close button elsewhere) ends `runApp`.
        "counter" => (
            counter_app(""),
            Box::new(|driver: Driver| {
                let scale = driver.opened(1, "Counter", (400.0, 300.0));
                driver.click(PLUS, scale);
                driver.click(PLUS, scale);
                driver.press(PLUS, scale);
                driver.release((300.0, 250.0), scale);
                driver.settle();
                let received = driver.statistics().frames_received();
                if !driver.windows.request_inner_size(300, 200) {
                    fail("no window to resize");
                }
                let statistics = driver.statistics();
                wait_until("the resized app presents a frame", || {
                    statistics.frames_received() > received
                });
                driver.settle();
                let closed = statistics.windows_closed();
                if cfg!(target_os = "macos") {
                    driver.character(ModifiersState::SUPER, "w", "w");
                } else {
                    driver.deliver(WindowInput::CloseRequested);
                }
                driver.closed(closed + 1);
            }),
        ),
        // 8.2: the close button.
        "close-button" => (counter_app(""), Box::new(|driver: Driver| {
            driver.opened(1, "Counter", (400.0, 300.0));
            driver.close();
        })),
        // 8.2: Cmd+Q closes the window and does not end the program.
        "cmd-q" => (
            counter_app("; println(\"after\")"),
            Box::new(|driver: Driver| {
                driver.opened(1, "Counter", (400.0, 300.0));
                driver.character(ModifiersState::SUPER, "q", "q");
                driver.closed(1);
            }),
        ),
        // 8.3: a second window opens after the first closes, on the same event loop.
        "two-apps" => (
            format!(
                "import /prism/core\n{}fun main() {{ runApp(title = \"One\", width = 200, height = 100) {{ Text(\"1\") }}; println(\"one closed\"); runApp(title = \"Two\", width = 220, height = 120) {{ Text(\"2\") }}; println(\"two closed\") }}\n",
                IMPORTS
            ),
            Box::new(|driver: Driver| {
                driver.opened(1, "One", (200.0, 100.0));
                driver.close();
                driver.opened(2, "Two", (220.0, 120.0));
                driver.close();
                if driver.statistics().event_loops_created() != 1 {
                    fail("the second window did not reuse the event loop");
                }
            }),
        ),
        // 8.4: typing, Backspace, key repeat, and shortcuts and arrows that type nothing.
        "text-field" => (
            format!(
                "import /prism/core\nimport /prism/compose/runtime\n{}compose fun Form() {{
    val name = remember {{ mutableStateOf(\"\") }}
    Column {{
        TextField(value = name.value, onValueChange = {{ println(\"value ${{it}}\"); name.value = it }})
        Text(\"Hello ${{name.value}}\")
    }}
}}
fun main() {{ runApp(title = \"Form\", width = 400, height = 300) {{ Form() }}; println(\"closed\") }}
",
                IMPORTS
            ),
            Box::new(|driver: Driver| {
                let scale = driver.opened(1, "Form", (400.0, 300.0));
                driver.click((100.0, 18.0), scale);
                let none = ModifiersState::empty();
                driver.character(none, "a", "a");
                driver.character(ModifiersState::SHIFT, "B", "B");
                driver.key(none, Key::Named(NamedKey::Backspace), Some("\u{8}"));
                driver.character(ModifiersState::CONTROL, "c", "\u{3}");
                driver.character(ModifiersState::SUPER, "a", "a");
                driver.key(none, Key::Named(NamedKey::ArrowLeft), None);
                driver.key(none, Key::Named(NamedKey::Enter), Some("\r"));
                // A held key: the press and its repeats.
                for _ in 0..3 {
                    driver.character(none, "x", "x");
                }
                driver.settle();
                driver.close();
            }),
        ),
        // 8.6: a runtime error in a handler ends the run with status 101 and its window.
        "runtime-error" => (
            format!(
                "import /prism/core\n{}fun zero(): Int = 0
compose fun Divider() {{ Button(onClick = {{ println(1 / zero()) }}) {{ Text(\"/\") }} }}
fun main() {{ runApp(title = \"Divider\", width = 200, height = 100) {{ Divider() }}; println(\"closed\") }}
",
                IMPORTS
            ),
            Box::new(|driver: Driver| {
                let scale = driver.opened(1, "Divider", (200.0, 100.0));
                driver.click((20.0, 20.0), scale);
            }),
        ),
        _ => fail(&format!("no scenario {}", name)),
    }
}

/// The watch scenario (8.6): the counter under `prism run --watch`; closing its window ends
/// the run, and an edit runs `main` again in a new window, whose handler then divides by zero,
/// which stops the program and closes its window. The driver says when it is done, and the
/// parent ends the session.
fn watch_driver(directory: PathBuf) -> Box<dyn FnOnce(Driver) + Send> {
    Box::new(move |driver: Driver| {
        let scale = driver.opened(1, "Counter", (400.0, 300.0));
        driver.click(PLUS, scale);
        driver.close();
        std::fs::write(
            directory.join("main.prism"),
            format!(
                "import /prism/core\n{}fun zero(): Int = 0\ncompose fun Counter() {{ Column(modifier = Modifier.padding(16.dp)) {{ Text(\"Count\"); Button(onClick = {{ println(1 / zero()) }}) {{ Text(\"+\") }} }} }}\nfun main() {{ runApp(title = \"Counter\", width = 400, height = 300) {{ Counter() }}; println(\"closed\") }}\n",
                IMPORTS
            ),
        )
        .unwrap();
        let scale = driver.opened(2, "Counter", (400.0, 300.0));
        driver.click(PLUS, scale);
        driver.closed(2);
        eprintln!("window input driver: done");
    })
}

/// The hot reload scenario (`compose-hot-reload` D2 and D9): the counter under `prism run
/// --watch`; after a click, saving an edit of its label and handler wakes the idle window,
/// which presents the new label without any input, and the next click runs the new handler on
/// the kept count. Closing the window then ends the run.
fn reload_driver(directory: PathBuf) -> Box<dyn FnOnce(Driver) + Send> {
    Box::new(move |driver: Driver| {
        let statistics = driver.statistics();
        statistics.record_presented_text();
        let scale = driver.opened(1, "Counter", (400.0, 300.0));
        driver.click(PLUS, scale);
        driver.settle();
        if !statistics.presented_text().contains("Count: 1") {
            fail(&format!(
                "the window shows {:?} after a click",
                statistics.presented_text()
            ));
        }
        std::fs::write(
            directory.join("main.prism"),
            format!(
                "import /prism/core\n{}{}fun main() {{ runApp(title = \"Counter\", width = 400, height = 300) {{ Counter() }}; println(\"closed\") }}\n",
                IMPORTS,
                COUNTER
                    .replace("Count: ", "Clicks: ")
                    .replace("println(\"count ", "println(\"new count ")
            ),
        )
        .unwrap();
        // No input reaches the window until the reloaded code is on screen: the reload's
        // wake-up alone must present it (the `ui` scenario "A reload is presented without
        // input").
        wait_until("the window presents the reloaded app without input", || {
            statistics.presented_text().contains("Clicks: 1")
        });
        driver.settle();
        driver.click(PLUS, scale);
        driver.settle();
        driver.close();
        eprintln!("window input driver: done");
    })
}

fn child(name: &str, directory: &Path) -> i32 {
    let entry = directory.join("main.prism");
    let (server, windows) = MainThreadServer::new();
    let driver = Driver {
        windows: windows.clone(),
    };
    if name == "watch" || name == "watch-reload" {
        std::fs::write(&entry, format!("import /prism/core\n{}{}fun main() {{ runApp(title = \"Counter\", width = 400, height = 300) {{ Counter() }}; println(\"closed\") }}\n", IMPORTS, COUNTER)).unwrap();
        let drive = match name {
            "watch" => watch_driver(directory.to_path_buf()),
            _ => reload_driver(directory.to_path_buf()),
        };
        std::thread::spawn(move || drive(driver));
        let options = prism_cli::RunOptions {
            check_allocations: true,
            ..prism_cli::RunOptions::default()
        };
        return prism_cli::windows::watch_with_windows(&entry, options, None, server, windows);
    }
    let (source, drive) = scenario(name);
    std::fs::write(&entry, source).unwrap();
    std::thread::spawn(move || drive(driver));
    let options = prism_cli::RunOptions {
        check_allocations: true,
        screenshot: Some(directory.join("shot.png")),
        ..prism_cli::RunOptions::default()
    };
    prism_cli::windows::run_with_windows(&entry, &options, server, windows)
}

// --- The parent: runs each scenario in a child and checks what it did

/// A child process, killed and reaped when dropped.
struct Scenario {
    child: Child,
    lines: mpsc::Receiver<String>,
    stderr: Vec<String>,
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Outcome {
    status: Option<i32>,
    stdout: String,
    stderr: String,
    directory: PathBuf,
}

impl Scenario {
    fn start(name: &str) -> (Scenario, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("prism_window_input_{}_{}", name, nanos));
        std::fs::create_dir_all(&directory).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .env("PRISM_WINDOW_INPUT_SCENARIO", name)
            .env("PRISM_WINDOW_INPUT_DIRECTORY", &directory)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the scenario's process starts");
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
        (
            Scenario {
                child,
                lines,
                stderr: Vec::new(),
            },
            directory,
        )
    }

    /// Reads stderr lines until one satisfies `wanted`, for up to 60 s.
    fn wait_for_line(&mut self, wanted: impl Fn(&str) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(60);
        while let Ok(line) = self
            .lines
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            let found = wanted(&line);
            self.stderr.push(line);
            if found {
                return true;
            }
        }
        false
    }

    /// Waits up to 60 s for the child to end, killing it then, and gives what it did.
    fn finish(mut self, directory: PathBuf, kill: bool) -> Outcome {
        let deadline = Instant::now() + Duration::from_secs(60);
        if kill {
            let _ = self.child.kill();
        }
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status.code();
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                break None;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut stdout = String::new();
        if let Some(mut pipe) = self.child.stdout.take() {
            let _ = pipe.read_to_string(&mut stdout);
        }
        while let Ok(line) = self.lines.recv_timeout(Duration::from_millis(200)) {
            self.stderr.push(line);
        }
        Outcome {
            status,
            stdout,
            stderr: self.stderr.join("\n"),
            directory,
        }
    }
}

fn run_scenario(name: &str) -> Outcome {
    let (scenario, directory) = Scenario::start(name);
    scenario.finish(directory, false)
}

macro_rules! check {
    ($failures:expr, $name:expr, $outcome:expr, $condition:expr, $($message:tt)+) => {
        if !$condition {
            $failures.push(format!(
                "{}: {}\n  status {:?}\n  stdout {:?}\n  stderr {:?}",
                $name,
                format!($($message)+),
                $outcome.status,
                $outcome.stdout,
                $outcome.stderr
            ));
        }
    };
}

/// The pixel at physical `(x, y)` of a screenshot, as `0xAARRGGBB`.
fn pixel(shot: &Pixmap, x: u32, y: u32) -> u32 {
    let color = shot.pixel(x, y).expect("a pixel inside the screenshot");
    let color = color.demultiply();
    (color.alpha() as u32) << 24
        | (color.red() as u32) << 16
        | (color.green() as u32) << 8
        | color.blue() as u32
}

fn parent() -> Vec<String> {
    let mut failures = Vec::new();

    let name = "counter";
    let outcome = run_scenario(name);
    check!(failures, name, outcome, outcome.status == Some(0), "status");
    check!(
        failures,
        name,
        outcome,
        outcome.stdout == "count 1\ncount 2\nclosed\n",
        "two clicks count, a press released elsewhere does not, and runApp returns"
    );
    check!(
        failures,
        name,
        outcome,
        outcome.stderr.is_empty(),
        "nothing on stderr: the allocation check is clean"
    );
    match prism_desktop::screenshot::read(&outcome.directory.join("shot.png")) {
        Ok(shot) => {
            // The window's scale factor: the screenshot is 300 × 200 logical pixels after the
            // resize.
            let scale = shot.width() as f64 / 300.0;
            check!(
                failures,
                name,
                outcome,
                scale >= 1.0 && shot.height() as f64 == (200.0 * scale).round(),
                "the screenshot is {} x {}, not 300 x 200 logical pixels at one scale",
                shot.width(),
                shot.height()
            );
            let at = |x: f64, y: f64| pixel(&shot, (x * scale) as u32, (y * scale) as u32);
            check!(
                failures,
                name,
                outcome,
                at(20.0, 63.0) == 0xFF67_50A4,
                "the button is purple: {:#010X}",
                at(20.0, 63.0)
            );
            check!(
                failures,
                name,
                outcome,
                at(290.0, 190.0) == 0xFFFF_FFFF,
                "the resized window is white at its corner: {:#010X}",
                at(290.0, 190.0)
            );
            let dark = (16..80)
                .flat_map(|x| (16..35).map(move |y| (x, y)))
                .filter(|&(x, y)| at(x as f64, y as f64) & 0xFF < 0x80)
                .count();
            check!(
                failures,
                name,
                outcome,
                dark > 20,
                "the count's text is drawn ({} dark pixels)",
                dark
            );
        }
        Err(error) => failures.push(format!("{}: no screenshot: {}", name, error)),
    }

    let name = "close-button";
    let outcome = run_scenario(name);
    check!(
        failures,
        name,
        outcome,
        outcome.status == Some(0) && outcome.stdout == "closed\n" && outcome.stderr.is_empty(),
        "the close button ends runApp, with a clean allocation check"
    );

    if cfg!(target_os = "macos") {
        let name = "cmd-q";
        let outcome = run_scenario(name);
        check!(
            failures,
            name,
            outcome,
            outcome.status == Some(0)
                && outcome.stdout == "closed\nafter\n"
                && outcome.stderr.is_empty(),
            "Cmd+Q closes the window, and main continues"
        );
    }

    let name = "two-apps";
    let outcome = run_scenario(name);
    check!(
        failures,
        name,
        outcome,
        outcome.status == Some(0)
            && outcome.stdout == "one closed\ntwo closed\n"
            && outcome.stderr.is_empty(),
        "two apps run one after the other"
    );

    let name = "text-field";
    let outcome = run_scenario(name);
    check!(
        failures,
        name,
        outcome,
        outcome.status == Some(0)
            && outcome.stdout
                == "value a\nvalue aB\nvalue a\nvalue ax\nvalue axx\nvalue axxx\nclosed\n"
            && outcome.stderr.is_empty(),
        "typing, Backspace and repeats edit the field, and shortcuts, arrows and Enter do not"
    );

    let name = "runtime-error";
    let outcome = run_scenario(name);
    check!(
        failures,
        name,
        outcome,
        outcome.status == Some(101)
            && outcome.stdout.is_empty()
            && outcome.stderr.contains("panic: division by zero"),
        "a runtime error in a handler ends the program with status 101"
    );

    let name = "watch";
    let (mut scenario, directory) = Scenario::start(name);
    let done = scenario.wait_for_line(|line| line == "window input driver: done");
    let outcome = scenario.finish(directory, true);
    let lines: Vec<&str> = outcome.stderr.lines().collect();
    let position = |wanted: &str| lines.iter().position(|line| *line == wanted);
    let returned = position("watch: main returned with status 0; waiting for changes");
    let again = position("watch: running main again");
    let stopped = position("watch: the program stopped with a runtime error; waiting for changes");
    check!(
        failures,
        name,
        outcome,
        done && returned < again && again < stopped && returned.is_some(),
        "closing the window returns from main, an edit runs it again in a new window, and a runtime error in a handler stops it and closes its window"
    );
    check!(
        failures,
        name,
        outcome,
        outcome.stdout == "count 1\nclosed\n",
        "the first run counts and returns"
    );

    let name = "watch-reload";
    let (mut scenario, directory) = Scenario::start(name);
    // The run's output is written when its `main` returns, which may come before or after the
    // driver's last line.
    let returned = "watch: main returned with status 0; waiting for changes";
    let done = scenario.wait_for_line(|line| line == "window input driver: done")
        && (scenario.stderr.iter().any(|line| line == returned)
            || scenario.wait_for_line(|line| line == returned));
    let outcome = scenario.finish(directory, true);
    check!(
        failures,
        name,
        outcome,
        done && outcome.stderr.contains("reload: applied main.prism"),
        "a saved edit reloads the running app, whose idle window presents it"
    );
    check!(
        failures,
        name,
        outcome,
        outcome.stdout == "count 1\nnew count 2\nclosed\n",
        "the next click runs the new handler on the kept count"
    );
    failures
}

fn main() {
    if let Ok(name) = std::env::var("PRISM_WINDOW_INPUT_SCENARIO") {
        let directory = PathBuf::from(std::env::var_os("PRISM_WINDOW_INPUT_DIRECTORY").unwrap());
        std::process::exit(child(&name, &directory));
    }
    if std::env::var("PRISM_WINDOW_TESTS").as_deref() != Ok("1") || !display_available() {
        println!("{}", SKIPPED);
        return;
    }
    let failures = parent();
    if !failures.is_empty() {
        for failure in &failures {
            eprintln!("window input test failed: {}", failure);
        }
        std::process::exit(1);
    }
    println!("window input test passed");
}
