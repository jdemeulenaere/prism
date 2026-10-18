# Proposal: reactive-app-verification

## Why

After `standard-library`, `compose-runtime`, `compose-transform`, `state-handles`, `ui-toolkit`, `desktop-window` and `compose-hot-reload`, every layer of the compose engine exists and is tested on its own, mostly through small fixtures written for one rule each. Nothing yet shows that they add up to what Milestone 2 promises: real reactive apps, written as a Jetpack Compose programmer would write them, running natively in a window and headless, reloading while they keep their state, and doing so "with zero GC pauses". That last claim is not measured anywhere. `examples/demo/ui/ui.prism` still uses an API that never existed, and the apply guidance of `openspec/config.yaml` still lets it fail. This change is item 33 of "Then: the road to Milestone 2" in `ROADMAP.md`, the last item of Phase 4. It implements `docs/architecture.md` §5 "Reactive App Verification" and `compose-architecture` D24 (scripted apps and measured frames), keeps D25's scope (no effects, animation or GPU renderer), and ends with the documentation pass that closes Milestone 2.

## What Changes

- **Example apps** (D2, D3). Three runnable UI programs under `examples/`, written in idiomatic Compose-style Prism and kept formatted:
  - `examples/apps/counter/` (`package apps.counter`): `remember { mutableStateOf(0) }`, `Button(onClick = { count.value += 1 }) { Text("Count: ${count.value}") }`, and a reset button that is disabled at zero;
  - `examples/apps/todo/` (`package apps.todo`), in four files: a model (`Todo`, `Filter` and a `Copy` state holder of four handles), the screen's composables, `main.prism`, and `large.prism`, the same screen with 1,000 sample items for measurements. It holds a `MutableState<List<Todo>>`, adds items from a `TextField` and a `Button`, toggles them with a `Checkbox`, removes them, filters them with an "All / Active / Done" row, and composes each row in `key(todo.id)`;
  - `examples/demo/ui/ui.prism` is replaced by a runnable tour of the toolkit (every composable, modifier, colour, arrangement and alignment), so `prism check`, `prism fmt --check` and `prism run` pass on every file under `examples/`.
- **App sessions** (D4). Event scripts beside the apps (`examples/apps/<app>/sessions/<scenario>.events`, `examples/demo/ui/sessions/`) with their expected stdout, run by a new fixture test under `--check-allocations` and headless. Each session is named after a scenario of the new `example-apps` capability, and the apps are never copied into fixtures. Sessions that hold `edit` commands run under `--watch`: the counter's label edited while it keeps its count, and the todo row's layout edited while the list, its checked items, the typed draft and the focus are kept.
- **Script targets by kind** (D5). Event scripts can name the n-th node of a kind or of a role, `click checkbox 2` or `click textfield`, besides text targets and coordinates, since checkboxes and empty fields show no text.
- **Live runtime objects in the allocation check** (D6). When `main` returns under `--check-allocations` and no composition is live, groups, slots, state cells and nodes still live are reported as leaks (`allocation check: groups still live at exit: <n>`, ...), with status 102. That extends "no allocation outlives its owner" to the runtime's arenas, which the allocation map cannot see.
- **`prism run --frame-stats`** (D7). Every `runApp` measures its frames and reports on stderr when it returns: frame, event, pass and reload counts; the median, 99th percentile and maximum time of event dispatch, reloads, recomposition, layout, the display list and whole frames; allocations per frame and the live allocations after each frame; and the time to the first frame. In a window it adds rasterization and input-to-present times from `desktop-window`'s `WindowStatistics`. Measuring uses fixed histograms, so it allocates nothing, and costs one branch per stage when off. A counting allocator in the `prism` binary, enabled only by the option, counts the program thread's allocations.
- **"Zero GC pauses", made measurable** (D8, D9). Three properties, each with a target on a release build:
  - no deferred reclamation: the live allocations after every frame of a repeated click or toggle stay equal, and the allocation check reports nothing at exit;
  - no work and no allocation when nothing changes: idle apps run no frame, and a frame whose event invalidates nothing allocates nothing;
  - bounded frames: a counter click in a median under 1 ms headless; toggling an item of the 1,000-item todo list in a recomposition median under 4 ms and a frame 99th percentile under 4 ms headless, and under 16 ms from input to presented pixels in an 800 × 600 window.
