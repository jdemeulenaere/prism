# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/`, one case per scenario, or one per form of a scenario that has several (`<scenario-slug>--<form>`), as the header of `fixture_tests.rs` describes. Each case runs real `.prism` source:

- headless app cases hold `events.txt` (and `edits` with `.v2` files under `--watch`), as the existing `ui` cases do;
- executable cases hold `stdout.snap` and `stderr.snap` (written with `PRISM_BLESS=1`), and every executable case runs under `--check-allocations`;
- read each snapshot against its scenario's THEN before committing it.

Commit one group at a time, each passing `cargo test --workspace` ("Implement layout-primitive: <group>"). Another agent may be working in the same checkout, so stage explicit paths only.

Implementation note: groups 3 to 7 landed in one commit. Removing the native containers (task 3.3) leaves every app's columns, rows, boxes and spacers without a measure until the library gives them their policies (group 6), so no commit between them could pass `cargo test --workspace`.

## 1. Reconcile and baseline

- [x] 1.1 Check that `vararg`, `core-library-in-prism`, `field-visibility` and `compose-library-in-prism` are archived, and stop if one is not.
  - Re-read the main `ui`, `compose`, `standard-library`, `execution`, `example-apps`, `references` and `extension-functions` specs.
  - Rebase every MODIFIED block on the current main text, keeping every scenario of the main requirement. Turn ADDED blocks whose requirement exists by then into MODIFIED.
  - Check the names the tasks use:
    - `Context::measure`, `measure_content`, `measure_box`, `measure_line`, `relayout_from`, `remeasure_dirty_children` and `Layout::run` (`crates/prism-ui/src/layout.rs`);
    - `NodeKind` and `Node` (`node.rs`), `UiTree::set_on_value_change` and `value_change_pair` (`tree.rs`);
    - `HandlerKind` and `HandlerTable` (`handlers.rs`), `runtime::borrowed` and `runtime::run_handler` (`runtime.rs`);
    - `UI_FUNCTIONS` (`functions.rs`), `TreeBuilder` (`test_support.rs`), `App::reload_point` and `App::settle` (`app.rs`);
    - how `compose-library-in-prism` landed `Node`, `LeafNode`, `NodeId`, the registration externals and `@ExplicitGroupsComposable` in `library/prism/compose/ui.prism`.
  - Get `openspec validate layout-primitive --strict` clean with no INFO lines.
  - Record the reconciliation as a "Baseline update" paragraph in `design.md`.
