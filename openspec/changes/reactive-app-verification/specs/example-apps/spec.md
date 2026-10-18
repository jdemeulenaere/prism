# Example Apps Specification

## Purpose
The reference reactive apps under `examples/` show what Milestone 2 does: a counter, a todo list and a tour of the toolkit, written as Compose code. This capability states what each app must do, the sessions that verify them under the allocation check, how they keep their state across hot reloads, and the frame, allocation and startup targets they must meet.

## ADDED Requirements

### Requirement: Every Example Checks, Is Formatted and Runs
Every `.prism` file under `examples/` MUST pass `prism check` with no diagnostic and be formatted as `prism fmt` would format it. The UI entries `examples/apps/counter/main.prism`, `examples/apps/todo/main.prism`, `examples/apps/todo/large.prism` and `examples/demo/ui/ui.prism` MUST each run under `prism run --check-allocations --headless` with status 0 and nothing on stderr. No example MUST be exempt from these rules.

#### Scenario: Checking and formatting the examples
- **WHEN** `prism check` runs on each `.prism` file under `examples/`, and `prism fmt --check examples/` runs
- **THEN** no diagnostic is reported and no file is listed

#### Scenario: Running the UI entries headless
- **WHEN** `prism run --check-allocations --headless <entry>` runs on each of the four UI entries, without an event script
- **THEN** each composes, lays out and returns, and exits with status 0 with nothing on stderr

### Requirement: The Apps Are Packages Under examples/apps
Each app MUST be a directory `examples/apps/<name>/` whose files declare `package apps.<name>`, with its entry in `main.prism`. The counter MUST be one file. The todo app MUST keep its data and operations, its composables and its entries in separate files, so that its composables reach its model through an import and declarations with explicit types.

#### Scenario: The todo app's files
- **WHEN** `prism check examples/apps/todo` runs
- **THEN** it checks `model.prism`, `screen.prism`, `main.prism` and `large.prism`, all declaring `package apps.todo`, with no diagnostic

### Requirement: A Session Is an Event Script Beside Its App
A session MUST be an event script `<name>.events` in a `sessions` directory beside an app's files, whose leading comment lines name its entry (`# entry: <path>`, relative to the script) and optionally its window size (`# headless: <width>x<height>`). Its name MUST be the slug of a scenario of this specification.

#### Scenario: A session named after no scenario
- **WHEN** a session is renamed to a name that is no scenario of this specification
- **THEN** `example_app_sessions` fails, naming the session

### Requirement: Sessions Pass Under the Allocation Check
Run headless under `--check-allocations`, and under `--watch` when it holds `edit` commands, every session MUST exit with status 0 and print exactly the stdout and stderr recorded beside it.

#### Scenario: Running every session
- **WHEN** `cargo test -p prism-cli --test fixture_tests example_app_sessions` runs
- **THEN** every session under `examples/` passes, and the test fails for a session whose name is no scenario of this specification or for a scenario naming a session that does not exist

### Requirement: The Counter Counts Clicks
The counter app MUST show a button whose label is `Count: <n>`, starting at 0, and add one to `n` on each click. It MUST show a `Reset` button that sets `n` to 0 and is disabled while `n` is 0.

#### Scenario: Counting to three
- **WHEN** the session `counting-to-three` runs the counter with `click "Count: 0"`, `click "Count: 1"`, `click "Count: 2"` and `dump`
- **THEN** the dump holds a text node reading `Count: 3` inside a button, and a `Reset` button that is not disabled

#### Scenario: Resetting the count
- **WHEN** the session `resetting-the-count` runs the counter with `click "Reset"`, `click "Count: 0"`, `click "Count: 1"`, `click "Reset"` and `dump`
- **THEN** the first click changes nothing, and the dump reads `Count: 0` with the `Reset` button disabled

### Requirement: The Todo App Adds Items From Its Text Field
The todo app MUST hold its items in a `MutableState<List<Todo>>`. Clicking `Add` MUST append an item, not done, whose title is the text field's value, and clear the field. With an empty field, `Add` MUST add nothing. Below the items, the app MUST show `<n> left`, the number of items not done.

#### Scenario: Adding, toggling and removing
- **WHEN** the session `adding-toggling-and-removing` adds `Milk`, `Eggs` and `Bread` by `click textfield`, `type` and `click "Add"`, then runs `click checkbox 2`, `click "Remove" 1` and `dump`
- **THEN** the dump shows two rows, `Eggs` checked and `Bread` not, an empty text field and `1 left`, and the session passes the allocation check

#### Scenario: An empty title is not added
- **WHEN** the session `an-empty-title-is-not-added` runs `click "Add"` and `dump` on the empty app
- **THEN** the dump shows no row and `0 left`

### Requirement: Rows Toggle and Remove Their Item
Each item MUST be shown in a row of a `Checkbox` showing whether it is done, its title and a `Remove` button. The checkbox MUST set the item's done state, and `Remove` MUST remove the item, keeping the others in order.

#### Scenario: Toggling an item back
- **WHEN** the session `toggling-an-item-back` adds `Milk`, runs `click checkbox 1`, `dump`, `click checkbox 1` and `dump`
- **THEN** the first dump shows the checkbox checked and `0 left`, and the second shows it unchecked and `1 left`

### Requirement: Filtering Items
The todo app MUST show the buttons `All`, `Active` and `Done`, the selected one disabled. `All` MUST show every item, `Active` the items not done and `Done` the done items, in their order, without changing the items.

