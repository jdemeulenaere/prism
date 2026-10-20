## ADDED Requirements

### Requirement: Rows Recompose Only When Their Item Changes
Over the 1,000 items of `large.prism`, toggling an item MUST rerun only that item's row and the count of items left, typing into the text field MUST rerun the screen and skip every row, and adding an item MUST rerun every row once and then skip them again, as `dump recompositions` shows.

#### Scenario: Toggling one of a thousand items recomposes its row
- **WHEN** the session `toggling-one-of-a-thousand-items-recomposes-its-row` runs `large.prism` with `dump recompositions`, `click checkbox 2` and `dump recompositions`
- **THEN** the second section lists `ran 1 skipped 0` for `TodoRow` and for `LeftCount`, and no other composable, and the session passes the allocation check

#### Scenario: Typing into the field of a thousand items skips every row
- **WHEN** the session `typing-into-the-field-of-a-thousand-items-skips-every-row` runs `large.prism` with `click textfield`, `dump recompositions`, `type "a"` and `dump recompositions`
- **THEN** the second section lists `ran 1 skipped 0` for `TodoScreen` and `ran 0 skipped 1000` for `TodoRow`, and no other composable that ran

#### Scenario: Adding an item to a thousand recomposes every row once
- **WHEN** the session `adding-an-item-to-a-thousand-recomposes-every-row-once` runs `large.prism` with `click textfield`, `type "x"`, `dump recompositions`, `click "Add"`, `dump recompositions`, `click textfield`, `type "y"` and `dump recompositions`
- **THEN** the second section lists `ran 1001 skipped 0` for `TodoRow`, and the third `ran 0 skipped 1001`

## MODIFIED Requirements

### Requirement: The Apps Meet Their Frame Budgets
On a release build, headless, a click of the counter and the frame that shows it SHOULD take under 1 ms in the median and under 2 ms at the 99th percentile. With inputs 100 ms apart, as a person's, toggling one item of `large.prism` SHOULD take under 1 ms of recomposition in the median, and typing one character into its field under 2 ms; each SHOULD take under 4 ms of frame time at the 99th percentile. An ignored benchmark MUST measure and report these numbers through `--frame-stats`.

#### Scenario: Timing the apps' frames
- **WHEN** `crates/prism-cli/tests/app_frame_timing.rs` runs on a release build with `PRISM_ASSERT_TIMING=1`
- **THEN** it reports each app's frame, recomposition and allocation numbers, and passes only if the targets are met
