//! Frame statistics (D7 of `reactive-app-verification`): what `prism run --frame-stats` measures
//! of each app's frames and reports on stderr when `runApp` returns.
//!
//! - **A frame** is the program thread's work for one presented image: from `runApp`'s start,
//!   or the end of the previous present, up to and including the next present. Headless, the
//!   script's `frame` command is the present, and work after the last one forms a last frame.
//!   Its time is the busy time of its frame-loop iterations: waiting for events, `dump`
//!   commands, `edit` and `wait reload` are not counted, nor are the dumps' allocations.
//! - **Stages** are summed per frame: `events` (hit testing and handlers of presses, releases,
//!   characters, keys and resizes), `reload` (applying reloads at the reload point),
//!   `recomposition` (every pass), `layout`, `display list` (building it) and `frame` (the
//!   whole). A stage that did not run in a frame has no sample for it.
//! - **Allocations** are the program thread's, from `prism_runtime::allocation_counter`, which
//!   the `prism` binary enables with the option: per frame, and live after each frame.
//! - **Measuring allocates nothing.** Each stage keeps a [`Histogram`] of fixed buckets,
//!   allocated once when the app starts, so the report's own bookkeeping never shows up in the
//!   allocations it reports. Maxima are exact; medians and percentiles are nearest-rank and
//!   within 1 %.
//!
//! The report's lines all start with `frame stats: ` ([`FrameStatistics::report`]).

use std::fmt::Write;
use std::time::Duration;

/// Values below this are counted exactly, one bucket each.
const EXACT: u64 = 128;
/// The buckets of each power of two at and above [`EXACT`]: its upper half, 64 wide.
const PER_OCTAVE: u64 = 64;
/// The highest power of two counted apart: 2^37 ns is about 137 s, above the 100 s the
/// histograms must cover. Larger values share the last bucket.
const HIGHEST_EXPONENT: u32 = 37;
/// How many buckets a histogram has.
pub const BUCKETS: usize = (EXACT + (HIGHEST_EXPONENT as u64 - 6) * PER_OCTAVE) as usize;

/// A log-linear histogram of `u64` values, nanoseconds or counts: exact below 128, and with
/// 64 buckets per power of two above, so a bucket's midpoint is within 1/128 of every value in
/// it. It holds its buckets inline, so building one allocates nothing and a `const` one can be
/// a static.
#[derive(Clone)]
pub struct Histogram {
    buckets: [u32; BUCKETS],
    count: u64,
    min: u64,
    max: u64,
}

impl std::fmt::Debug for Histogram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Histogram")
            .field("count", &self.count)
            .field("min", &self.min)
            .field("max", &self.max)
            .finish_non_exhaustive()
    }
}

impl Default for Histogram {
    fn default() -> Histogram {
        Histogram::new()
    }
}

/// The bucket of `value`.
fn bucket(value: u64) -> usize {
    if value < EXACT {
        return value as usize;
    }
    let exponent = (63 - value.leading_zeros()).min(HIGHEST_EXPONENT);
    let value = value.min((1u64 << (HIGHEST_EXPONENT + 1)) - 1);
    let sub = (value >> (exponent - 6)) - PER_OCTAVE;
    (EXACT + (exponent as u64 - 7) * PER_OCTAVE + sub) as usize
}

/// The midpoint of bucket `index`, which stands for its values.
fn representative(index: usize) -> u64 {
    let index = index as u64;
    if index < EXACT {
        return index;
    }
    let offset = index - EXACT;
    let exponent = 7 + offset / PER_OCTAVE;
    let sub = PER_OCTAVE + offset % PER_OCTAVE;
    let width = 1u64 << (exponent - 6);
    sub * width + width / 2
}

impl Histogram {
    /// An empty histogram.
    pub const fn new() -> Histogram {
        Histogram {
            buckets: [0; BUCKETS],
            count: 0,
            min: u64::MAX,
            max: 0,
        }
    }

    /// Counts `value`.
    pub fn record(&mut self, value: u64) {
        let index = bucket(value);
        self.buckets[index] = self.buckets[index].saturating_add(1);
        self.count += 1;
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }

    /// Counts a duration, in nanoseconds.
    pub fn record_duration(&mut self, duration: Duration) {
        self.record(duration.as_nanos().min(u64::MAX as u128) as u64);
    }

