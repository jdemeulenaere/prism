# UI Specification Delta

## Purpose
The UI toolkit turns compositions into user interfaces. This change adds frame statistics to the capability that `ui-toolkit` creates, so that an app's frames, their stages and their allocations can be measured, and lets event scripts name nodes by their kind.

## MODIFIED Requirements

### Requirement: Headless Runs Follow an Event Script
A headless run MUST read its event script's commands in order and end the app after the last one: `click`, `press` and `release` at a text target, at a kind target ("Script Targets by Kind") or at `<x> <y>`, `type "<text>"`, `key backspace`, `key enter`, `resize <width> <height>`, `frame`, `dump`, `dump tree` and `dump draw`, and, under `--watch`, `edit <target> <replacement>` and `wait reload`. Blank lines and `#` comments MUST be ignored.

#### Scenario: Scripted counter session
- **WHEN** the counter program runs headless with the script `# two clicks`, `click "+"`, `click "+"` and `dump`
- **THEN** stdout holds a tree whose text node reads `Count: 2`, and the run passes the allocation check

#### Scenario: Resizing the window
- **WHEN** a headless app composes `Box(modifier = Modifier.fillMaxSize()) { }` and its script is `resize 200 100` then `dump`
- **THEN** stdout is the lines `window 200x100` and `  box #1 0,0 200x100 fillMaxSize`

## ADDED Requirements

### Requirement: Frames Are Measured Between Presents
With frame statistics on, a frame MUST be the program thread's work from `runApp`'s start, or the end of the previous present, up to and including the next present: events and handlers, reloads, passes, layout, and the display list. Headless, the script's `frame` command MUST count as the present, and work after the last one MUST form a last frame when the app ends.

#### Scenario: Two clicks and one frame
- **WHEN** the counter screen runs headless with `--frame-stats` and the script `click "+"`, `click "+"` and `frame`
- **THEN** the report's first line starts with `frame stats: frames 1, events 4, passes 3, reloads 0, `: the first composition, two presses, two releases and their passes form one frame

### Requirement: Frame Time Is Busy Time
A frame's time MUST be the busy time of its frame-loop iterations, without the time spent waiting for events. Only presses, releases, characters, keys and resizes MUST count as input events. The script's `dump` commands MUST NOT count in any frame, neither their time nor their allocations, and an app that waits with nothing to do MUST have no frame.

#### Scenario: Dumps are not measured
- **WHEN** the same app runs with the script `frame`, `dump` and `dump`
- **THEN** the report's first line starts with `frame stats: frames 1, events 0, passes 1, `, as for the script `frame` alone

### Requirement: Frame Statistics Report
When a measured app returns from `runApp`, after its composition is dropped, it MUST print to stderr lines starting with `frame stats: `: first the counts of frames, input events, passes and reloads, and when the first frame was ready (its first composition and layout done) after `runApp` and after the run started; then, for `events`, `reload` (only after reloads), `recomposition`, `layout`, `display list` and `frame`, the median, 99th percentile and maximum per frame in milliseconds, or `none`.

#### Scenario: The lines of a report
- **WHEN** a test of `run_cli_tests.rs` runs the counter screen headless with `--frame-stats` and ten rounds of `click "+"` and `frame`
- **THEN** stderr holds, in this order, lines starting with `frame stats: frames 10, `, `frame stats: events median `, `frame stats: recomposition median `, `frame stats: layout median `, `frame stats: display list median `, `frame stats: frame median `, `frame stats: allocations per frame median ` and `frame stats: live allocations after each frame from `, and no `reload` line

### Requirement: The Report Counts Allocations per Frame
After its stage lines, the report MUST give the median and maximum number of allocations of the program thread per frame and how many frames allocated nothing, and then the smallest and largest number of live allocations of the program thread after a frame.

#### Scenario: Allocations of a counter
- **WHEN** a test of `run_cli_tests.rs` runs the counter screen headless with `--frame-stats` and the script `frame`, `click "+"` and `frame`
- **THEN** the report's last two lines start with `frame stats: allocations per frame median ` and `frame stats: live allocations after each frame from `

### Requirement: Measuring Frames Costs Nothing When Off
Without frame statistics, the frame loop MUST do no timing and the `prism` binary MUST count no allocation. With them, measuring MUST allocate nothing after `runApp` has started, so that the allocations a report shows are the app's own.

#### Scenario: The statistics allocate nothing
- **WHEN** a Rust test of `prism-ui` drives an app with a counting allocator through 100 frames with frame statistics on, whose handlers write nothing
- **THEN** no allocation happens after the first frame, as with frame statistics off

### Requirement: Script Targets by Kind
Wherever an event script accepts a text target, it MUST also accept `<kind> [<n>]`: the n-th node, 1 by default and in tree order, of kind `column`, `row`, `box`, `text`, `spacer` or `textfield`, or whose `clickable` or `toggleable` element has the role `button` or `checkbox`. The command MUST act at the centre of that node's bounds, rounded down, hit-testing from there.

#### Scenario: Clicking the second checkbox
- **WHEN** a headless app of 400 × 300 composes `compose fun Two() { val a = remember { mutableStateOf(false) }; val b = remember { mutableStateOf(false) }; Column { Checkbox(checked = a.value, onCheckedChange = { a.value = it }); Checkbox(checked = b.value, onCheckedChange = { b.value = it }) } }`, and its script is `click checkbox 2` then `dump`
- **THEN** stdout is the lines `window 400x300`, `  column #1 0,0 20x40`, `    box #2 0,0 20x20 toggleable(Checkbox off) size(20x20) border(2 #FF49454F RoundedCorner(2))` and `    box #3 0,20 20x20 toggleable(Checkbox on) size(20x20) background(#FF6750A4 RoundedCorner(2)) border(2 #FF6750A4 RoundedCorner(2))`

#### Scenario: Focusing a field by its kind
- **WHEN** the typing app of `ui`'s "Typing into a text field" runs headless with the script `click textfield`, `type "ab"` and `dump`
- **THEN** the dump holds `textfield #2 0,0 200x36 "ab" focused defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)`

### Requirement: Kind Target Errors
Another word in a target's place MUST be reported before the program runs as `Error: <script>:<line>: unknown target '<word>'`, with status 1. A kind target that matches fewer than `n` nodes MUST end the program with `event script line <line>: the tree has no <kind> <n>`.

#### Scenario: A missing kind target
- **WHEN** the counter screen runs headless with the script `click checkbox`
- **THEN** stderr is exactly `panic: event script line 1: the tree has no checkbox 1` and a newline, and the exit status is 101

#### Scenario: An unknown target word
- **WHEN** `prism run --headless --events events main.prism` runs an app and `events` holds `click slider` on its first line
- **THEN** stderr is exactly `Error: events:1: unknown target 'slider'` and a newline, nothing runs, and the exit status is 1
