## ADDED Requirements

### Requirement: A Full Relayout of a Thousand Items
On a release build, headless, resizing the window of `examples/apps/todo/large.prism`, which measures every node again through its measure policy, SHOULD take under 4 ms of layout at the 99th percentile. Every frame of such a resize after the first frame of the run MUST allocate nothing. The frame benchmark MUST measure and report both.

#### Scenario: Resizing a thousand items allocates nothing
- **WHEN** a test of `run_cli_tests.rs` runs `large.prism` headless with `--frame-stats`, a first `frame`, and then ten rounds of `resize 800 601`, `frame`, `resize 800 600` and `frame`
- **THEN** stderr holds `frame stats: allocations per frame median 0` and `none in 20 of 21 frames`, and the exit status is 0

#### Scenario: Timing a full relayout
- **WHEN** `crates/prism-cli/tests/app_frame_timing.rs` runs on a release build with `PRISM_ASSERT_TIMING=1`
- **THEN** it reports the `layout` median and 99th percentile of fifty resizes of `large.prism`, and passes only if the 99th percentile is under 4 ms and the resize frames after the first allocate nothing

## MODIFIED Requirements

### Requirement: The Toolkit Tour
`examples/demo/ui/ui.prism` MUST be a runnable app that composes every composable of `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3`, uses every modifier, shape, arrangement and alignment kind at least once, and declares and composes a layout of its own over `Layout`.

#### Scenario: Touring the toolkit
- **WHEN** the session `touring-the-toolkit` runs the tour with `dump` and `dump draw`
- **THEN** the tree dump holds at least one node of each kind (`column`, `row`, `box`, `text`, `spacer`, `textfield`, `layout`), a `clickable(Button)` and a `toggleable(Checkbox ...)` element, and the display list holds rectangles, rounded rectangles, borders and text