    /// How many values were counted.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The largest value counted, exactly, or `None` before any.
    pub fn max(&self) -> Option<u64> {
        (self.count > 0).then_some(self.max)
    }

    /// The nearest-rank `percentile`, from 0 to 100, or `None` before any value: the smallest
    /// bucket at or below which `ceil(percentile / 100 × count)` values lie, kept between the
    /// smallest and the largest value, so a histogram of one value, or of equal values, gives
    /// that value exactly.
    pub fn percentile(&self, percentile: f64) -> Option<u64> {
        if self.count == 0 {
            return None;
        }
        let rank = ((percentile / 100.0 * self.count as f64).ceil() as u64).clamp(1, self.count);
        let mut seen = 0u64;
        for (index, &count) in self.buckets.iter().enumerate() {
            seen += count as u64;
            if seen >= rank {
                return Some(representative(index).clamp(self.min, self.max));
            }
        }
        Some(self.max)
    }

    /// The median, nearest-rank.
    pub fn median(&self) -> Option<u64> {
        self.percentile(50.0)
    }

    /// Forgets every value.
    pub fn clear(&mut self) {
        self.buckets.fill(0);
        self.count = 0;
        self.min = u64::MAX;
        self.max = 0;
    }
}

/// `<name> median <m> ms, p99 <p> ms, max <x> ms`, or `<name> none`, of a histogram of
/// nanoseconds, as a report line after `frame stats: `.
pub fn stage_line(out: &mut String, name: &str, histogram: &Histogram) {
    let milliseconds = |nanoseconds: u64| nanoseconds as f64 / 1_000_000.0;
    match (
        histogram.median(),
        histogram.percentile(99.0),
        histogram.max(),
    ) {
        (Some(median), Some(p99), Some(max)) => {
            let _ = write!(
                out,
                "frame stats: {} median {:.3} ms, p99 {:.3} ms, max {:.3} ms",
                name,
                milliseconds(median),
                milliseconds(p99),
                milliseconds(max)
            );
        }
        _ => {
            let _ = write!(out, "frame stats: {} none", name);
        }
    }
}

/// The stages of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Events,
    Reload,
    Recomposition,
    Layout,
    DisplayList,
}

const STAGES: usize = 5;

/// What one app's frames measured, and the frame being measured.
#[derive(Debug)]
pub struct FrameStatistics {
    stages: [Histogram; STAGES],
    frame: Histogram,
    allocations: Histogram,
    frames: u64,
    input_events: u64,
    passes: u64,
    reloads: u64,
    frames_without_allocations: u64,
    smallest_live: u64,
    largest_live: u64,
    /// From `runApp`'s call, and from the run's start, to the end of the first composition and
    /// layout.
    first_frame_after_run_app: Option<Duration>,
    first_frame_after_start: Option<Duration>,
    // The frame being measured.
    current: [Option<u64>; STAGES],
    busy: u64,
    work: bool,
    allocations_at_start: u64,
    excluded_allocations: u64,
}

impl FrameStatistics {
    /// The statistics of an app whose frame starts now, with the program thread's allocation
    /// count `allocations`. Allocated once, when the app starts.
    pub fn new(allocations: u64) -> Box<FrameStatistics> {
        Box::new(FrameStatistics {
            stages: [
                Histogram::new(),
                Histogram::new(),
                Histogram::new(),
                Histogram::new(),
                Histogram::new(),
            ],
            frame: Histogram::new(),
            allocations: Histogram::new(),
            frames: 0,
            input_events: 0,
            passes: 0,
            reloads: 0,
            frames_without_allocations: 0,
            smallest_live: u64::MAX,
            largest_live: 0,
            first_frame_after_run_app: None,
            first_frame_after_start: None,
            current: [None; STAGES],
            busy: 0,
            work: false,
            allocations_at_start: allocations,
            excluded_allocations: 0,
        })
    }

    /// Adds `duration` to `stage` of the current frame.
    pub fn stage(&mut self, stage: Stage, duration: Duration) {
        let slot = &mut self.current[stage as usize];
        *slot = Some(slot.unwrap_or(0) + duration.as_nanos() as u64);
        self.work = true;
    }

    /// Adds busy time to the current frame.
    pub fn busy(&mut self, duration: Duration) {
        self.busy += duration.as_nanos() as u64;
    }

    /// Counts an input event, a pass or an applied reload.
    pub fn input_event(&mut self) {
        self.input_events += 1;
        self.work = true;
    }

