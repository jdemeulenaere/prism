# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/<scenario-slug>[--<form>]/`, as the header of `fixture_tests.rs` describes. Every fixture of this change is a watch case (`// watch: edits`, with `// run: <status>`), run under `--check-allocations`. Those that show a UI also run headless with an event script (`// events: events`), start with the imports of the toolkit modules they use (`import /prism/compose/ui`, `/prism/compose/layout`, `/prism/compose/material3`; `ui-toolkit` D21), and print dumps. Their edit script holds only a comment unless the scenario names an `after line` step (D12). "The counter screen" means `ui-toolkit`'s `Counter` composable of the scenario "Counter screen", run by `fun main() { runApp(width = 400, height = 300) { Counter() } }`. "The typing app" means its `Form` composable of "Typing into a text field", run the same way. Scenarios whose WHEN names a Rust test, a benchmark or a file saved on disk are tested where their task says.

Prerequisites (design, "Baseline"): `hot-reload`, `standard-library`, `compose-runtime`, `compose-transform`, `state-handles`, `ui-toolkit` and `desktop-window` are archived. Only task 6.3 needs `desktop-window`.

## 1. Baseline reconciliation

- [x] 1.1 Compare the names the design takes from the six packets (design, "Baseline") with the code they left, and fix every task below that names one differently:
  - `prism_runtime::reload` and `AppliedReload`;
  - `contract.rs::{slot_contract, BodyKind}` and `reload.rs::{PreparedReload, ReloadWrites}`;
  - `watch.rs::{WatchSession, SessionEvent, EditScript}` and `run.rs::KeptFiles`;
  - `FileSummary`, `GenericSummary` and their codec;
  - `prism-compose`'s thread state, `Group`, the pass driver, `invalidate_where` and the forced-pass flag;
  - `compose-transform`'s `keys.rs`, `BodyOwner::Restart` and `BodyOwner::RootContent`;
  - `prism-ui`'s `app.rs` loop, `Host`, `HostEvent`, `ScriptCommand`, the script parser and the headless host;
  - `WindowHost::wake`.

  If `hot-reload` is archived, turn the `hot-reload` requirement "Edit Steps Wait for the Reload Point of a Frame Loop" into a MODIFIED block of "Edit Triggers" in the main spec's wording. Check the other ADDED requirements against the archived `compose`, `ui` and `hot-reload` specs for overlaps. Run `openspec validate compose-hot-reload --strict`. Verify by recording the result in the commit message. No code changes in this task.

## 2. Runtime: the applied-reload hook and reload policies (D2, D3, D12)

