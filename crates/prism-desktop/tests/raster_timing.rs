//! The rasterization benchmark of design D19 of `desktop-window` and the scenario
//! "Rasterization benchmark": how long drawing a display list takes in a window of 800 × 600
//! logical pixels at scales 1 and 2, so that super-linear growth shows up as a factor well
//! above ten.
//!
//! It builds display lists directly, in two shapes of 1,000 and 10,000 nodes:
//!
//! - `grid`: every node visible, a labelled rectangle in a grid filling the window (40 × 25
//!   cells of 20 × 24 for 1,000 nodes, 100 × 100 cells of 8 × 6 for 10,000);
//! - `todo`: a column of rows 40 high, each a background, a checkbox border and an
//!   `Item <n>` label, of which only the first 15 reach the window.
//!
//! and also the counter screen, rasterized and converted into a surface buffer as a window
//! frame is. It reports the median of 50 rasterizations after a warm-up, with the glyph cache
//! warm, and one rasterization by a fresh rasterizer (a cold cache). It also reports the time
//! the first use of the bundled font takes to parse it (D6).
//!
//! It is ignored by default, since timings of a debug build or a busy machine vary. Run it with
//!
//! ```text
//! PRISM_ASSERT_TIMING=1 cargo test --release -p prism-desktop --test raster_timing -- --ignored --nocapture
//! ```
//!
//! Under `PRISM_ASSERT_TIMING=1` it fails when a 1,000-node list or the counter frame takes
//! 4 ms or more at scale 2, or when a 10,000-node list takes more than 15 times as long as the
//! 1,000-node list of its shape.

use prism_desktop::raster::Rasterizer;
use prism_desktop::test_support::{counter_screen_list, ListBuilder};
use prism_ui::draw::DisplayList;
use std::time::{Duration, Instant};

const WIDTH: i64 = 800;
const HEIGHT: i64 = 600;
const RUNS: usize = 50;

fn grid(nodes: usize) -> DisplayList {
    let (columns, rows, cell_width, cell_height, font_size) = match nodes {
        1_000 => (40, 25, 20, 24, 12),
        10_000 => (100, 100, 8, 6, 5),
        _ => unreachable!(),
    };
    let mut builder = ListBuilder::new().clear(WIDTH, HEIGHT, 0xFFFF_FFFF);
    for row in 0..rows {
        for column in 0..columns {
            let (x, y) = (column * cell_width, row * cell_height);
            let colour = 0xFF00_0000 | ((row * 37 + column * 11) % 256) << 8 | 0x80;
            builder = builder
                .rect((x, y, cell_width - 1, cell_height - 1), colour, 0)
                .text(
                    x + 1,
                    y + 1,
                    font_size,
                    0xFFFF_FFFF,
                    &format!("{}", column % 10),
                );
        }
    }
    builder.build()
}

fn todo(nodes: usize) -> DisplayList {
    let mut builder = ListBuilder::new().clear(WIDTH, HEIGHT, 0xFFFF_FFFF);
    for row in 0..nodes as i64 / 3 {
        let y = row * 40;
        builder = builder
            .rect((0, y, WIDTH, 39), 0xFFF3_EDF7, 0)
            .border((8, y + 10, 20, 20), 2, 0xFF67_50A4, 2)
            .text(40, y + 10, 16, 0xFF1D_1B20, &format!("Item {}", row));
    }
    builder.build()
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// The median warm time and the cold time of drawing `list` at `scale`, with the conversion
/// into a surface buffer when `present` is set.
fn measure(list: &DisplayList, scale: f64, present: bool) -> (Duration, Duration) {
    let (width, height) = (
        (WIDTH as f64 * scale) as u32,
        (HEIGHT as f64 * scale) as u32,
    );
    let mut buffer = vec![0u32; (width * height) as usize];
    let mut frame = |rasterizer: &mut Rasterizer| {
        let start = Instant::now();
        rasterizer.rasterize(list, scale, width, height);
        if present {
            rasterizer.copy_to_surface(&mut buffer);
        }
        start.elapsed()
    };
    let cold = frame(&mut Rasterizer::new());
    let mut rasterizer = Rasterizer::new();
    for _ in 0..3 {
        frame(&mut rasterizer);
    }
    let warm = median((0..RUNS).map(|_| frame(&mut rasterizer)).collect());
    (warm, cold)
}

#[test]
#[ignore]
fn raster_timing() {
    let start = Instant::now();
    prism_desktop::font::font();
    println!("font parse: {:.3}ms", milliseconds(start.elapsed()));
    let assert = std::env::var_os("PRISM_ASSERT_TIMING").is_some_and(|value| value == "1");
    println!(
        "{:>16} {:>6} {:>10} {:>10}",
        "list", "scale", "warm", "cold"
    );
    let mut failures = Vec::new();
    let counter = counter_screen_list();
    for scale in [1.0, 2.0] {
        let (warm, cold) = measure(&counter, scale, true);
        println!(
            "{:>16} {:>6} {:>8.3}ms {:>8.3}ms",
            "counter frame",
            scale,
            milliseconds(warm),
            milliseconds(cold)
        );
        if scale == 2.0 && warm >= Duration::from_millis(4) {
            failures.push(format!("the counter frame takes {:?} at scale 2", warm));
        }
    }
    for (shape, build) in [("grid", grid as fn(usize) -> DisplayList), ("todo", todo)] {
        let lists = [build(1_000), build(10_000)];
        for scale in [1.0, 2.0] {
            let times: Vec<(Duration, Duration)> = lists
                .iter()
                .map(|list| measure(list, scale, false))
                .collect();
            for (nodes, (warm, cold)) in [1_000, 10_000].iter().zip(&times) {
                println!(
                    "{:>16} {:>6} {:>8.3}ms {:>8.3}ms",
                    format!("{} {}", shape, nodes),
                    scale,
                    milliseconds(*warm),
                    milliseconds(*cold)
                );
            }
            let growth = times[1].0.as_secs_f64() / times[0].0.as_secs_f64();
            println!("{:>16} {:>6} growth x{:.1}", shape, scale, growth);
            if growth > 15.0 {
                failures.push(format!("{} at scale {} grows x{:.1}", shape, scale, growth));
            }
            if scale == 2.0 && times[0].0 >= Duration::from_millis(4) {
                failures.push(format!("{} 1000 takes {:?} at scale 2", shape, times[0].0));
            }
        }
    }
    if assert {
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
