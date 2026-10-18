# Design: reactive-app-verification

## Context

See `proposal.md` for motivation. This is item 33 of "Then: the road to Milestone 2" in `ROADMAP.md`, the last item of Phase 4 and the second half of the development-loop layer of `docs/architecture.md` §5. It implements the requirement "Reactive App Verification" and `compose-architecture` D24 ("Verification by scripted apps and measured frames"), keeps D25's scope, and closes Milestone 2. It adds no language feature and no compiler stage. What it adds is mostly outside the compiler: three apps, the sessions that verify them, two measurements (live runtime objects and frame statistics), a script target, benchmarks, a manual checklist and the documentation pass. It fills the seams the earlier packets left for it:

- `compose-runtime` D13: the allocation check's finish report, which counts undisposed compositions and leaves "groups, cells and nodes" to this change;
- `state-handles` D11 and D12: cells owned by `remember` slots, compositions and the program, program cells released before the check, and no live count of cells yet;
- `ui-toolkit` D15, D16 and D19: the frame loop, the event script grammar and dumps, the headless host, `ui_run_timing.rs` without a target, and node release with the composition;
- `desktop-window` D1 and D19: `WindowStatistics`, kept by the window server for `--frame-stats`, and its raster targets;
- `compose-hot-reload` D1, D12 and D13: `edit` and `wait reload` in event scripts, `// watch:` with `// events:` in fixtures, and frame statistics left to this change.

The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Fast compilation is a primary criterion (user, 2026-10-19). This change adds no compiler work, so each decision states its compile-time cost, which is none, and its run-time cost, which is none when the new options are off.

**Baseline.** This change was proposed while `standard-library` was being implemented and `compose-runtime`, `compose-transform`, `state-handles`, `ui-toolkit`, `desktop-window` and `compose-hot-reload` were proposed but not implemented. It needs all of them archived. It takes these names from their packets, which task 1.1 checks against the code they leave:

- **`compose-runtime`:** `prism_runtime::set_finish_report(fn() -> Option<String>)`, `finish_tracking`'s order (top-level values, the report, the leak line), `prism_compose::live_compositions_report`, the group arena and the slot records of `group.rs` and `slot.rs`.
- **`state-handles`:** the cell arena and `prism_compose::finish`, which releases program cells and then runs the composition report.
- **`ui-toolkit`:** `crates/prism-ui/src/{app.rs, host.rs, script.rs, headless.rs, dump.rs, node.rs}`, `run_app` and its loop, `Host`, `HostEvent`, `HostConfiguration`, `ScriptCommand`, the dump's kind names, `PanicKind::EventScriptFailed`, `crates/prism-cli/tests/ui_run_timing.rs`, and the `// events:` and `// headless:` fixture headers.
- **`desktop-window`:** `crates/prism-desktop/src/statistics.rs::WindowStatistics`, `WindowHost`, the frame mailbox, `--screenshot`, and the manual checklist of its tasks (group 8).
- **`compose-hot-reload`:** the `edit` and `wait reload` script commands, the `--watch` and `--events` combination, and `// watch:` with `// events:` in `fixture_tests.rs`.
- **`standard-library`:** the bundled `/prism/collections` helpers on `List<T>` (`indexOfFirst`, `count`), and `removeAt` among the list members.

The deltas are ADDED requirements of `execution`, `ui` and `desktop`, and a new `example-apps` capability. If `ui-toolkit`'s and `desktop-window`'s capabilities are archived when this change is applied, as the prerequisites require, task 1.1 checks the ADDED requirements against them for overlaps.

## Goals / Non-Goals

**Goals:**
- Apps that a Jetpack Compose programmer reads without a guide, using every part of the toolkit the milestone built.
- Every behaviour of those apps pinned by a session under the allocation check, run by `cargo test`, from the apps' own source.
- "Zero GC pauses" stated as properties that a run reports and a benchmark asserts.
- No cost to programs that do not ask for statistics.
- A repository that says, in its README, ROADMAP and AGENTS.md, that Milestone 2 is done and what it does, with a review document for the decisions taken on the user's behalf.

**Non-Goals:**
- New language features or toolkit composables. A gap the apps hit is a plain fix when the spec already implies the right behaviour, and otherwise a follow-up written around (D3).
- Effects, animation, lazy lists, scrolling, a GPU renderer, `prism test` (proposal, "Not in scope").
- Making the apps fast by restructuring them around the runtime's costs. The apps are written as a Compose programmer would, and the targets are met by the runtime (D9).

## Decisions

### D1. The boundary with the other changes (autonomous)

| Concern | `reactive-app-verification` (this change) | Elsewhere |
|---|---|---|
| Example programs | the apps, the tour, their sessions (D2 to D4) | the fixtures of each packet pin single rules |
| Script grammar | targets by kind (D5) | the grammar is `ui-toolkit` D16; `edit` and `wait reload` are `compose-hot-reload` D12 |
| Allocation check | live groups, slots, cells and nodes (D6) | undisposed compositions are `compose-runtime` D13 |
| Frame measurement | `--frame-stats` and its report (D7) | `WindowStatistics` is `desktop-window` D19; `--timings` of reloads is `compose-hot-reload` D13 |
| Targets | the apps' frame, allocation and startup targets (D9) | the raster targets are `desktop-window` D19; the reload latency is `compose-hot-reload` D13 |
| Documents | the pass that closes Milestone 2 (D13, D14) | each archive's own ROADMAP and architecture edits |

