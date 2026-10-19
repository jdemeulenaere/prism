//! What the window system did (D19 of `desktop-window`), shared by both threads: atomic
//! counters, the frame times of the main thread, and what tests check about threads and
//! windows.
//!
//! For `--frame-stats` (D7 of `reactive-app-verification`), each frame of the program that the
//! main thread presents adds a raster sample (its time to rasterize and present) and, when the
//! program thread stamped it with the instant it took the frame's first event, an
//! input-to-present sample, to fixed histograms that allocate nothing. A frame the mailbox
//! replaced before it was drawn was never presented and has no sample. The app's report reads
//! them when its window has closed ([`WindowStatistics::frame_statistics_lines`]).

use prism_ui::statistics::{stage_line, Histogram};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread::ThreadId;
use std::time::Duration;

/// How many recent frame times the median is taken over.
const SAMPLES: usize = 64;

/// The most recent frame times, in nanoseconds, in a ring.
#[derive(Debug)]
struct FrameTimes {
    ring: [u64; SAMPLES],
    len: usize,
    next: usize,
}

/// The histograms of the frames of the program the window presented.
#[derive(Debug)]
struct PresentedFrames {
    raster: Histogram,
    input_to_present: Histogram,
}

/// A window as the main thread created it.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowReport {
    pub title: String,
    /// The inner size in logical pixels, as the platform reports it.
    pub logical_size: (f64, f64),
    pub scale_factor: f64,
}

/// The window system's counters and times.
#[derive(Debug)]
pub struct WindowStatistics {
    event_loops_created: AtomicU64,
    windows_opened: AtomicU64,
    windows_closed: AtomicU64,
    font_loaded: AtomicBool,
    frames_received: AtomicU64,
    frames_rasterized: AtomicU64,
    frames_presented: AtomicU64,
    new_frames_presented: AtomicU64,
    last_frame: AtomicU64,
    times: Mutex<FrameTimes>,
    presented: Mutex<PresentedFrames>,
    window_thread: Mutex<Option<ThreadId>>,
    last_window: Mutex<Option<WindowReport>>,
}