- [x] 2.1 In `crates/prism-runtime/src/reload.rs`, give `AppliedReload` two fields: `composable_bodies: Vec<u64>` (the keys of the affected composable bodies, D4) and `retired_slots: Vec<usize>` (the old slot addresses of the reload's splits, D8). Add `set_applied_hook(Option<fn(&AppliedReload)>)`. `apply_pending` calls the hook after the slot writes and `instruction_barrier`, and before it prints the message. Verify with unit tests: the hook sees the reload's fields, and runs before the message is written.
- [x] 2.2 Add an atomic `REQUESTED` flag, set by `request` and cleared by `apply_pending` and `withdraw`. `reload_point` returns `None` without taking the lock when it is clear. Make `apply_at_next_call` raise the stack-limit interrupt (`stack::interrupt`, setting `INTERRUPT_PENDING`) when a reload is pending. Make `apply_now` leave the reload pending under `ReloadPolicy::AtReloadPoints`, restoring the limit as today. Verify with unit tests, and with the Rust test of "A reload pending when runApp returns" in `crates/prism-codegen/tests/reload_tests.rs`: request a reload under reload points, switch back, and check that the next call applies it.
- [x] 2.3 Add `ScriptedEdit { target: PathBuf, content: String }`, `set_edit_hook(Option<Arc<dyn Fn(Vec<ScriptedEdit>) + Send + Sync>>)` and `request_scripted_edit(edits)`, which calls the hook on the program thread and does nothing without one, since the script parser rejects `edit` outside `--watch` (D12). Verify with a unit test that the hook receives the edits in order.

## 3. Compiler: body keys and restart contracts (D5, D8)

- [x] 3.1 In `crates/prism-mir/src/summary.rs`, add `FileSummary::composable_bodies` and `GenericSummary::composable_bodies` (`Vec<u64>`), and encode both in the summary codec of the build cache's check entries. In `crates/prism-compose-transform`, record the key of every composable body the transform composes on the body, `Body::body_key` (in the MIR codec): functions by `keys.rs::body_key`, composable closures by their closure key; `FileSummary::of` collects them in body order (design, "Baseline reconciliation"). A generic's bodies go to its `GenericSummary`, the others to the file's. Verify with unit tests: a file of two composables and a content lambda lists three keys, equal to the keys its composed MIR starts its body groups with (`prism dump-mir --composed`), and a codec round trip keeps them.
- [x] 3.2 In `crates/prism-codegen/src/contract.rs`, make `slot_contract` hash a restart body's contract (`BodyKind::Restart`) with the contract of the composable it restarts. Verify with the Rust test of "A changed parameter type splits the restart function": `Label(n: Int)` and `Label(n: Double)` give different restart contracts, and two bodies of `Label(n: Int)` give equal ones.
- [x] 3.3 In `crates/prism-codegen/src/reload.rs`, add `PreparedReload::retired_slots() -> Vec<usize>`, the slot each split symbol was mapped to before the reload, and leave out of `splits` the restart and root-content bodies (their composable's note covers them). Verify with a unit test of a reload that changes a skippable composable's parameter type: one split note, two retired slots.
- [x] 3.4 In `crates/prism-codegen/src/program.rs::Program::start`, install `prism_compose::reload::reload_applied` with `reload::set_applied_hook`, next to the composition report. Verify with the existing reload tests, which must still pass.

## 4. The compose runtime: invalidating reloaded scopes (D3 to D8)

- [x] 4.1 Add `crates/prism-compose/src/reload.rs` with `invalidate_reloaded(bodies: &HashSet<u64>, retired: &HashSet<usize>)`. For each live composition of the thread, it walks the groups in pre-order with an explicit stack holding the nearest enclosing restart scope whose restart function is current, the root counting as one. A group whose key is in `bodies` invalidates that scope, or itself when it is a current restart group. A restart group whose restart function's descriptor names a slot in `retired` is marked `reset_slots`, and its invalidation goes to the scope on the stack. Verify with Rust tests over hand-built compositions: a replaceable body invalidates its restart ancestor, a content lambda's group invalidates its author, a retired scope invalidates its parent, and a group without a restart ancestor invalidates the root.
- [x] 4.2 Add `reload_applied(&AppliedReload)`, the hook of task 3.4. With no active pass it calls `invalidate_reloaded` at once. During a pass it queues the keys in the thread state, and the pass driver (`composition.rs`) runs them after the pass's releases. Verify with a Rust test that applies a reload from inside a body: nothing is invalidated during the pass, and the next pass recomposes the scope.
- [x] 4.3 In `group.rs` and the start of restart groups (`composer.rs`), add the `reset_slots` flag: a restart group started while it is set forgets all its slots onto the pass's release list, keeps its children and clears the flag. Verify with a Rust test with recording drop entries: the slots are released at the end of the pass, and a child group keeps its slots.
- [x] 4.4 Remove `invalidate_where` and the forced-pass flag of `compose-runtime` D15, if they landed, with their tests (D6).
- [x] 4.5 In `crates/prism-compose/tests/runtime_timing.rs`, add the walk for compositions of 1,000, 10,000 and 100,000 groups with one affected body, and assert roughly linear growth under `PRISM_ASSERT_TIMING=1` ("Invalidating after a reload in a large composition"). Record the medians in the design's Costs.

## 5. The watch session (D4, D12)

- [x] 5.1 In `crates/prism-cli/src/watch.rs::WatchSession::reload`, compute the affected bodies after `prepare`. Take the files of the build's file units, add every file that imports one of them directly or not (the graph's `direct_imports`, reversed, in one breadth-first pass), and collect the `composable_bodies` of their kept summaries and of their generics. Add those of the generic of every instance unit the build defines. Pass them, with `PreparedReload::retired_slots`, into the `AppliedReload` the pending reload returns. Verify with a unit test over a three-file graph: an edit of a leaf lists the leaf's and its importers' bodies and not a sibling's.
- [x] 5.2 Install the edit hook of task 2.3 in `run_script` and `run_watching`. It sends `SessionEvent::ScriptedEdit(edits)` and waits on the resume channel as the line hook does. Both loops overlay the edits, call `reload` and resume. Verify with the fixture "Two edits form one reload".
- [x] 5.3 In `crates/prism-cli/src/main.rs` and `run.rs`, `--headless` and `--events` are already accepted with `--watch` (design, "Baseline reconciliation"); `read_event_script` takes whether the run watches. Make the script parser of `prism-ui` (`script.rs`) parse `edit <target> <replacement>`, with the replacement read relative to the script's directory, and `wait reload`. It is given whether the run watches, and reports `the edit command needs --watch` or `the wait command needs --watch` otherwise. Verify with the fixture "Edit without watch" (`// events:` and `// run: 1`, no `// watch:`).
- [x] 5.4 In `crates/prism-cli/tests/fixture_tests.rs`, let a case have `// watch:` together with `// events:` and `// headless:`. The runner spawns `prism run --check-allocations --watch --edits <edits> --headless[=<size>] --events <events> <entry>`. Document it in the header comment. Verify with the fixtures of group 7.

## 6. The frame loop (D2, D9, D10, D12, D13)

- [x] 6.1 In `crates/prism-ui/src/app.rs`, make `prism_ui_run_app` call `reload::defer_to_reload_points(host::wake_for_reload)` before it starts its composition and `reload::apply_at_next_call()` after it drops it, and call `reload::reload_point()` at the top of every loop iteration. Under `--timings` (a host option the CLI sets), print `timing: reload pass <ms> (<n> scopes)` after the passes that follow an applied reload. Verify with the Rust test of "A reload waits for the running handler", where a hand-built handler requests a reload, and with the fixtures of group 7.
- [x] 6.2 In `host.rs`, add `wake_for_reload()`: it notifies a `Condvar` and calls the waker registered with `set_reload_waker(fn())`. In `headless.rs`, run an `edit` group through `reload::request_scripted_edit`, and make `wait reload` wait on the `Condvar` for a request, at most `WAIT_FOR_RELOAD` (10 s, shortened by a test-only setter), raising `event script line <n>: no reload arrived within 10 seconds` past it. Verify with the Rust test of "Waiting for a reload that never comes" in a child process.
- [x] 6.3 In `crates/prism-desktop`, register `WindowHost::wake` with `prism_ui::host::set_reload_waker` when a window host is configured. Verify with a Rust test of `prism-desktop` that a request under reload points makes the host's `next_event` return `HostEvent::Frame`.

## 7. Fixtures

- [x] 7.1 `compose` fixtures, each with its `.prism.v2` replacements:
  - "Editing a composable body recomposes it with its state";
  - "Editing a helper of another file recomposes its callers";
  - "A sibling in an unaffected file skips";
  - "Editing a remember calculation keeps the value";
  - "Changing a remembered type resets that state only";
  - "Adding a composable call";
  - "A composable whose parameters changed restarts with the new code";
  - "A plain composition recomposes reloaded scopes at its next recompose" (no UI, an `after line 2` step).
- [x] 7.2 `ui` fixtures:
  - "A handler whose captures changed";
  - "A handler body edit reaches the next click";
  - "A focused field keeps its focus and text across a reload";
  - "A type error in a composable";
  - "Reloaded code divides by zero" (`// run: 101`);
  - "Two edits form one reload";
  - "Edit without watch" (task 5.3).
- [x] 7.3 `hot-reload` fixture: "A line printed during a pass" (an `after line 1` step under an app).

## 8. Real files and the benchmark (D13)

- [x] 8.1 In `crates/prism-cli/tests/run_cli_tests.rs`, add "Saving a file under a waiting headless app": run the counter app under `prism run --watch --headless --events` with `wait reload` and `dump`, save `main.prism` on disk with the new label, and expect the dump within a 20-second timeout. Add "Timing a UI reload": check the `timing: reload pass` line with the line-pattern assertion that "Timing a reload" uses.
- [x] 8.2 Add `crates/prism-cli/tests/ui_reload_timing.rs`, an `#[ignore]` benchmark (D13): the counter app in a temporary workspace, three clicks, then twenty rounds of a write on disk with a new label constant and `wait reload`, `frame` and `dump`, measured from the write to the dump showing the label. Print the median and 90th percentile, and fail on 50 ms only under `PRISM_ASSERT_TIMING=1`. Run it on a release build and record the numbers in the design's D13.

- [x] 8.3 In `crates/prism-cli/tests/window_input.rs` (opt-in, `PRISM_WINDOW_TESTS=1`), add the scenario `watch-reload`: the counter under `prism run --watch` in a real window; after a click, saving an edit of its label and handler makes the idle window present a frame without input, and the next click runs the new handler on the kept count (D2, D9). Added at apply time; it passes on macOS.

## 9. Documentation and checks

- [x] 9.1 Update `ROADMAP.md`: tick item 33 with what was built and the measured latency, and list the follow-ups (per-body hashes for D4, function values received from unaffected files, resetting edited `remember` calculations, the content closure of a running `runApp`).
- [x] 9.2 In `docs/architecture.md` §5, update the "Implemented today" paragraph, and on archive remove "Hot Reload of Composables Keeps State" and the `edit` command of "Headless Rendering and Scripted Input".
- [x] 9.3 Run every check of `openspec/config.yaml`'s apply guidance:
  - `cargo fmt --all -- --check`;
  - `cargo clippy --workspace --all-targets`, with no new warnings;
  - `cargo test --workspace`;
  - `prism check`, `prism fmt --check` and `prism run --check-allocations` on `examples/`;
  - `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture`, to check that the summaries' growth left every shape linear.