    pub fn passes(&mut self, passes: u64) {
        self.passes += passes;
        if passes > 0 {
            self.work = true;
        }
    }

    pub fn reload(&mut self) {
        self.reloads += 1;
        self.work = true;
    }

    /// Excludes `allocations` the program thread made for instrumentation, a dump, from the
    /// current frame.
    pub fn exclude_allocations(&mut self, allocations: u64) {
        self.excluded_allocations += allocations;
    }

    /// Records that the first composition and layout are done, at these times.
    pub fn first_frame_ready(&mut self, after_run_app: Duration, after_start: Duration) {
        if self.first_frame_after_run_app.is_none() {
            self.first_frame_after_run_app = Some(after_run_app);
            self.first_frame_after_start = Some(after_start);
        }
    }

    /// Whether anything happened since the last frame ended.
    pub fn has_work(&self) -> bool {
        self.work
    }

    /// Ends the current frame, the program thread's counts of allocations and frees being
    /// `allocations` and `frees`.
    pub fn end_frame(&mut self, allocations: u64, frees: u64) {
        for (histogram, value) in self.stages.iter_mut().zip(self.current.iter_mut()) {
            if let Some(nanoseconds) = value.take() {
                histogram.record(nanoseconds);
            }
        }
        self.frame.record(self.busy);
        let made = allocations
            .saturating_sub(self.allocations_at_start)
            .saturating_sub(self.excluded_allocations);
        self.allocations.record(made);
        if made == 0 {
            self.frames_without_allocations += 1;
        }
        let live = allocations.saturating_sub(frees);
        self.smallest_live = self.smallest_live.min(live);
        self.largest_live = self.largest_live.max(live);
        self.frames += 1;
        self.busy = 0;
        self.work = false;
        self.allocations_at_start = allocations;
        self.excluded_allocations = 0;
    }

    /// The report's lines, each followed by a newline: the counts, the stages, the lines of
    /// `window` (the window's raster and input-to-present lines, after the frame line), then
    /// the allocations.
    pub fn report(&self, window: &[String]) -> String {
        let mut out = String::new();
        let milliseconds = |duration: Option<Duration>| {
            duration.map_or(0.0, |duration| duration.as_secs_f64() * 1000.0)
        };
        let _ = writeln!(
            out,
            "frame stats: frames {}, events {}, passes {}, reloads {}, first frame ready {:.3} ms after runApp, {:.3} ms after start",
            self.frames,
            self.input_events,
            self.passes,
            self.reloads,
            milliseconds(self.first_frame_after_run_app),
            milliseconds(self.first_frame_after_start),
        );
        let names = [
            (Stage::Events, "events"),
            (Stage::Reload, "reload"),
            (Stage::Recomposition, "recomposition"),
            (Stage::Layout, "layout"),
            (Stage::DisplayList, "display list"),
        ];
        for (stage, name) in names {
            if stage == Stage::Reload && self.reloads == 0 {
                continue;
            }
            stage_line(&mut out, name, &self.stages[stage as usize]);
            out.push('\n');
        }
        stage_line(&mut out, "frame", &self.frame);
        out.push('\n');
        for line in window {
            out.push_str(line);
            out.push('\n');
        }
        let _ = writeln!(
            out,
            "frame stats: allocations per frame median {}, max {}, none in {} of {} frames",
            self.allocations.median().unwrap_or(0),
            self.allocations.max().unwrap_or(0),
            self.frames_without_allocations,
            self.frames
        );
        let smallest = if self.frames == 0 {
            0
        } else {
            self.smallest_live
        };
        let _ = writeln!(
            out,
            "frame stats: live allocations after each frame from {} to {}",
            smallest, self.largest_live
        );
        out
    }