Rejected: folding the live counts into `compose-runtime` and `ui-toolkit` (they were left here on purpose, `compose-runtime` D13), and folding the documentation pass into the last archive (it needs this change's measurements).

### D2. Apps under `examples/apps/<name>/`, each a package; the tour replaces `examples/demo/ui` (autonomous)

`examples/` is already a workspace root: `examples/run/main.prism` is `package run`, and `examples/demo/models/models.prism` is `package demo.models`. The apps join it:

| Directory | Package | Files | Entry |
|---|---|---|---|
| `examples/apps/counter/` | `apps.counter` | `main.prism` | `main.prism` |
| `examples/apps/todo/` | `apps.todo` | `model.prism`, `screen.prism`, `main.prism`, `large.prism` | `main.prism`; `large.prism` for measurements |
| `examples/demo/ui/` | `demo.ui` | `ui.prism` | `ui.prism` |

- **The todo app is four files** because a real app is, and because it exercises what single-file compilation promises: `model.prism` holds data and plain functions with `package` visibility and explicit types, `screen.prism` the composables, and the entries only `main`. A hot-reload edit of the row layout in `screen.prism` then reaches `main.prism` through `compose-hot-reload` D4's importer rule. `main` is private, so two entries in one package do not clash.
- **`large.prism`** composes the same `TodoScreen` over 1,000 sample items. It is what the benchmark (D10) and the window checklist (D12) run, so a measurement is always of the shipped screen, never of a copy.
- **The tour.** `examples/demo/ui/ui.prism` keeps its path and package, since `fixture_tests.rs` and the formatter corpus already scan it, and becomes a runnable screen that shows every `/prism/ui` composable, modifier, colour, shape, arrangement and alignment once. `ui-toolkit` task 12.2 only moved it to the new API.
- **`prism check examples/`, `prism fmt --check examples/`** and the example tests of `fixture_tests.rs` cover the new files with no change. Session files (`.events`, `.snap`, `.edits`, `.prism.v<k>`) are not `.prism` sources, so neither the checker nor the formatter reads them.

Compile-time cost: none; the apps are ordinary files. Kotlin-likeness: one directory per app with a `main`, as a Gradle module would have.

Rejected:
- **Apps only as fixture cases** under `crates/prism-cli/tests/fixtures/`: users would not find them, and an app split across cases would be copied.
- **A one-file todo app:** shorter, but it tests none of the cross-file paths a real app uses.
- **Deleting `examples/demo/ui`:** it would remove the exemption too, but a page that shows every toolkit feature once is useful next to two apps that each use a few.
- **`examples/counter/` at the root of `examples/`:** it mixes apps with the language demos.

### D3. The apps are written as Compose code, and gaps are fixed or written around (autonomous)

The apps use Compose's idioms where Prism has them, and the idioms the ownership model asks for where it does not. The counter, in full:

```
package apps.counter

import /prism/ui

compose fun Counter() {
    val count = remember { mutableStateOf(0) }
    Column(modifier = Modifier.padding(16), verticalArrangement = Arrangement.spacedBy(8)) {
        Button(onClick = { count.value += 1 }) {
            Text("Count: ${count.value}")
        }
        Button(onClick = { count.value = 0 }, enabled = count.value > 0) {
            Text("Reset")
        }
    }
}

fun main() {
    runApp(title = "Counter", width = 320, height = 200) { Counter() }
}
```

The todo app, in outline (the implementer writes it in the implemented syntax, `prism fmt` decides its layout):

- `model.prism`: `struct Todo { val id: Int, val title: String, var done: Bool }`; `enum Filter : Copy { All, Active, Done }` with `fun Filter.shows(todo: &Todo): Bool`; a state holder `struct TodoState : Copy { val todos: MutableState<List<Todo>>, val draft: MutableState<String>, val filter: MutableState<Filter>, val nextId: MutableState<Int> }`, built by `fun todoState(items: List<Todo>): TodoState`; and its operations as extensions: `add()` (`draft.read { title -> ... todos.edit { it.add(Todo(id, title.clone(), false)) } }`, then `draft.value = ""`, ignoring an empty draft), `setDone(id, done)` and `remove(id)` (`todos.edit { list -> val index = list.indexOfFirst { it.id == id }; ... }`), `activeCount()` (`todos.read { list -> list.count { !it.done } }`), and `sampleTodos(count)`.
- `screen.prism`: `compose fun TodoApp() { val state = remember { todoState(listOf()) }; TodoScreen(state) }`; `TodoScreen(state: TodoState)` lays out a `Row` of `TextField(value = state.draft.value, onValueChange = { state.draft.value = it }, modifier = Modifier.weight(1.0))` and `Button(onClick = { state.add() }) { Text("Add") }`, then `FilterRow(state.filter)`, then `for (todo in state.todos.value) { if (state.filter.value.shows(todo)) { key(todo.id) { TodoRow(todo, state) } } }`, then `Text("${state.activeCount()} left")`; `FilterRow` shows three `Button`s, the selected one disabled; `TodoRow(todo: &Todo, state: TodoState)` is a `Row` of `Checkbox(checked = todo.done, onCheckedChange = { state.setDone(id, it) })`, `Text(todo.title, modifier = Modifier.weight(1.0))` and `Button(onClick = { state.remove(id) }) { Text("Remove") }`, with `val id = todo.id` first.
- `main.prism`: `fun main() { runApp(title = "Todo", width = 480, height = 640) { TodoApp() } }`. `large.prism`: a `LargeTodoApp` remembering `todoState(sampleTodos(1000))`, in an 800 × 600 window.

What the ownership model asks for, each a known consequence of an archived decision rather than a gap:
- `val id = todo.id` before the handlers: an escaping closure owns its captures and cannot hold the borrowed `todo` (`escaping-closures`), so it captures the `Int`.
- `read`, `edit` and `update` in handlers instead of `todos.value` (`state-handles` D5, D6).
- One `.clone()` of the draft in `add`, because a closure cannot move a capture (`closures`, `E4017`) and the draft is reset anyway.
- A `Copy` state holder of handles instead of four `remember`s threaded through every call, which is Compose's state hoisting with a `rememberXState`-style holder.

When the apps hit a bug where a spec implies the right behaviour, the implementer fixes it as a plain fix with a regression fixture (AGENTS.md section 1), in its own commit. When they hit a missing feature, the app is written around it, and the gap goes to `ROADMAP.md` as a follow-up naming this change. Nothing here may add a language rule.

Compile-time cost: none. Kotlin-likeness: the counter is Compose's own sample with `count.value` for `count`'s delegated property, which Prism does not have.

Rejected:
- **Delegated properties (`var count by remember { ... }`)** for the apps: a language feature, out of scope.
- **Apps tuned to the runtime**, for instance a `MutableState<Bool>` per item so that a toggle recomposes one row: it would meet the targets more easily, but it would verify a program nobody writes first (D9).

### D4. App sessions live beside the apps and run as fixtures (autonomous)

Fixture cases are workspaces of their own (`fixture_tests.rs`: "the case directory is the workspace root"), so they cannot run `examples/apps/todo` without a copy. Sessions therefore live with the apps:

```
examples/apps/todo/sessions/
  adding-toggling-and-removing.events
  adding-toggling-and-removing.stdout.snap
  editing-the-row-layout-keeps-the-list.events
  editing-the-row-layout-keeps-the-list.stdout.snap
  editing-the-row-layout-keeps-the-list.stderr.snap
  editing-the-row-layout-keeps-the-list.screen.prism.v2
  scripted.edits
```

- **A session** is an event script. Its leading comment lines are headers that `prism` ignores (`#` starts a comment, `ui-toolkit` D16): `# entry: <path>`, required, relative to the script, and `# headless: <width>x<height>`, optional.
- **The test** `example_app_sessions` in `crates/prism-cli/tests/fixture_tests.rs` finds every `sessions` directory under `examples/` and every `.events` file in it, and spawns `prism run --check-allocations --headless[=<size>] --events <script> <entry>` with `examples/` as working directory, so status lines show workspace paths. A session's expected status is always 0, and its stdout and stderr are compared with `<name>.stdout.snap` and `<name>.stderr.snap`, an absent file meaning empty output. `PRISM_BLESS=1` writes them, as for executable cases, and the same cache setup is used.
- **Watch sessions.** A script with an `edit` line runs with `--watch --edits sessions/scripted.edits` added. That edit script holds only a comment saying that the event script's `edit` commands drive the reloads, so the session watches no file and ends when `main` returns (`compose-hot-reload` D12). The `edit` targets are relative to the script, `../screen.prism`, and the replacements are `<name>.<file>.v<k>` beside it. Task 4.1 checks that a `..` target is accepted, and fixes it as a plain fix if not, since the edit script's rule ("relative to the script's directory") implies it.
- **Traceability.** Every session's name must be the slug of a scenario of the `example-apps` spec (this change's delta while it is open), and every `example-apps` scenario whose WHEN names a session must have one. The test checks both, as `fixture_tests.rs` checks case names.
- **The tour** gets one session, so `examples/demo/ui/sessions/` follows the same rule.