impl Default for WindowStatistics {
    fn default() -> WindowStatistics {
        WindowStatistics::new()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl WindowStatistics {
    pub const fn new() -> WindowStatistics {
        WindowStatistics {
            event_loops_created: AtomicU64::new(0),
            windows_opened: AtomicU64::new(0),
            windows_closed: AtomicU64::new(0),
            font_loaded: AtomicBool::new(false),
            frames_received: AtomicU64::new(0),
            frames_rasterized: AtomicU64::new(0),
            frames_presented: AtomicU64::new(0),
            new_frames_presented: AtomicU64::new(0),
            last_frame: AtomicU64::new(0),
            times: Mutex::new(FrameTimes {
                ring: [0; SAMPLES],
                len: 0,
                next: 0,
            }),
            presented: Mutex::new(PresentedFrames {
                raster: Histogram::new(),
                input_to_present: Histogram::new(),
            }),
            window_thread: Mutex::new(None),
            last_window: Mutex::new(None),
        }
    }

    /// Records a new frame of the program, presented after `raster` of rasterizing and
    /// presenting, `input` after the program thread took its first event when it had one.
    pub fn frame_shown(&self, raster: Duration, input: Option<Duration>) {
        let mut presented = lock(&self.presented);
        presented.raster.record_duration(raster);
        if let Some(input) = input {
            presented.input_to_present.record_duration(input);
        }
    }

    /// Forgets the presented frames' samples: a new app's window starts its own.
    pub fn reset_presented_frames(&self) {
        let mut presented = lock(&self.presented);
        presented.raster.clear();
        presented.input_to_present.clear();
    }

    /// The frames of the program presented since the last reset, and their input-to-present
    /// samples.
    pub fn presented_samples(&self) -> (u64, u64) {
        let presented = lock(&self.presented);
        (presented.raster.count(), presented.input_to_present.count())
    }

    /// The window's lines of the frame statistics report: `frame stats: raster median ...
    /// (<k> frames presented)` and `frame stats: input to present median ...`.
    pub fn frame_statistics_lines(&self) -> Vec<String> {
        let presented = lock(&self.presented);
        let mut raster = String::new();
        stage_line(&mut raster, "raster", &presented.raster);
        raster.push_str(&format!(" ({} frames presented)", presented.raster.count()));
        let mut input = String::new();
        stage_line(&mut input, "input to present", &presented.input_to_present);
        vec![raster, input]
    }

    pub fn event_loop_created(&self) {
        self.event_loops_created.fetch_add(1, Ordering::Relaxed);
    }

    pub fn event_loops_created(&self) -> u64 {
        self.event_loops_created.load(Ordering::Relaxed)
    }

    /// Records a window the current thread created.
    pub fn window_opened(&self, report: WindowReport) {
        self.windows_opened.fetch_add(1, Ordering::Relaxed);
        *lock(&self.window_thread) = Some(std::thread::current().id());
        *lock(&self.last_window) = Some(report);
    }

    pub fn windows_opened(&self) -> u64 {
        self.windows_opened.load(Ordering::Relaxed)
    }

    /// Records that the main thread closed a window it had opened.
    pub fn window_closed(&self) {
        self.windows_closed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn windows_closed(&self) -> u64 {
        self.windows_closed.load(Ordering::Relaxed)
    }

    /// The thread that created the last window, and so runs its event loop.
    pub fn window_thread(&self) -> Option<ThreadId> {
        *lock(&self.window_thread)
    }

    pub fn last_window(&self) -> Option<WindowReport> {
        lock(&self.last_window).clone()
    }

    pub fn font_was_loaded(&self) {
        self.font_loaded.store(true, Ordering::Relaxed);
    }

    /// Whether a window of this system parsed the font.
    pub fn font_loaded(&self) -> bool {
        self.font_loaded.load(Ordering::Relaxed)
    }

    pub fn frame_received(&self) {
        self.frames_received.fetch_add(1, Ordering::Relaxed);
    }

    pub fn frames_received(&self) -> u64 {
        self.frames_received.load(Ordering::Relaxed)
    }

    pub fn frame_rasterized(&self) {
        self.frames_rasterized.fetch_add(1, Ordering::Relaxed);
    }

    pub fn frames_rasterized(&self) -> u64 {
        self.frames_rasterized.load(Ordering::Relaxed)
    }

    /// Records a presented frame, whether it showed a new frame of the program, and how long
    /// drawing it took.
    pub fn frame_presented(&self, time: Duration, new_frame: bool) {
        let nanoseconds = time.as_nanos().min(u64::MAX as u128) as u64;
        self.last_frame.store(nanoseconds, Ordering::Relaxed);
        let mut times = lock(&self.times);
        let next = times.next;
        times.ring[next] = nanoseconds;
        times.next = (next + 1) % SAMPLES;
        times.len = (times.len + 1).min(SAMPLES);
        drop(times);
        self.frames_presented.fetch_add(1, Ordering::Relaxed);
        if new_frame {
            self.new_frames_presented.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The presented frames that showed a new frame of the program, rather than the last
    /// pixels again.
    pub fn new_frames_presented(&self) -> u64 {
        self.new_frames_presented.load(Ordering::Relaxed)
    }

    pub fn frames_presented(&self) -> u64 {
        self.frames_presented.load(Ordering::Relaxed)
    }

    pub fn last_frame_time(&self) -> Duration {
        Duration::from_nanos(self.last_frame.load(Ordering::Relaxed))
    }

    /// The median of the last 64 frame times, or zero before any frame.
    pub fn median_frame_time(&self) -> Duration {
        let times = lock(&self.times);
        let mut recent = times.ring;
        let recent = &mut recent[..times.len];
        if recent.is_empty() {
            return Duration::ZERO;
        }
        recent.sort_unstable();
        Duration::from_nanos(recent[recent.len() / 2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_times_and_counters() {
        let statistics = WindowStatistics::new();
        assert_eq!(statistics.median_frame_time(), Duration::ZERO);
        for milliseconds in [5, 1, 3] {
            statistics.frame_presented(Duration::from_millis(milliseconds), true);
        }
        assert_eq!(statistics.frames_presented(), 3);
        assert_eq!(statistics.last_frame_time(), Duration::from_millis(3));
        assert_eq!(statistics.median_frame_time(), Duration::from_millis(3));
        for _ in 0..100 {
            statistics.frame_presented(Duration::from_millis(2), false);
        }
        assert_eq!(statistics.median_frame_time(), Duration::from_millis(2));
        assert_eq!(statistics.new_frames_presented(), 3);
        statistics.window_opened(WindowReport {
            title: "Counter".to_string(),
            logical_size: (320.0, 200.0),
            scale_factor: 2.0,
        });
        assert_eq!(statistics.windows_opened(), 1);
        assert_eq!(
            statistics.window_thread(),
            Some(std::thread::current().id())
        );
        assert_eq!(statistics.last_window().unwrap().title, "Counter");
    }

    #[test]
    fn presented_frames_and_their_lines() {
        let statistics = WindowStatistics::new();
        assert_eq!(
            statistics.frame_statistics_lines(),
            [
                "frame stats: raster none (0 frames presented)",
                "frame stats: input to present none"
            ]
        );
        statistics.frame_shown(Duration::from_micros(400), None);
        statistics.frame_shown(Duration::from_micros(500), Some(Duration::from_millis(3)));
        assert_eq!(statistics.presented_samples(), (2, 1));
        let lines = statistics.frame_statistics_lines();
        assert!(
            lines[0].starts_with("frame stats: raster median 0.400 ms, p99 0.500 ms, max 0.500 ms"),
            "{:?}",
            lines
        );
        assert!(lines[0].ends_with(" (2 frames presented)"), "{:?}", lines);
        assert_eq!(
            lines[1],
            "frame stats: input to present median 3.000 ms, p99 3.000 ms, max 3.000 ms"
        );
        statistics.reset_presented_frames();
        assert_eq!(statistics.presented_samples(), (0, 0));
    }
}
