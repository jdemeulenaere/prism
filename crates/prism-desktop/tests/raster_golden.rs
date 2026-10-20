//! Golden images of the rasterizer (D20 of `desktop-window`, the scenario "Golden images").
//!
//! Each case is a display list built by hand, rasterized at scales 1, 1.5 and 2, and compared
//! with `tests/golden/<case>@<scale>.png` within one unit per channel, which absorbs
//! floating-point differences between architectures in `fontdue`'s glyph coverage. Run with
//! `PRISM_UPDATE_GOLDEN=1` to rewrite the images, and look at every changed one in the diff.

use prism_desktop::raster::Rasterizer;
use prism_desktop::test_support::{counter_screen_list, ListBuilder};
use prism_desktop::FontMetrics;
use prism_ui::draw::DisplayList;
use prism_ui::layout::Layout;
use prism_ui::test_support::{counter_screen, inline_handler, TreeBuilder};
use std::path::PathBuf;
use tiny_skia::Pixmap;

const WHITE: i64 = 0xFFFF_FFFF;
const BLACK: i64 = 0xFF00_0000;

fn rectangles() -> DisplayList {
    ListBuilder::new()
        .clear(48, 32, WHITE)
        .rect((2, 2, 10, 8), 0xFFFF_0000, 0)
        .rect((12, 2, 10, 8), 0xFF00_A000, 0)
        .rect((22, 2, 7, 8), 0xFF00_00FF, 0)
        .rect((4, 6, 30, 6), 0x8000_00FF, 0)
        .rect((3, 15, 1, 15), BLACK, 0)
        .rect((5, 15, 1, 15), BLACK, 0)
        .rect((7, 15, 1, 15), BLACK, 0)
        .rect((10, 16, 35, 13), 0xFF67_50A4, 0)
        .build()
}

fn rounded_rectangles() -> DisplayList {
    ListBuilder::new()
        .clear(64, 40, WHITE)
        .rect((2, 2, 36, 16), 0xFF67_50A4, 4)
        .rect((42, 2, 20, 20), 0xFFFF_0000, 10)
        .rect((2, 22, 36, 16), 0xFF00_80FF, 8)
        .rect((10, 26, 40, 10), 0x80FF_8000, 5)
        .rect((44, 26, 6, 6), BLACK, 1)
        .build()
}

fn borders() -> DisplayList {
    ListBuilder::new()
        .clear(64, 40, WHITE)
        .border((2, 2, 20, 16), 1, BLACK, 0)
        .border((24, 2, 20, 16), 2, 0xFF67_50A4, 8)
        .rect((46, 2, 16, 16), 0xFFE0_E0E0, 4)
        .border((46, 2, 16, 16), 3, 0xFFFF_0000, 4)
        .border((2, 22, 60, 16), 1, 0xFF79_747E, 0)
        .border((4, 24, 6, 6), 4, 0xFF00_80FF, 0)
        .build()
}

fn text() -> DisplayList {
    ListBuilder::new()
        .clear(160, 86, WHITE)
        .text(2, 2, 12, BLACK, "Hamburgefonstiv 0123")
        .text(2, 18, 16, BLACK, "Count: 0 +")
        .text(2, 38, 24, 0xFF67_50A4, "AV AX")
        .text(100, 38, 24, BLACK, "中")
        .rect((2, 66, 156, 18), 0xFF67_50A4, 9)
        .text(8, 68, 12, WHITE, "white on purple")
        .build()
}

/// The counter screen, as `dump draw` prints it headless.
fn counter_headless() -> DisplayList {
    counter_screen_list()
}

/// The counter screen laid out with the window's metrics.
fn counter_window() -> DisplayList {
    let metrics = FontMetrics::new();
    let mut builder = TreeBuilder::new(200, 120);
    counter_screen(&mut builder, "Count: 0", inline_handler(1));
    builder.lay_out_with(&mut Layout::new(), &metrics);
    let mut list = DisplayList::new();
    list.build(&builder.tree, &metrics, None);
    list
}

fn clipped_and_culled() -> DisplayList {
    ListBuilder::new()
        .clear(40, 30, WHITE)
        .rect((-6, -6, 14, 14), 0xFFFF_0000, 6)
        .rect((30, 20, 20, 20), 0xFF00_00FF, 0)
        .border((-2, 22, 12, 12), 2, BLACK, 0)
        .text(24, -6, 16, BLACK, "Ab")
        .rect((50, 0, 10, 10), 0xFF00_FF00, 0)
        .rect((0, -20, 10, 10), 0xFF00_FF00, 3)
        .text(-80, 10, 16, BLACK, "far left")
        .build()
}

/// A golden case: its name and how it builds its list.
type Case = (&'static str, fn() -> DisplayList);

const CASES: [Case; 7] = [
    ("rectangles", rectangles),
    ("rounded", rounded_rectangles),
    ("borders", borders),
    ("text", text),
    ("counter-headless", counter_headless),
    ("counter-window", counter_window),
    ("clipped", clipped_and_culled),
];

const SCALES: [(f64, &str); 3] = [(1.0, "1"), (1.5, "1.5"), (2.0, "2")];

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{}.png", name))
}

/// The logical size of a list: its clear's.
fn logical_size(list: &DisplayList) -> (i64, i64) {
    match list.commands.first() {
        Some(prism_ui::draw::DrawCommand::Clear { width, height, .. }) => (*width, *height),
        _ => unreachable!("every case starts with a clear"),
    }
}

/// The largest difference of one channel between two images of the same size.
fn largest_difference(a: &Pixmap, b: &Pixmap) -> u8 {
    a.data()
        .iter()
        .zip(b.data())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

#[test]
fn golden_images() {
    let update = std::env::var("PRISM_UPDATE_GOLDEN").as_deref() == Ok("1");
    let mut failures = Vec::new();
    for (case, build) in CASES {
        let list = build();
        let (width, height) = logical_size(&list);
        for (scale, label) in SCALES {
            let name = format!("{}@{}", case, label);
            let physical = |v: i64| (v as f64 * scale).round() as u32;
            let mut rasterizer = Rasterizer::new();
            rasterizer.rasterize(&list, scale, physical(width), physical(height));
            let image = rasterizer.take_pixmap().unwrap();
            // The same list gives the same pixels on every run.
            rasterizer.rasterize(&list, scale, physical(width), physical(height));
            assert_eq!(
                rasterizer.pixmap().unwrap().data(),
                image.data(),
                "{}",
                name
            );
            let path = golden(&name);
            if update {
                image.save_png(&path).unwrap();
                continue;
            }
            let expected = match Pixmap::load_png(&path) {
                Ok(expected) => expected,
                Err(error) => {
                    failures.push(format!("{}: {} ({:?})", name, error, path));
                    continue;
                }
            };
            if (expected.width(), expected.height()) != (image.width(), image.height()) {
                failures.push(format!(
                    "{}: {}x{} expected, {}x{} drawn",
                    name,
                    expected.width(),
                    expected.height(),
                    image.width(),
                    image.height()
                ));
                continue;
            }
            let difference = largest_difference(&expected, &image);
            if difference > 1 {
                let actual =
                    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{}.png", name));
                image.save_png(&actual).unwrap();
                failures.push(format!(
                    "{}: a channel differs by {} (drawn: {:?})",
                    name, difference, actual
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "golden images differ; rerun with PRISM_UPDATE_GOLDEN=1 and review the images:\n{}",
        failures.join("\n")
    );
}