Compile-time cost: none. Run-time cost: one `prism run` per session in `cargo test`, about a dozen.

Rejected:
- **Copying the apps into fixture cases:** two copies drift, and a test comparing them would be the same machinery with more files.
- **A fixture header pointing outside its case** (`// entry: ../../../../examples/...`): it breaks "the case directory is the workspace root", which every fixture assertion relies on.
- **Symbolic links:** not portable (`UNFIXTURED_CODES`' note on `E1005`).
- **Snapshots of the apps' dumps in Rust tests:** harder to bless and to read than files beside the scripts.

### D5. Event scripts target the n-th node of a kind (autonomous; extends `ui-toolkit` D16)

The todo app's checkboxes show no text, and an empty `TextField` shows none either. Scripts could click them by coordinates (`click 10 58`), but every layout edit, a padding or a row height, would then break every session, which is what `ui-toolkit` D16 chose text targets to avoid. So a target may also be `<kind> [<n>]`: the n-th node, 1 by default and in tree order, whose kind is `<kind>`, one of the kinds `dump` prints (`column`, `row`, `box`, `text`, `button`, `spacer`, `checkbox`, `textfield`). It is accepted wherever a text target is: `click checkbox 2`, `press button 1`, `click textfield`. The command acts at the centre of the node's bounds and hit-tests from there, as for text targets.

- An unknown kind word is a parse error before the program runs: `Error: <script>:<line>: unknown target '<word>'`, status 1.
- A kind target that matches fewer than `n` nodes ends the program with `event script line <n>: the tree has no <kind> <k>`, status 101, like a missing text target.

The forms do not overlap: a text target is quoted, a position is two integers, and a kind is an identifier.

Compile-time cost: none. Run-time cost: one tree walk per command, as for text targets.

Rejected:
- **Coordinates:** fragile, as above.
- **Node numbers (`click #7`):** numbers follow creation order, so adding an item renumbers nothing but removing and re-adding does, and a session would encode the history of the tree.
- **A test tag modifier (`Modifier.testTag(...)`):** Compose's answer, but a tag is a string, which a `Copy` `Modifier` cannot hold (`ui-toolkit` D6), and an `Int` tag would read nothing like Compose.
- **Targets relative to a text (`click checkbox near "Milk"`):** more grammar for what `checkbox <n>` and a dump already say.

### D6. The allocation check also reports live groups, slots, state cells and nodes (autonomous; implements the counting of `compose-architecture` D24)

Groups, slots and cells live in `prism-compose`'s per-thread arenas and nodes in `prism-ui`'s. Their records are Rust memory, which the program's allocation map never sees. A runtime bug that forgets to release a group, a slot or a node, or a cell owned by nothing, would leave every program allocation freed, so `--check-allocations` would pass. The check therefore also counts them.

- **When.** In `finish_tracking`'s sequence, after program cells are released and after the composition report (`state-handles` D11, `compose-runtime` D13), and before the allocation line.
- **What.** If no composition is live, each non-zero count is a line, in this order: `allocation check: groups still live at exit: <n>`, `allocation check: slots still live at exit: <n>`, `allocation check: state cells still live at exit: <n>`, `allocation check: nodes still live at exit: <n>`. Any of them makes the status 102. When a composition is live, its groups, slots, cells and nodes are expected to be live, so only the composition line is printed, and `compose-runtime`'s "A composition that is never disposed" scenario keeps its exact stderr.
- **How.** Each arena keeps a live count, incremented on insert and decremented on release, read on the program thread, which is the thread that runs `finish` (`compose-runtime` D13). `prism-compose` gains `live_objects_report()` and `prism-ui` `live_nodes_report()`. `prism-codegen`'s `Program::run`, which links both crates, installs one finish report that runs the composition report, then these two.
- **Tests.** No correct program can make these lines appear, so they are pinned by Rust tests that leak a group, a slot, a cell and a node through test-support hooks and run the report. Every session and every UI fixture already asserts their absence, since each runs under the check.

Compile-time cost: none. Run-time cost: one integer update per insert and release, and one read per arena at exit under the check.

Rejected:
- **Counting everything even when a composition is live:** an undisposed composition would print four more lines and change `compose-runtime`'s scenario, for no information.
- **Counting only nodes,** since groups and cells are freed with them: the bug most worth catching is a release path that forgets an arena.
- **Routing arena records through the program allocator** so that the map sees them: it would make every runtime allocation pay the map's cost under the check and blur the line between program and runtime memory.

### D7. `prism run --frame-stats` reports each app's frames when it returns (autonomous)

**A frame** is the work the program thread does for one presented image: everything from `runApp`'s start, or from the end of the previous present, up to and including the next present, so the first frame holds the first composition and layout. In a window that is the iterations of the frame loop between two `present` calls (`ui-toolkit` D15). Headless, the script's `frame` command plays the present, and work after the last `frame` forms a last frame when the app ends. A frame's time is the busy time of its iterations: waiting for events is not counted, and neither are the script's `dump` commands, whose time and allocations are excluded, since they are instrumentation. Iterations that only dispatch an event without a pass or layout, a press for instance, belong to the frame they precede. So an idle app has no frame, which is "Idle Apps Do No Work".

**Stages** are timed inside each frame: `events` (hit testing and handlers), `reload` (the reload point, `compose-hot-reload` D2), `recomposition` (every pass), `layout`, `display list` (building it) and `frame` (the whole). In a window, the window server adds `raster` (rasterizing and presenting a frame on the main thread) and `input to present` (from the program thread taking the first event of a frame to the main thread presenting it), kept in `WindowStatistics` (`desktop-window` D19). A frame that the latest-wins mailbox replaced before it was drawn has no raster sample.

**The report** goes to stderr when `runApp` returns, after its composition is dropped, one line per item, each starting with `frame stats: `:

```
frame stats: frames 41, events 80, passes 41, reloads 0, first frame ready 1.912 ms after runApp, 14.208 ms after start
frame stats: events median 0.006 ms, p99 0.011 ms, max 0.012 ms
frame stats: recomposition median 0.031 ms, p99 0.052 ms, max 0.060 ms
frame stats: layout median 0.009 ms, p99 0.014 ms, max 0.015 ms
frame stats: display list median 0.004 ms, p99 0.006 ms, max 0.007 ms
frame stats: frame median 0.056 ms, p99 0.081 ms, max 0.090 ms
frame stats: allocations per frame median 2, max 31, none in 1 of 41 frames
frame stats: live allocations after each frame from 118 to 118
```

- `events` counts input events (presses, releases, characters, keys and resizes), not the script's `frame` and `dump` commands.
- A stage with no sample prints `none` after its name, and `reload` is printed only when reloads were applied. In a window, `raster` and `input to present` follow `frame`, with `(<k> frames presented)` after `raster`.
- "first frame ready" is the time from `runApp`'s call to the end of the first composition and layout, and from `prism run`'s start, or the start of the run for later runs of a watch session.
- Times are milliseconds with three decimals; percentiles are nearest-rank.
- The report never changes stdout or the exit status. Under `--watch`, each `runApp` of each run prints its own report. A program that never calls `runApp` prints nothing.

**Measuring allocates nothing.** Each stage keeps a fixed log-linear histogram (two significant digits, from 1 µs to 100 s, about 1,700 counters), allocated once when `runApp` starts, so the report's own bookkeeping never shows up in "allocations per frame". Maxima are exact, and medians and percentiles are within 1 %.

**Counting allocations.** `prism-runtime` gains `allocation_counter.rs`: `CountingAllocator`, a `GlobalAlloc` over `System` that, when its `AtomicBool` is set, adds one to the current thread's allocation or free count in a `const` thread-local `Cell<u64>` (a reallocation counts as one of each). The `prism` binary declares it as its `#[global_allocator]` in `main.rs`, so library tests and other embedders are unaffected, and `--frame-stats` sets the flag before the program thread starts. A frame's allocations are the program thread's count at its end minus at its start, and its live allocations are allocations minus frees on the program thread since it started. Rust allocations of the runtime and program allocations both go through it. Under `--check-allocations`, the allocation map's own growth is counted too, so the benchmark runs without that option.

**Cost when off.** One relaxed atomic load per allocation in the `prism` binary, and one predictable branch per stage in the frame loop. When on: two `Instant::now()` per stage and two thread-local reads per frame.

Compile-time cost: none.

Rejected:
- **A frame per loop iteration:** a click would be two frames (press and release), a headless `frame` command a third, and the medians would mostly measure presses.
- **Sample vectors and sorting at the end:** exact, but growing a vector allocates inside frames, which is exactly what the report must not do.
- **A periodic report or a live overlay while the app runs:** useful for tuning, and a follow-up. The report at the end is what tests and benchmarks need.
- **Counting only program allocations through the allocation map:** it needs `--check-allocations`, whose map dominates the timings, and misses the runtime's own allocations, which a GC-free claim must cover too.
- **`--timings` printing frames:** `--timings` reports compilation stages before `main` and reload stages, and mixing per-frame numbers into it would make both harder to read.

### D8. "Zero GC pauses" means three measured properties (autonomous)

Prism has no collector by construction (the zero-cost memory invariant), so "zero GC pauses" cannot be shown by the absence of a collector's log. What a user feels as GC pauses are: memory reclaimed later in a burst, work done while nothing changes, and frames that are occasionally much slower than the others. Each becomes a property that `--frame-stats` reports and a benchmark asserts:

1. **No deferred reclamation.** Every value an app drops is freed during the frame that drops it. Measured as the live allocations after each frame: over a repeated click or toggle, the line reads `from N to N`. Every session also ends with the allocation check (D6) reporting nothing.
2. **No work and no allocation when nothing changes.** An idle app runs no frame (`ui-toolkit` "Idle Apps Do No Work"). A frame whose events invalidate nothing allocates nothing, for instance a click on empty space followed by `frame`, which `ui-toolkit`'s steady-state allocation test already pins inside the toolkit and which this change measures through the whole program.
3. **Bounded frames.** The 99th percentile and the maximum are reported with the median, and the targets of D9 bound the 99th percentile, which a collector's pause would break first.

Rejected:
- **Only a frame-time target:** a fast app can still accumulate garbage that a later frame frees.
- **Asserting the maximum frame time:** on a developer machine the maximum measures the operating system's scheduler as much as Prism. It is reported, and the 99th percentile is asserted.

### D9. Targets: 1 ms per counter click, 4 ms per toggle of 1,000 items headless, 16 ms in a window (autonomous; refines §5 "Reactive App Verification")

On a release build, measured by `--frame-stats` (D7) through the benchmark of D10:

| App and action | Measure | Target |
|---|---|---|
| counter, headless, `click button 1` then `frame` | `frame` median, p99 | under 1 ms, under 2 ms |
| todo with 1,000 items (`large.prism`), headless, `click checkbox 2` then `frame` | `recomposition` median | under 4 ms |
| the same | `frame` p99 | under 4 ms |
| the same, in an 800 × 600 window at the display's scale | `input to present` p99 | under 16 ms (manual, D12) |
| both apps, a click on empty space then `frame` | allocations of those frames | zero |
| counter clicks and todo toggles, repeated 100 times | live allocations after each frame | constant |
| todo app (`main.prism`), headless | first frame ready after start, cold and warm | under 50 ms and under 20 ms |

- **Where §5's numbers come from and what changed.** §5 asked for 1 ms per counter click, 4 ms of recomposition and 16 ms of frame time at 800 × 600 for the todo toggle. This change keeps them and adds a headless frame 99th percentile under 4 ms, which together with `desktop-window`'s raster target (a 1,000-item todo column under 4 ms at scale 2) leaves the 16 ms window budget with room to spare. It adds the allocation properties of D8 and a startup target, since fast compilation is a primary criterion and the first frame is what a developer waits for. The 16 ms window target is measured by hand, because `--events` cannot drive a window (`desktop-window`).
- **What a toggle costs.** A toggle edits `todos`, whose only reader is `TodoScreen`, the nearest restart scope. `TodoScreen` reruns its loop, and every `TodoRow` reruns, because a borrowed `&Todo` parameter is never stable (`compose-transform` D7). That is about 5,000 groups, 3,000 setter calls that find nothing changed except one, and two handler closures per row whose environments of two or more words are allocated, with the replaced ones freed. Layout is clean except for one checkbox, and the display list is rebuilt. The allocation count per toggle is therefore proportional to the visible list, all freed in the same frame (D8, property 1). Making rows skippable needs stable borrowed parameters, a follow-up.
- **If a target is missed,** the implementer profiles and fixes the runtime (plain fixes, or a follow-up when the fix is a design change). Only a target shown unreachable by design is revised, with the measurements, in `docs/decisions-for-review.md`.

Compile-time cost: none; the startup target bounds the compiler's cost on the apps.

Rejected:
- **Median-only targets** (§5's first wording): they hide outliers (D8).
- **Targets on the window only:** not reproducible in `cargo test`, and dominated by rasterization, which `desktop-window` already measures.
- **Restructuring the todo app so a toggle recomposes one row:** see D3.

### D10. One app benchmark reads `--frame-stats` (autonomous)

`crates/prism-cli/tests/app_frame_timing.rs` is an ignored benchmark in the style of `run_timing.rs`. On a release build (`cargo test --release -p prism-cli --test app_frame_timing -- --ignored --nocapture`), it:

- writes generated event scripts to a temporary directory: 200 rounds of `click button 1` and `frame` for the counter, 100 rounds of `click checkbox 2` and `frame` for `large.prism` (toggling the same item on and off), and 50 rounds of a click on empty space and `frame` for both;
- runs `prism run --frame-stats --headless --events <script> <entry>` on each, five times, and parses the `frame stats:` lines;
- runs `prism run --frame-stats --headless` on the todo app's `main.prism` twenty times cold (`PRISM_CACHE=off`) and twenty times warm, and takes "first frame ready after start";
- prints every number of D9, the median of the five runs' medians and the largest 99th percentile, and fails on the targets only under `PRISM_ASSERT_TIMING=1`.

`ui-toolkit`'s `ui_run_timing.rs` (a generated app of 1,000 `Text`s and a counter, 100 clicks) takes the click target of D9, under 1 ms median, under the same flag. The measured numbers are recorded in this design at apply time.

A Rust test in `run_cli_tests.rs`, not ignored, runs the counter with `--frame-stats` and ten clicks in a debug build and checks the report's shape: the line prefixes, the counts (frames, events, passes), "none in" for frames that change nothing, and `from N to N`. Those parts do not depend on timing.

Rejected: timing with wall clocks around `prism run` (it would measure compilation and process start with each frame); a Criterion harness (a new dependency for a handful of numbers).

### D11. Hot-reload demos are sessions of the apps (autonomous)

Two watch sessions (D4) show that state survives the edits a UI developer makes most:

- **The counter's label** (`relabelling-keeps-the-count`): three clicks, then `edit ../main.prism relabelling-keeps-the-count.main.prism.v2`, which changes `"Count: ${count.value}"` to `"Clicks: ${count.value}"`, then `dump`, which shows `Clicks: 3`, then one more click, which shows `Clicks: 4`.
- **The todo row's layout** (`editing-the-row-layout-keeps-the-list`): three items added, the second toggled, a draft typed but not added, then an edit of `screen.prism` that puts the title before the checkbox, adds `Modifier.padding(vertical = 4)` to the row and greys the titles of done items, then `dump`. The dump shows the same three items in the new layout, the second still checked, the field still focused with the draft, and the nodes kept their numbers (`compose-hot-reload` D10). Typing on and adding then works on the new code.

Both are also steps of the manual checklist in a window (D12). Their stderr snapshots pin the session's `reload: applied ...` lines. Changing a remembered type, reloads with errors and handlers whose captures changed are already pinned by `compose-hot-reload`'s fixtures and are not repeated.

Rejected: a reload demo that changes a remembered type in the apps (pinned elsewhere, and it resets state on purpose, which is not what a demo of kept state should show).

### D12. A manual checklist in a window, recorded with frame statistics and screenshots (autonomous)

What no test can drive (`desktop-window` D20) is checked by hand on a release build, and recorded in a "Manual verification" section of this design with the date, the machine, the display's scale factor and the `frame stats:` lines of each run. The checklist (tasks, group 8):

- opening the counter, the todo app and the tour in windows, each titled and sized as its `runApp` says;
- clicking, including a press released outside a button;
- typing into the todo field, Backspace, adding with the button;
- toggling, removing and filtering;
- resizing the window live, and moving it between displays if two are available;
- a hot-reload edit under `prism run --watch` of the counter's label and of the todo row's layout while the window keeps its state;
- an idle window using no CPU for 10 s;
- `large.prism` in an 800 × 600 window, toggling about 20 items, with its `input to present` 99th percentile under 16 ms (D9);
- closing each window, and the allocation check passing.

Screenshots of the counter after three clicks and of the todo app with three items, one checked, are written by the windowed runs with `--screenshot docs/images/counter.png` and `docs/images/todo.png`, at the display's scale. `README.md` shows them. They are not compared by any test (§5 D21).

Rejected: screenshots from headless runs only (scale 1, fixed-advance text, not what a user sees, `desktop-window` D7); a screen recording (large, and a review needs the numbers more than a video).

### D13. The documentation pass that closes Milestone 2 (autonomous)

The last groups of tasks, run after every other task, edit only documents; the `ROADMAP.md` and `docs/architecture.md` edits are made at archive, as the archive guidance of `openspec/config.yaml` asks:

- **`README.md`** gains "What works today" after the tenets: what a program can do in Milestone 2 (the language features of the specs, `prism check`, `fmt`, `run`, `--watch`, the compose engine and the toolkit), the three commands that run the apps in a window, headless and under `--watch`, the screenshots, and the known limits (no scrolling or lazy lists, no effects or animation, one window, CPU rendering, no shaping, Tier 1 only). The tenets stay the vision, and the section links to the specs for details.
- **`ROADMAP.md`**: item 33 and the Phase 4 items are ticked; the Phase 4 heading gains "(Completed: Milestone 2, <date>)"; "(Current)" moves from Phase 2 to Phase 5, and any unchecked follow-up stays listed under its phase; every follow-up of this change is added, naming it.
- **`AGENTS.md`**: section 4 says that Milestones 1 and 2 are complete and Milestone 3 is the current focus, and the spec list of section 3 gains `example-apps` and any capability of Phase 4 that its archive did not add (`standard-library`, `compose`, `ui`, `desktop`).
- **`docs/architecture.md`** §5: "Reactive App Verification" is removed, with the rest of the section's requirements if earlier archives left any by mistake; the design notes stay, under a line saying that §5 is implemented and the notes are kept for its rationale.
- **`openspec/config.yaml`**: see D15.
- **`docs/decisions-for-review.md`**: see D14.

Rejected: splitting the pass into a separate change (it is the end of this item, and it needs this change's measurements); editing the documents before the review (the numbers would not be final).

### D14. `docs/decisions-for-review.md` gets a review summary at its top (autonomous)

The document holds almost 500 rows. The user asked for decisions to be logged so they could be reviewed once Milestone 2 is complete, and nobody reviews 500 rows in order. So the last task adds a section "Review summary" between the introduction and "Process decisions", written after every change of the road is archived:

- **How to review**, in three sentences: start with the summary, follow each link to its row and its design, and turn a decision you would take differently into a change.
- **High revert cost**: every row marked high, one line each, with the change, the decision number and a link to the row.
- **Medium revert cost, by theme**: the medium rows that change what programs can write or mean, how memory and state behave at run time, or how the compiler is staged. At most about 30, grouped under "Language surface", "Ownership, borrows and memory", "Compiler architecture and compile speed", "Compose engine and UI" and "Tooling and process". Every other row stays in the tables.
- **Revisions**: decisions that revised or corrected an earlier one (rows whose text says "revises" or "corrects"), so the user sees where the plan changed.
- **Targets and measurements**: one table of every performance target set on the road (startup, compile timing shapes, reload latency, raster, frames), with the number each change measured.
- **Counts**: rows per revert cost, and the changes that logged them.

Revert costs are re-estimated while writing the summary, now that code exists, and a changed estimate is edited in its row with "(re-estimated <date>)".

Rejected: a separate review document (two places to keep in step); sorting the tables themselves by revert cost (they are ordered by change, which the design links rely on); summarizing only the high rows (four rows would miss most of what a reviewer would want to change).

### D15. The examples have no exemption any more

`openspec/config.yaml`'s apply guidance changes in two lines:

- "prism check on every file under examples/" loses "(only files needing an unbuilt subsystem, such as examples/demo/ui, may fail)";
- "Run the executable examples" adds: each UI entry (`examples/apps/counter/main.prism`, `examples/apps/todo/main.prism`, `examples/apps/todo/large.prism` and `examples/demo/ui/ui.prism`) runs with `prism run --check-allocations --headless <entry>` and exits with status 0, with no allocation check line; their sessions run in `cargo test` (D4). `prism run` without `--headless` on them would open a window, so the guidance never does that.

`fixture_tests.rs`'s `example_typed_trees_are_well_formed`, `example_mir_is_well_formed` and `example_elaborated_mir_is_well_formed` lose their `ui` exemptions, so every example, apps included, must give an error-free typed tree and well-formed MIR.

### D16. One new capability, three deltas, and how the change is tested

The apps' behaviour is a durable contract of the repository (what the examples do, and that they keep doing it), so it is a capability of its own, `example-apps`, rather than scenarios of `ui`. The allocation-check lines and the option are `execution`'s, as the composition line is. The report is `ui`'s, beside the frame loop it measures, and the window's lines `desktop`'s. No error code is added: the new script errors are details of `ui-toolkit`'s `EventScriptFailed` and of its parse errors.

- **Sessions** (D4) pin the `example-apps` scenarios of the apps' behaviour and hot reload. Its scenarios on the examples' rules are pinned by the example tests of `fixture_tests.rs` and the apply guidance, and those on frames and allocations by tests of `run_cli_tests.rs` and the benchmark.
- **Executable fixtures** under `crates/prism-cli/tests/fixtures/ui/` pin the kind targets ("Clicking the second checkbox", "Focusing a field by its kind", "A missing kind target", "An unknown target word").
- **Rust tests** pin the live-object report (D6), the histograms and the counting allocator (D7), and the report's shape (D10).
- **The ignored benchmark** (D10) and the manual checklist (D12) pin the targets.

## Costs

| Compile-time stage | Cost |
|---|---|
| everything in this change | none: no language change, no compiler stage, no new analysis |

| Run-time operation | Cost |
|---|---|
| allocation, `prism` binary, `--frame-stats` off | one relaxed atomic load |
| allocation, `--frame-stats` on | two thread-local updates |
| frame loop stage, off / on | one branch / two `Instant::now()` and a histogram increment |
| report | O(stages × histogram buckets) once per `runApp` |
| arena insert or release (D6) | one integer update |
| finish report under the check | one read per arena |
| kind target | one tree walk per command |

`compile_timing.rs` gains no shape, since no compiler stage changes. The startup target (D9) bounds the compiler's cost on the apps end to end.

## Risks / Trade-offs

- **[The apps hit a missing feature or a bug of an earlier change]** They are the first programs to combine everything. → Bugs are plain fixes with regression fixtures, gaps are written around and become follow-ups (D3). The task list puts the apps first, so these surface early.
- **[Rows that take `&Todo` never skip]** A toggle recomposes every row (D9). → The targets are set with that cost, and stable borrowed parameters are a follow-up.
- **[A global allocator in the `prism` binary]** Every allocation of the compiler pays one atomic load, and a `const` thread-local inside an allocator must never allocate. → `const` thread-locals with `Cell<u64>` need no lazy initialization on the supported platforms; task 3.1 checks it with a test that allocates on a fresh thread. The load is measured with `run_timing.rs` before and after, which must not grow by more than its noise.
- **[Benchmarks on developer machines are noisy]** → Medians of five runs, a 99th percentile rather than a maximum, assertions only under `PRISM_ASSERT_TIMING=1`, and a revision path for unreachable targets (D9).
- **[The 16 ms window target is checked by hand]** → It is recorded with the numbers, and the headless and raster targets bound it from below and above in `cargo test` and the ignored benchmarks.
- **[Session paths with `..`]** The `edit` targets leave the sessions directory. → Task 4.1 checks it, and it is a plain fix if refused (D4).
- **[The review summary is written last]** It depends on every other change being archived. → It is the last task, and its selection rules (D14) make it mechanical except for the themes.
- **[Prerequisites unimplemented]** Every name taken from the seven packets comes from their designs. → Task 1.1 checks them against the code.

## Migration Plan

No program changes meaning. `--check-allocations` can now fail on runtime bugs it missed before, which is its purpose. `examples/demo/ui/ui.prism` is rewritten. The apply guidance stops exempting it, and the example tests stop skipping `ui` directories. The `prism` binary gains a global allocator, which changes no output.

## Open Questions

- Should `--frame-stats` also report while an app runs, every second or on a key, for tuning windows by hand? The report at the end covers tests and the checklist (follow-up).
- Should rows that take a borrowed item become skippable, by keeping a clone of `Clone` arguments or a hash, as `compose-architecture` D9 left open? It would change the todo toggle's cost from the list's size to one row (follow-up).
