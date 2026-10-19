//! The UI benchmark of design D19 of `ui-toolkit` and the scenario "Layout timing benchmark":
//! how long layout, relayout, a display list build and a hit test take on trees of 1,000,
//! 10,000 and 100,000 nodes, so that super-linear growth shows up as a factor well above ten.
//!
//! Each tree is a column of rows, each row holding a text and a 20 × 20 clickable box with a
//! background (three nodes a row), built through the tree's Rust API. It reports the median of
//! several runs of:
//!
//! - `layout`: a full layout, every node marked for measuring;
//! - `relayout`: a layout after one text in the middle changed, keeping its size as a counter's
//!   text does (`row 1666` and `row 0000` are as wide);
//! - `draw`: a display list build;
//! - `hit`: a hit test of the first row's box, which the walk in reverse drawing order reaches
//!   last.
//!
//! It is ignored by default, since timings of a debug build or a busy machine vary. Run it with
//!
//! ```text
//! PRISM_ASSERT_TIMING=1 cargo test --release -p prism-ui --test ui_timing -- --ignored --nocapture
//! ```
//!
//! Under `PRISM_ASSERT_TIMING=1` it fails when a tenfold larger tree multiplies the full
//! layout or the display list time by more than 15, or when relaying out after one text change
//! of the 10,000-node tree takes a tenth of its full layout or more.

use prism_ui::draw::DisplayList;
use prism_ui::handlers::HandlerKind;
use prism_ui::input::hit_test;
use prism_ui::layout::Layout;
use prism_ui::node::NodeKind;
use prism_ui::properties::{Role, Shape};
use prism_ui::test_support::*;
use prism_ui::text::FixedAdvanceMetrics;
use prism_ui::{NodeId, UiTree};
use std::time::{Duration, Instant};

const SIZES: [usize; 3] = [1_000, 10_000, 100_000];
const RUNS: usize = 7;

/// A column of `nodes / 3` rows, and the texts of the rows.
fn build(nodes: usize) -> (TreeBuilder, Vec<NodeId>) {
    // Texts of eight characters, so a changed text keeps its size.
    let mut builder = TreeBuilder::new(800, 1_000_000_000);
    let root = builder.root();
    let column = builder.add(root, NodeKind::Column);
    let mut texts = Vec::new();
    for index in 0..nodes / 3 {
        let row = builder.add(column, NodeKind::Row);
        texts.push(builder.text(row, &format!("row {:04}", index % 10_000)));
        let button = builder.add(row, NodeKind::Box);
        let handler = builder.register(HandlerKind::Click, inline_handler(index as u64));
        builder.chain(
            button,
            &[
                size(20, 20),
                background(0xFF67_50A4, Shape::new(4, 0)),
                clickable(handler, true, Some(Role::Button)),
            ],
        );
    }
    (builder, texts)
}

fn mark_all(tree: &mut UiTree) {
    let mut stack = vec![tree.root()];
    while let Some(id) = stack.pop() {
        stack.extend(tree.children(id));
        tree.mark_needs_measure(id);
    }
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

struct Timings {
    layout: Duration,
    relayout: Duration,
    draw: Duration,
    hit: Duration,
}

fn measure(nodes: usize) -> Timings {
    let (mut builder, texts) = build(nodes);
    let mut layout = Layout::new();
    let metrics = FixedAdvanceMetrics;
    layout.run(&mut builder.tree, &metrics);
    let mut full = Vec::new();
    let mut partial = Vec::new();
    let mut draw = Vec::new();
    let mut hit = Vec::new();
    let mut list = DisplayList::new();
    let middle = texts[texts.len() / 2];
    // The first row's box, which a hit test reaches last.
    let first_box = builder.tree.node(texts[0]).unwrap().next.unwrap();
    for run in 0..RUNS {
        mark_all(&mut builder.tree);
        let start = Instant::now();
        layout.run(&mut builder.tree, &metrics);
        full.push(start.elapsed());

        builder
            .tree
            .set_text(middle, if run % 2 == 0 { "row 0000" } else { "row 1111" });
        let start = Instant::now();
        layout.run(&mut builder.tree, &metrics);
        partial.push(start.elapsed());

        let start = Instant::now();
        list.build(&builder.tree, &metrics, None);
        draw.push(start.elapsed());

        let bounds = prism_ui::layout::absolute_bounds(&builder.tree, first_box);
        let start = Instant::now();
        let target = hit_test(&builder.tree, bounds.x + 1, bounds.y + 1);
        hit.push(start.elapsed());
        assert!(target.is_some());
    }
    Timings {
        layout: median(full),
        relayout: median(partial),
        draw: median(draw),
        hit: median(hit),
    }
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[test]
#[ignore]
fn ui_timing() {
    let timings: Vec<Timings> = SIZES.iter().map(|&nodes| measure(nodes)).collect();
    println!(
        "{:>8} {:>10} {:>10} {:>10} {:>10}",
        "nodes", "layout", "relayout", "draw", "hit"
    );
    for (nodes, timing) in SIZES.iter().zip(&timings) {
        println!(
            "{:>8} {:>8.3}ms {:>8.3}ms {:>8.3}ms {:>8.3}ms",
            nodes,
            milliseconds(timing.layout),
            milliseconds(timing.relayout),
            milliseconds(timing.draw),
            milliseconds(timing.hit)
        );
    }
    let growth = |pick: fn(&Timings) -> Duration| -> Vec<f64> {
        timings
            .windows(2)
            .map(|pair| pick(&pair[1]).as_secs_f64() / pick(&pair[0]).as_secs_f64())
            .collect()
    };
    let layout_growth = growth(|timing| timing.layout);
    let draw_growth = growth(|timing| timing.draw);
    println!(
        "growth per x10: layout {:?}, draw {:?}",
        layout_growth, draw_growth
    );
    let ratio = timings[1].relayout.as_secs_f64() / timings[1].layout.as_secs_f64();
    println!("relayout / layout at 10,000 nodes: {:.4}", ratio);
    if std::env::var_os("PRISM_ASSERT_TIMING").is_some_and(|value| value == "1") {
        assert!(
            layout_growth.iter().all(|growth| *growth <= 15.0),
            "layout grows super-linearly: {:?}",
            layout_growth
        );
        assert!(
            draw_growth.iter().all(|growth| *growth <= 15.0),
            "the display list grows super-linearly: {:?}",
            draw_growth
        );
        assert!(ratio < 0.1, "relayout takes {:.3} of a full layout", ratio);
    }
}