- **Benchmarks** (D10). An ignored `crates/prism-cli/tests/app_frame_timing.rs` runs the real apps with `--frame-stats` and generated scripts, reports the numbers and asserts the targets under `PRISM_ASSERT_TIMING=1`, with the time from `prism`'s start to the first frame, cold and warm. `ui-toolkit`'s `ui_run_timing.rs` takes the 1 ms click target.
- **Hot-reload demos** (D11): the two watch sessions above, and the same edits by hand in a window.
- **A manual verification checklist** (D12) for the desktop window, run on a release build and recorded in this change's design with the date, the machine and the frame statistics: opening each app, clicking, typing, resizing, a hot-reload edit while the window keeps its state, and closing. Screenshots of the counter and todo windows are written with `--screenshot` to `docs/images/` and shown in `README.md`.
- **The documentation pass that closes Milestone 2** (D13, D14):
  - `README.md` gains a "What works today" section: the apps, the commands that run them, and the known limits;
  - `ROADMAP.md`: the Phase 4 items and item 33 are ticked, Milestone 2 is marked complete with its date, Phase 5 becomes current, and the follow-ups are listed;
  - `AGENTS.md`'s "Current Milestone & Focus" moves to Milestone 3, and its spec list gains the capabilities Phase 4 added;
  - `docs/architecture.md` §5 loses its last requirement;
  - `docs/decisions-for-review.md` is completed as the user's review document, with a summary at its top that groups the most consequential decisions by revert cost;
  - `openspec/config.yaml`'s apply guidance drops the `examples/demo/ui` exemption and runs the UI examples headless, and the example tests of `fixture_tests.rs` drop theirs.
- **Not in scope.** Each of the following is left to later changes:
  - effects, timers and animation, `derivedStateOf`, lazy lists and scrolling, and a GPU renderer (`compose-architecture` D25);
  - a `prism test` command and a Prism-level UI test API (§5 D21);
  - `--events` driving a window (`desktop-window`);
  - skipping rows that take a borrowed item, which would need stable borrowed parameters (D9);
  - continuous frame statistics while an app runs, such as a live overlay or a periodic report (D7);
  - comparing screenshots in tests (§5 D21);
  - Tier 2 builds of the apps.

## Capabilities

### New Capabilities
- `example-apps`: the reference apps under `examples/`, what each must do (counting, adding, toggling, removing, filtering, keyed rows, the tour), the sessions that verify them under the allocation check, their hot-reload sessions, the rule that every example checks, formats and runs, and their frame and allocation targets.

### Modified Capabilities
- `execution`: live groups, slots, state cells and nodes reported by `--check-allocations`, and the `--frame-stats` option. Both are ADDED requirements beside `compose-runtime`'s "Undisposed Compositions Are Reported", which is not in the main spec yet.
- `ui`: the frame statistics report, and script targets by kind. ADDED, since `ui` is created by `ui-toolkit` and is not in the main specs yet.
- `desktop`: the window's lines in the frame statistics and the window frame budget of the todo app. ADDED, for the same reason, after `desktop-window`.

## Impact

- `examples/`:
  - new `apps/counter/main.prism` and `apps/todo/{model,screen,main,large}.prism`;
  - `demo/ui/ui.prism` rewritten;
  - `sessions/` directories with `.events` scripts, `.stdout.snap` files, a `scripted.edits` script and `.prism.v<k>` replacements.
- `crates/prism-runtime`: `allocation_counter.rs` (`CountingAllocator` and per-thread counts), and the finish sequence of the allocation check extended by the live-object report.
- `crates/prism-compose`: live counts of groups, slots and cells, and their finish report.
- `crates/prism-ui`:
  - `statistics.rs` (histograms, the per-frame record and the report);
  - `app.rs` timing its stages;
  - `script.rs` and `headless.rs` with kind targets;
  - the live node count.
- `crates/prism-desktop`: `WindowStatistics` gains histograms of raster and input-to-present times, read by the report.
- `crates/prism-cli`:
  - `main.rs`: the `#[global_allocator]` and `--frame-stats`;
  - `run.rs`: `RunOptions::frame_stats`;
  - `tests/fixture_tests.rs`: the `example_app_sessions` test, and no more `ui` exemptions;
  - `tests/run_cli_tests.rs`: frame statistics tests;
  - `tests/app_frame_timing.rs`: the ignored benchmark;
  - `tests/ui_run_timing.rs`: its target.
- Documents: `README.md`, `ROADMAP.md`, `AGENTS.md`, `docs/architecture.md`, `docs/decisions-for-review.md`, `docs/images/*.png`, `openspec/config.yaml`.
- No language change, no new error code and no new compiler stage.
- **Prerequisites:** `standard-library`, `compose-runtime`, `compose-transform`, `state-handles`, `ui-toolkit`, `desktop-window` and `compose-hot-reload` are archived first (design, "Baseline"). Groups 1 to 6 need only `ui-toolkit` and `compose-hot-reload`. The window parts (tasks 5.4 and group 8) also need `desktop-window`.
