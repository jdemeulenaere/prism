//! The window smoke test (D20 of `desktop-window`): opens real windows, so it has its own
//! `main` (`harness = false`), which runs on the process's main thread as `winit` requires.
//!
//! It runs only with `PRISM_WINDOW_TESTS=1` and, outside macOS and Windows, `DISPLAY` or
//! `WAYLAND_DISPLAY`; otherwise it prints why it is skipped and passes, so `cargo test` stays
//! safe without a display:
//!
//! ```text
//! PRISM_WINDOW_TESTS=1 cargo test -p prism-desktop --test window_smoke
//! ```
//!
//! The main thread serves windows while a program thread, as `prism run`'s would, opens a
//! 320 × 200 window titled `Counter` and presents the counter screen. The test checks the
//! scenarios "Threads of a window app" (the window is created, rasterized and presented on the
//! main thread, the frames presented on the program thread), "Title and size of the window"
//! and "An idle window" (500 ms without input rasterize and present nothing), then that a
//! changed frame is drawn once, that dropping the host closes the window, that a second window
//! opens after it on the same event loop, and that the server returns within 1 s of the
//! program's end. It reports the time to the first frame and the median frame time.
//!
//! The second window checks the scenario "A window app's frame statistics" of
//! `reactive-app-verification`: it presents two different display lists, each after one press
//! delivered through the window, and its report lines then count two presented frames with a
//! raster sample and an input-to-present sample each.

use prism_desktop::input::{ElementState, MouseButton, WindowInput};
use prism_desktop::{FontMetrics, MainThreadServer, WindowHost, WindowSystem};
use prism_ui::draw::DisplayList;
use prism_ui::host::HostEvent;
use prism_ui::layout::Layout;
use prism_ui::test_support::{counter_screen, inline_handler, TreeBuilder};
use prism_ui::Host;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

const SKIPPED: &str =
    "window smoke test skipped: set PRISM_WINDOW_TESTS=1 on a machine with a display";

fn display_available() -> bool {
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return true;
    }
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// The counter screen showing `count`, laid out with the window's metrics.
fn counter(size: (i64, i64), count: i64) -> DisplayList {
    let metrics = FontMetrics::new();
    let mut builder = TreeBuilder::new(size.0, size.1);
    counter_screen(
        &mut builder,
        &format!("Count: {}", count),
        inline_handler(1),
    );
    Layout::new().run(&mut builder.tree, &metrics);
    let mut list = DisplayList::new();
    list.build(&builder.tree, &metrics, None);
    list
}

/// Waits up to `limit` for `condition`.
fn wait_for(limit: Duration, condition: impl Fn() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    condition()
}

macro_rules! check {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            return Err(format!($($message)+));
        }
    };
}

/// What the program thread saw.
struct Report {
    thread: ThreadId,
    first_frame: Duration,
    ended: Instant,
}