- [x] 1.2 On an idle machine and a release build, before any code change, record in `design.md` (D18) the medians and 99th percentiles of:
  - `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-ui --test ui_timing -- --ignored --nocapture`;
  - `cargo test --release -p prism-cli --test app_frame_timing -- --ignored --nocapture`;
  - `--test ui_run_timing -- --ignored --nocapture`;
  - the native full relayout of `examples/apps/todo/large.prism`: `cargo run --release -q -p prism-cli -- run --frame-stats --headless --events <script> examples/apps/todo/large.prism` with a first `frame` and fifty rounds of `resize 800 601`, `frame`, `resize 800 600` and `frame` (and the same with `resize 801 600`, the full relayout: D18's correction), recording the `layout` median and 99th percentile and the allocations per frame;
  - the `ui-calls` and `nested-compose-ui` shapes of `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture`.

  This is the baseline task 8.3 compares with. Verify that the numbers are in `design.md`.

## 2. Int and Double conversions (D15)

- [x] 2.1 Declare `public intrinsic fun Int.toDouble(): Double` and `public intrinsic fun Double.toInt(): Int` in `library/prism/core.prism`, with comments.
  - Add their catalog entries in `crates/prism-typechecker/src/intrinsics.rs` beside `toString`, as `BuiltinFunction::IntToDouble` and `BuiltinFunction::DoubleToInt` (`crates/prism-typed-tree/src/tree.rs`, with their names in the typed-tree printer).
  - Add the MIR codec tags, in sequence, never reusing a retired tag.
  - Lower them in `crates/prism-codegen` to `fcvt_from_sint` and `fcvt_to_sint_sat` (NaN gives 0).
- [x] 2.2 Add fixtures for `standard-library` "Conversions need no import" and "Conversions in the ABI of the core module", and for `execution` "Converting numbers". Verify with `cargo test -p prism-typechecker -p prism-codegen -p prism-cli --test fixture_tests --test library_tests`.

## 3. The native measurement protocol (D4, D6, D8, D10, D13, D14)

- [x] 3.1 Nodes and policies.
  - `NodeKind::Layout = 7` (`from_code(7)`, name `layout`, default arrangement and alignment as `Box`'s).
  - `Node` gains:
    - `policy: Option<HandlerId>`, held like `value_change` and listed by `held_handlers`, so release drops it;
    - `placed: bool`, true for new nodes of native kinds;
    - `stamp: u64`, the measurement frame that last measured it.
  - `HandlerKind::Measure`.
  - `UiTree::measure_policy_pair` and `UiTree::set_measure_policy`, mirroring `value_change_pair` and `set_on_value_change`: keep an equal policy, otherwise claim, release the old one and `mark_needs_measure`.
  - `setters::register(HandlerKind::Measure, ..)` and `setters::set_measure_policy`, comparing outside the state's borrow as `set_on_value_change` does.
  - Unit tests in `tree.rs`: an equal policy is kept, a new one replaces the old and marks the node, and release drops it.
- [x] 3.2 Calling a policy.
  - `prism-runtime`'s `FunctionValue` gains a call of closure code with the signature `(result, environment, receiver, list, constraints)`, all addresses.
  - `crates/prism-ui/src/runtime.rs` gains `call_measure_policy`, with `#[repr(C)]` mirrors of `MeasureScope`, `Constraints` (four `i64`), `Measurable` (one word), `Placeable` (`node`, `width`, `height`) and `MeasureResult` (`width`, `height`, `node`).
  - A test in `crates/prism-cli/tests/library_tests.rs` compares each mirror with `prism dump-layout` of the library's struct, so that field order cannot drift.
- [x] 3.3 Make `layout.rs` re-entrant.
  - `Layout::run` no longer holds `&mut UiTree` across a policy call. Each step borrows the thread's UI state for its own reads and writes, as handlers' callers do (D14).
  - Keep `measure`'s cache test, the chain fold, `relayout_from`, `remeasure_dirty_children` and `clear_ancestors` unchanged in behaviour.
  - For the five policy kinds, `measure_content` does the following:
    - pushes a measurement frame (node, fresh stamp, content origin, `layout_called`, `placing`, size);
    - clears `placed` on the children;
    - fills the depth's measurable buffer and builds the list header on its own Rust frame;
    - copies the policy out with `runtime::borrowed` and calls it through `runtime::run_handler` of the app's composition;
    - then checks `layout_called`, clears the last constraints of children not stamped by the frame, pops the frame, and clamps the result's size into the content constraints.
  - Text, text field and window content stay native. The window measures its children under its loose size at `(0, 0)`.
  - Remove `measure_line` and `measure_box`.
- [x] 3.4 Entry points in `crates/prism-ui/src/entry.rs`, each listed in `UI_FUNCTIONS` with its machine signature:
  - `prism_ui_register_measure_policy(policy) -> word` and `prism_ui_set_measure_policy(node, word)`;
  - `prism_ui_measure(node, constraints) -> Placeable` (result address first);
  - `prism_ui_measured(node) -> Placeable`;
  - `prism_ui_begin_placement(node, width, height)` and `prism_ui_end_placement(node)`;
  - `prism_ui_place(parent, child, x, y)`;
  - `prism_ui_arrangement(node) -> Arrangement` and `prism_ui_alignment(node) -> Alignment`;
  - `prism_ui_parent_data(node) -> ParentData`.

  Each checks the innermost frame as D6 and D13 state, and raises the misuse errors of D13 through new constants in `error.rs`. The existing test `every_entry_point_is_listed_once` covers the table.
- [x] 3.5 Verify with `cargo test -p prism-runtime -p prism-ui`.

## 4. Rust policies for the engine's tests (D17)

- [x] 4.1 In `crates/prism-ui/src/test_support.rs`, add Rust policies for columns, rows, boxes and spacers.
  - Each is `extern "C"` closure code with a null environment that calls the Rust API behind the entry points, porting `measure_line` and `measure_box` exactly.
  - `TreeBuilder::add` gives each node of those kinds its policy through `set_measure_policy`.
  - Add a counting policy for the scenario "A text keeping its size calls no policy".
- [x] 4.2 Keep every test in `layout.rs`, `tree.rs`, `draw.rs`, `input.rs`, `dump.rs` and `app.rs` passing unchanged, `incremental_relayouts_match_full_layouts` included. Add:
  - "A text keeping its size calls no policy";
  - child-process tests for "Measuring with no layout running" and "A policy that never calls layout";
  - unit tests for the other misuse details of D13, and a nested-layout test three levels deep whose middle policy clones its measurables.
- [x] 4.3 Extend `crates/prism-ui/tests/allocations.rs` with "Resizing an app allocates nothing", and `crates/prism-ui/tests/ui_timing.rs` to print its full layout against task 1.2's baseline. Verify with `cargo test -p prism-ui`.

## 5. Unplaced nodes and the `layout` kind (D7, D10)

- [x] 5.1 Skip unplaced nodes and their subtrees:
  - in the display list build (`draw.rs`);
  - in hit testing (`input.rs`);
  - in text and kind targets (`script.rs`), where `layout` is also a kind word.

  `dump.rs` prints `unplaced` after `alignment` and before the chain, and the kind name `layout`.
- [x] 5.2 Unit tests for each. Verify with `cargo test -p prism-ui`.

## 6. The library (D2, D3, D5, D9, D16)

- [x] 6.1 In `library/prism/compose/ui.prism`, declare, with comments as the module's other declarations have:
  - `Constraints`, `ConstraintsCompanion`, `Constraints` the value, and its helpers;
  - `Measurable`, `Placeable`, `MeasureScope`, `PlacementScope`, `MeasureResult`, `measure`, `layout`, `place`, `placeRelative` and `Dp.roundToPx()`;
  - `layoutKind`, `ParentData`, `parentData` and `measured`;
  - the package helpers `LayoutNode` and `LeafLayoutNode` (`@ExplicitGroupsComposable`), and `Layout`;
  - the private and package externals of task 3.4.
- [x] 6.2 In `library/prism/compose/layout.prism`, write `Column`, `Row`, `Box` and `Spacer` over `LayoutNode`/`LeafLayoutNode` with capture-free policies.
  - `measureLine` and `measureBox` are extensions of `MeasureScope`, with the arrangement and alignment arithmetic as private functions.
  - Port D9's arithmetic exactly.
- [x] 6.3 Run every `ui`, `compose` and `example-apps` fixture, and `example_app_sessions`: every dump, display list and session must stay byte-identical. Investigate any difference as a port bug, never re-bless it.
- [x] 6.4 Add fixtures for every new `ui` scenario:
  - "A custom layout places its children diagonally" and "A grid of two columns";
  - "Building and using constraints";
  - "Measuring a child twice" and "Invalid constraints";
  - "A flow row wraps its children" and "Placing outside a placement block";
  - "An unplaced text";
  - "A changed policy lays its node out again";
  - "A policy divides by zero";
  - "Calling layout twice";
  - "The counter screen through Prism layouts";
  - "Clicking inside a custom layout by its kind".
- [x] 6.5 Verify with `cargo test -p prism-cli --test fixture_tests --test library_tests --test run_library_tests`.

## 7. Hot reload (D12)

- [x] 7.1 In `App::reload_point`, after a reload is applied, mark every node of the five policy kinds `needs_measure`. The layout that follows records whether any node's size, position or content rectangle changed, and leaves `tree.redraw` unset when none did and the pass drew nothing new.
- [x] 7.2 Add the `--watch` fixture "Editing a function a policy calls" (`app/main.prism`, `lib/gaps.prism`, `lib/gaps.prism.v2`, `edits`, `events.txt`). Re-run the existing reload fixtures, which must keep their output. Add a `prism-ui` test that a reload changing no layout presents no frame. Verify with `cargo test -p prism-ui -p prism-cli --test fixture_tests --test watch_session_tests`.

## 8. Examples, timing and documents (D18, D19)

- [x] 8.1 Add a `FlowRow` over `Layout` to `examples/demo/ui/ui.prism` and compose it in the tour. Re-bless `touring-the-toolkit.stdout.snap` and read the diff: only the new nodes may appear.
  - Add the `run_cli_tests.rs` test "Resizing a thousand items allocates nothing".
  - Run `prism fmt --check examples/`, `prism check` on every file under `examples/`, `prism run --check-allocations examples/run/main.prism`, and each UI entry with `--check-allocations --headless`, as `openspec/config.yaml` lists.
- [x] 8.2 Timing.
  - Add the resize session and its targets (layout p99 under 4 ms, resize frames after the first allocate nothing, and the median printed against task 1.2's native baseline) to `crates/prism-cli/tests/app_frame_timing.rs`, with its header table updated.
  - Add the `layout-policies` shape to `crates/prism-cli/tests/compile_timing.rs` at 256, 1024 and 4096.
- [x] 8.3 On an idle machine and a release build, re-run task 1.2's measurements and `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture`. Record them in `design.md` against the baseline and check D18's budgets. Investigate a miss before going on.
- [x] 8.4 Documents.
  - In `docs/architecture.md`, replace the note that layout is native "for now" with a pointer to the `ui` spec.
  - Tick item 11a of "After Milestone 2: the review round" in `ROADMAP.md`.
  - Mark the layout part of A4 in `docs/milestone-2-review-notes.md` as half done, naming `layout-in-prism` as what remains.
- [x] 8.5 Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace`, reading the `test result:` lines.