    /// The histogram of a stage, for tests.
    pub fn stage_histogram(&self, stage: Stage) -> &Histogram {
        &self.stages[stage as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic spread of values over the histograms' range.
    fn samples() -> Vec<u64> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..20_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                // From 1 µs to 100 s, spread over the decades.
                let base = 10u64.pow(3 + (state % 8) as u32);
                base + (state >> 8) % (9 * base)
            })
            .collect()
    }

    #[test]
    fn percentiles_are_within_one_percent_and_the_maximum_is_exact() {
        let values = samples();
        let mut histogram = Histogram::new();
        for &value in &values {
            histogram.record(value);
        }
        let mut sorted = values.clone();
        sorted.sort_unstable();
        for percentile in [1.0, 10.0, 50.0, 90.0, 99.0, 99.9, 100.0] {
            let rank = ((percentile / 100.0 * sorted.len() as f64).ceil() as usize).max(1);
            let exact = sorted[rank - 1] as f64;
            let estimate = histogram.percentile(percentile).unwrap() as f64;
            assert!(
                (estimate - exact).abs() <= exact * 0.01,
                "p{}: {} against {}",
                percentile,
                estimate,
                exact
            );
        }
        assert_eq!(histogram.max(), sorted.last().copied());
        assert_eq!(histogram.count(), values.len() as u64);
    }

    #[test]
    fn small_values_are_exact_and_an_empty_histogram_has_none() {
        let mut histogram = Histogram::new();
        assert_eq!(histogram.median(), None);
        assert_eq!(histogram.max(), None);
        for value in [0, 0, 2, 5, 127] {
            histogram.record(value);
        }
        assert_eq!(histogram.median(), Some(2));
        assert_eq!(histogram.percentile(100.0), Some(127));
        histogram.record(u64::MAX);
        assert_eq!(histogram.max(), Some(u64::MAX));
        histogram.clear();
        assert_eq!(histogram.count(), 0);
        let mut line = String::new();
        stage_line(&mut line, "events", &histogram);
        assert_eq!(line, "frame stats: events none");
    }

    /// A report of one frame, or of frames that all took the same, reads the same number as its
    /// median, 99th percentile and maximum, although the bucket's midpoint differs.
    #[test]
    fn percentiles_stay_between_the_smallest_and_largest_value() {
        let mut histogram = Histogram::new();
        histogram.record(629);
        assert_ne!(
            representative(bucket(629)),
            629,
            "629 is not its bucket's midpoint"
        );
        assert_eq!(histogram.median(), Some(629));
        assert_eq!(histogram.percentile(99.0), Some(629));
        histogram.record(629);
        histogram.record(629);
        assert_eq!(histogram.percentile(1.0), Some(629));
        histogram.clear();
        // Both lie in one bucket whose midpoint, 1,003,520, is above them both.
        histogram.record(999_999);
        histogram.record(1_000_001);
        let low = histogram.percentile(1.0).unwrap();
        let high = histogram.percentile(100.0).unwrap();
        assert!((999_999..=1_000_001).contains(&low), "{}", low);
        assert_eq!(high, 1_000_001);
    }

    #[test]
    fn every_bucket_holds_the_values_it_stands_for() {
        for index in 0..BUCKETS - 1 {
            assert_eq!(bucket(representative(index)), index, "bucket {}", index);
        }
        assert_eq!(
            bucket(1 << 40),
            BUCKETS - 1,
            "larger values share the last bucket"
        );
    }

    #[test]
    fn a_report_of_two_frames() {
        let mut statistics = FrameStatistics::new(10);
        statistics.passes(1);
        statistics.stage(Stage::Recomposition, Duration::from_micros(30));
        statistics.stage(Stage::Layout, Duration::from_micros(10));
        statistics.busy(Duration::from_micros(50));
        statistics.first_frame_ready(Duration::from_micros(45), Duration::from_millis(12));
        statistics.end_frame(14, 2);
        assert!(!statistics.has_work());
        statistics.input_event();
        statistics.stage(Stage::Events, Duration::from_micros(5));
        statistics.busy(Duration::from_micros(5));
        statistics.end_frame(14, 2);
        let report = statistics.report(&[]);
        assert_eq!(
            report,
            "frame stats: frames 2, events 1, passes 1, reloads 0, first frame ready 0.045 ms after runApp, 12.000 ms after start\n\
             frame stats: events median 0.005 ms, p99 0.005 ms, max 0.005 ms\n\
             frame stats: recomposition median 0.030 ms, p99 0.030 ms, max 0.030 ms\n\
             frame stats: layout median 0.010 ms, p99 0.010 ms, max 0.010 ms\n\
             frame stats: display list none\n\
             frame stats: frame median 0.005 ms, p99 0.050 ms, max 0.050 ms\n\
             frame stats: allocations per frame median 0, max 4, none in 1 of 2 frames\n\
             frame stats: live allocations after each frame from 12 to 12\n"
        );
    }
}