/// The program thread: two windows, one after the other.
fn program(system: WindowSystem) -> Result<Report, String> {
    let statistics = system.statistics();
    let opened = Instant::now();
    let mut host: WindowHost = system.try_open("Counter", 320, 200)?;
    check!(
        host.size() == (320, 200),
        "the window is {:?}, not 320 x 200",
        host.size()
    );
    host.present(&counter(host.size(), 0));
    check!(
        wait_for(Duration::from_secs(5), || statistics.new_frames_presented()
            >= 1),
        "no frame was presented within 5 s"
    );
    let first_frame = opened.elapsed();
    let window = statistics.last_window().expect("a window report");
    check!(window.title == "Counter", "the title is {:?}", window.title);
    check!(
        window.logical_size == (320.0, 200.0),
        "the inner size is {:?} logical pixels",
        window.logical_size
    );
    // The platform may redraw a new window once or twice as it is shown and activated, which
    // presents the last pixels again; let that settle first.
    let counts = || {
        (
            statistics.frames_rasterized(),
            statistics.frames_presented(),
        )
    };
    let settle = Instant::now();
    loop {
        let before = counts();
        std::thread::sleep(Duration::from_millis(200));
        if counts() == before {
            break;
        }
        check!(
            settle.elapsed() < Duration::from_secs(3),
            "the new window keeps redrawing"
        );
    }
    // An idle window does nothing.
    let (rasterized, presented) = counts();
    std::thread::sleep(Duration::from_millis(500));
    check!(
        counts() == (rasterized, presented),
        "an idle window rasterized {} and presented {} frames",
        statistics.frames_rasterized() - rasterized,
        statistics.frames_presented() - presented
    );
    // A changed frame is drawn once.
    let shown = statistics.new_frames_presented();
    host.present(&counter(host.size(), 1));
    check!(
        wait_for(Duration::from_secs(5), || statistics.new_frames_presented()
            > shown),
        "the changed frame was not presented"
    );
    std::thread::sleep(Duration::from_millis(100));
    check!(
        statistics.frames_rasterized() == rasterized + 1,
        "the changed frame was rasterized {} times",
        statistics.frames_rasterized() - rasterized
    );
    drop(host);
    // A second window, on the same event loop, whose two frames each follow one press.
    let mut second = system.try_open("Two", 200, 100)?;
    for count in [2, 3] {
        system.deliver(WindowInput::CursorMoved { x: 10.0, y: 10.0 });
        system.deliver(WindowInput::MouseButton {
            button: MouseButton::Left,
            state: ElementState::Pressed,
        });
        let event = second.next_event();
        let event = if event == Some(HostEvent::Frame) {
            second.next_event()
        } else {
            event
        };
        check!(
            matches!(event, Some(HostEvent::Press(_, _))),
            "the press arrived as {:?}",
            event
        );
        let shown = statistics.new_frames_presented();
        second.present(&counter(second.size(), count));
        check!(
            wait_for(Duration::from_secs(5), || statistics.new_frames_presented()
                > shown),
            "the second window did not present frame {}",
            count
        );
    }
    let lines = second.frame_statistics_lines();
    check!(
        lines.len() == 2
            && lines[0].starts_with("frame stats: raster median ")
            && lines[0].ends_with(" (2 frames presented)")
            && lines[1].starts_with("frame stats: input to present median "),
        "the window's report lines are {:?}",
        lines
    );
    check!(
        statistics.presented_samples() == (2, 2),
        "the second window's samples are {:?}",
        statistics.presented_samples()
    );
    println!("window smoke test: {}", lines.join("; "));
    check!(
        statistics.last_window().map(|window| window.title) == Some("Two".to_string()),
        "the second window is not titled Two"
    );
    drop(second);
    let ended = Instant::now();
    system.program_ended();
    Ok(Report {
        thread: std::thread::current().id(),
        first_frame,
        ended,
    })
}

fn run() -> Result<(), String> {
    let main_thread = std::thread::current().id();
    let (server, system) = MainThreadServer::new();
    let statistics = server.statistics();
    let result = Arc::new(Mutex::new(None));
    let slot = result.clone();
    let program = std::thread::Builder::new()
        .name("program".to_string())
        .spawn(move || {
            let report = program(system.clone());
            if report.is_err() {
                system.program_ended();
            }
            *slot.lock().unwrap() = Some(report);
        })
        .unwrap();
    server.serve();
    let returned = Instant::now();
    program
        .join()
        .map_err(|_| "the program thread panicked".to_string())?;
    let report = result.lock().unwrap().take().expect("a report")?;
    check!(
        report.thread != main_thread,
        "the program ran on the main thread"
    );
    check!(
        statistics.window_thread() == Some(main_thread),
        "the window was not created on the main thread"
    );
    check!(
        statistics.event_loops_created() == 1,
        "{} event loops were created",
        statistics.event_loops_created()
    );
    check!(
        statistics.windows_opened() == 2,
        "{} windows opened",
        statistics.windows_opened()
    );
    check!(
        returned.duration_since(report.ended) < Duration::from_secs(1),
        "the server returned {:?} after the program ended",
        returned.duration_since(report.ended)
    );
    println!(
        "window smoke test: first frame after {:.1} ms, median frame {:.3} ms over {} frames",
        report.first_frame.as_secs_f64() * 1000.0,
        statistics.median_frame_time().as_secs_f64() * 1000.0,
        statistics.frames_presented()
    );
    Ok(())
}

fn main() {
    if std::env::var("PRISM_WINDOW_TESTS").as_deref() != Ok("1") || !display_available() {
        println!("{}", SKIPPED);
        return;
    }
    if let Err(message) = run() {
        eprintln!("window smoke test failed: {}", message);
        std::process::exit(1);
    }
    println!("window smoke test passed");
}