#### Scenario: Filtering by state
- **WHEN** the session `filtering-by-state` adds `Milk`, `Eggs` and `Bread`, checks `Eggs`, and runs `click "Active"`, `dump`, `click "Done"`, `dump`, `click "All"` and `dump`
- **THEN** the dumps show `Milk` and `Bread`, then `Eggs` only, then all three, each time with the selected filter's button disabled

### Requirement: Rows Are Keyed by Their Item
Each row MUST be composed in `key(todo.id)`, so that removing or filtering out an item keeps the other rows' groups, nodes and state.

#### Scenario: Removing the first row keeps the others' nodes
- **WHEN** the session `removing-the-first-row-keeps-the-others-nodes` adds `Milk`, `Eggs` and `Bread`, runs `dump`, `click "Remove" 1` and `dump`
- **THEN** in the second dump, the rows of `Eggs` and `Bread` and their checkbox, text and button nodes have the numbers they had in the first

### Requirement: The Toolkit Tour
`examples/demo/ui/ui.prism` MUST be a runnable app that composes every composable of `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3` and uses every modifier, shape, arrangement and alignment kind at least once.

#### Scenario: Touring the toolkit
- **WHEN** the session `touring-the-toolkit` runs the tour with `dump` and `dump draw`
- **THEN** the tree dump holds at least one node of each kind (`column`, `row`, `box`, `text`, `spacer`, `textfield`), a `clickable(Button)` and a `toggleable(Checkbox ...)` element, and the display list holds rectangles, rounded rectangles, borders and text

### Requirement: A Thousand Items Run Cleanly
`examples/apps/todo/large.prism` MUST compose the todo app's screen over 1,000 sample items in an 800 × 600 window, and free everything when it ends.

#### Scenario: A thousand items start and end cleanly
- **WHEN** the session `a-thousand-items-start-and-end-cleanly` runs `large.prism` with `click checkbox 2`, `frame`, `click checkbox 2` and `frame`
- **THEN** stdout and stderr are empty, and the exit status is 0 under the allocation check

### Requirement: Hot Reload Keeps the Apps' State
Under `prism run --watch`, an edit of an app's composable that keeps the types of its remembered values MUST keep every remembered value, state cell, item, checked state, text field value and focus, and the next frame MUST show the new code.

#### Scenario: Relabelling keeps the count
- **WHEN** the session `relabelling-keeps-the-count` clicks the counter three times, edits `main.prism` so that the label reads `Clicks: ${count.value}`, then runs `dump`, `click "Clicks: 3"` and `dump`
- **THEN** the first dump reads `Clicks: 3` and the second `Clicks: 4`, and stderr holds `reload: applied apps/counter/main.prism`

#### Scenario: Editing the row layout keeps the list
- **WHEN** the session `editing-the-row-layout-keeps-the-list` adds three items, checks the second, types `Jam` into the field without adding it, edits `screen.prism` so that a row shows its title before its checkbox, with a vertical padding of 4 and done titles in grey, and then runs `dump`, `type "s"`, `click "Add"` and `dump`
- **THEN** the first dump shows the three items in the new layout, the second checked, and the field focused with `Jam`, every node keeping its number, and the second dump shows a fourth item `Jams`

### Requirement: Apps Free What They Drop in the Same Frame
An app MUST free every value it drops during the frame that drops it. Over repeated clicks of the counter or toggles of a todo item, the live allocations after each frame MUST stay equal.

#### Scenario: Repeated clicks keep the live allocations constant
- **WHEN** a test of `run_cli_tests.rs` runs the counter headless with `--frame-stats` and ten rounds of `click button 1` and `frame`
- **THEN** stderr holds `frame stats: live allocations after each frame from <n> to <n>`, the same number twice

### Requirement: Frames That Change Nothing Allocate Nothing
A frame whose events invalidate no scope and change no node MUST allocate nothing.

#### Scenario: Clicking empty space
- **WHEN** a test of `run_cli_tests.rs` runs the counter headless with `--frame-stats`, a first `frame`, and then ten rounds of a click on a position no node covers and `frame`
- **THEN** stderr holds `frame stats: allocations per frame median 0` and `none in 10 of 11 frames`

### Requirement: The Apps Meet Their Frame Budgets
On a release build, headless, a click of the counter and the frame that shows it SHOULD take under 1 ms in the median and under 2 ms at the 99th percentile. Toggling one item of `large.prism` SHOULD take under 4 ms of recomposition in the median and under 4 ms of frame time at the 99th percentile. An ignored benchmark MUST measure and report these numbers through `--frame-stats`.

#### Scenario: Timing the apps' frames
- **WHEN** `crates/prism-cli/tests/app_frame_timing.rs` runs on a release build with `PRISM_ASSERT_TIMING=1`
- **THEN** it reports each app's frame, recomposition and allocation numbers, and passes only if the targets are met

### Requirement: The Apps Start Quickly
The benchmark MUST report when the todo app's first frame is ready after `prism run` starts, headless on a release build, without the build cache and with a warm one. It SHOULD be under 50 ms and under 20 ms.

#### Scenario: Timing the first frame
- **WHEN** the same benchmark runs `prism run --frame-stats --headless examples/apps/todo/main.prism` twenty times without the cache and twenty times with it
- **THEN** it reports the median `first frame ready ... after start` of each, and, with `PRISM_ASSERT_TIMING=1`, passes only if both are under their targets
