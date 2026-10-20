# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/`, one case per scenario, or one per form of a scenario that has several (`<scenario-slug>--<form>`), as the header of `fixture_tests.rs` describes. Each case runs real `.prism` source:

- headless app cases hold `events.txt` (and `edits` with `.v2` files under `--watch`);
- executable cases hold `stdout.snap` and `stderr.snap` (written with `PRISM_BLESS=1`), and every executable case runs under `--check-allocations`;
- read each snapshot against its scenario's THEN before committing it.

Commit one group at a time, each passing `cargo test --workspace` ("Implement layout-in-prism: <group>"). Another agent may be working in the same checkout, so stage explicit paths only.

## 1. Reconcile and baseline

- [x] 1.1 Check that `layout-primitive` and the packets it depends on are archived, and stop if one is not.
  - Re-read the main `ui` and `example-apps` specs.
  - Rebase the MODIFIED block "Layout and Drawing After a Reload" on the main text as `layout-primitive` left it, keeping every scenario.
  - Check the names the tasks use, as `layout-primitive` landed them:
    - the frame stack, `Context::measure`, the step-free chain fold and `ChainEntry` (`crates/prism-ui/src/layout.rs`, `node.rs`);
    - `ModifierMirror` and `Element` (`modifier.rs`), `HandlerKind` (`handlers.rs`), `prism_ui_run_app` (`entry.rs`), `App::reload_point` (`app.rs`);
    - the Rust policies of `test_support.rs`;
    - `Measurable`, `Placeable`, `MeasureScope`, `PlacementScope` and `MeasureResult` in `library/prism/compose/ui.prism`, and `runApp`.
  - Get `openspec validate layout-in-prism --strict` clean with no INFO lines.
  - Record the reconciliation as a "Baseline update" paragraph in `design.md`.
- [x] 1.2 On an idle machine and a release build, before any code change, record in `design.md` (D9) the medians and 99th percentiles of:
  - `cargo test --release -p prism-cli --test app_frame_timing -- --ignored --nocapture`, which has `layout-primitive`'s resize session;
  - `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-ui --test ui_timing -- --ignored --nocapture`;
  - `--test ui_run_timing -- --ignored --nocapture`.

  Verify that the numbers are in `design.md`.

## 2. Positions in a chain (D5)

- [x] 2.1 Add `package val element: Int` to `Measurable`, `Placeable`, `MeasureScope`, `PlacementScope` and `MeasureResult` in `library/prism/compose/ui.prism`.
  - The two-word measurables and the mirrors in `crates/prism-ui/src/runtime.rs` follow, with the mirror test of `library_tests.rs` updated.
  - Frames become `(node, element)`, and the entry checks of `entry.rs` compare both.
  - A child's measurable is `(child, 0)`. The content's position is the chain's length.
- [x] 2.2 Keep every `prism-ui` test and every `ui` fixture passing unchanged. Verify with `cargo test -p prism-ui -p prism-cli --test fixture_tests --test library_tests`.

## 3. Steps in the native fold (D1, D6, D7, D10, D11)

- [ ] 3.1 `ChainEntry` keeps each element's raw words as `ModifierMirror` read them (`modifier.rs`), beside the decoded `Element`.
- [ ] 3.2 Rewrite `Context::measure`'s fold as D6 describes:
  - walk from element 0, passing non-layout elements through;
  - at a layout element, push the frame `(node, k)` and call the app's element measurer (task 4.2) with the raw element by address, the measurable `(node, k + 1)` and the constraints;
  - measure the content at the end;
  - then compute the offsets from the placements and record the bounds of drawing and input elements in one backward loop.

  A step that did not place its measurable raises `ui misuse: a layout modifier did not place its content` (new constant in `error.rs`). Remove the native `Padding`, `Size`, `DefaultMinSize` and `FillMax` arms.
- [ ] 3.3 In `crates/prism-ui/src/test_support.rs`, add the Rust element measurer porting those arms exactly, and give it to every test app and `TreeBuilder` tree. Keep every test of `layout.rs` passing unchanged, `incremental_relayouts_match_full_layouts` included. Add a unit test for the misuse of task 3.2.
- [ ] 3.4 Extend `crates/prism-ui/tests/allocations.rs` with "Resizing padded rows allocates nothing". Verify with `cargo test -p prism-ui`.

## 4. The library's steps (D2, D4)

- [ ] 4.1 In `library/prism/compose/ui.prism`:
  - add the variant `ModifierElement.Offset(Int, Int)`;
  - write `MeasureScope.measureElement` with the steps of `Padding`, `Size`, `DefaultMinSize`, `FillMax` and `Offset`, porting D2's arithmetic exactly, and a private `Nothing` external for any other element;
  - make `runApp` pass `{ element, measurable, constraints -> measureElement(element, measurable, constraints) }` to `prism_ui_run_app`.
- [ ] 4.2 `prism_ui_run_app` (`entry.rs`, `functions.rs`, `app.rs`) takes the element measurer as a fifth parameter: the app owns it, calls it for built-in steps, and releases it when the app ends.
  - `modifier.rs` decodes `Offset` and dumps it as `offset(<x>,<y>)`.
  - In `library/prism/compose/layout.prism`, add `Modifier.offset(x: Dp = 0.dp, y: Dp = 0.dp)`.
- [ ] 4.3 Run every `ui`, `compose` and `example-apps` fixture and `example_app_sessions`: every dump, display list and session must stay byte-identical. Investigate any difference as a port bug, never re-bless it.
  - Add fixtures for "Layout modifiers inside a custom layout", "An offset background" and "The clickable area follows an offset".
  - Verify with `cargo test -p prism-cli --test fixture_tests --test run_library_tests`.

## 5. `Modifier.layout` (D3, D8)

- [ ] 5.1 Add `HandlerKind::LayoutModifier` and `prism_ui_register_layout_modifier(step) -> word`.
  - `ModifierElement.Layout(HandlerId)` is a handler element: claimed, compared, duplicated and released as `Clickable` is in `tree.rs`'s chain code, and dumped as `layout`.
  - The fold calls its function directly for its step.
  - In `library/prism/compose/ui.prism`, declare `Modifier.layout(measure: attached MeasureScope.(Measurable, Constraints) -> MeasureResult)`.
- [ ] 5.2 In `App::reload_point`, also mark every node whose chain holds a `Layout` element.
- [ ] 5.3 Add fixtures for:
  - "A layout modifier moves its content";
  - "A card with an equal layout modifier is skipped";
  - "A layout modifier that hides its content";
  - "Editing a function a layout modifier calls" (`--watch`, `app/main.prism`, `lib/shifts.prism` and `.v2`).

  Re-run "Editing a function a policy calls". Verify with `cargo test -p prism-ui -p prism-cli --test fixture_tests --test watch_session_tests`.

## 6. Examples, timing and documents (D9, D11)

- [ ] 6.1 Use `offset` and `Modifier.layout` in `examples/demo/ui/ui.prism`. Re-bless `touring-the-toolkit.stdout.snap` and read the diff: only the new elements and the nodes they move may change.
  - Run `prism fmt --check examples/`, `prism check` on every file under `examples/`, `prism run --check-allocations examples/run/main.prism`, and each UI entry with `--check-allocations --headless`, as `openspec/config.yaml` lists.
- [ ] 6.2 Make sure `app_frame_timing.rs`'s header table and targets still hold `layout-primitive`'s resize target.
- [ ] 6.3 On an idle machine and a release build, re-run task 1.2's measurements and `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture`. Record them in `design.md` against the baseline and check D9's budgets. Investigate a miss before going on.
- [ ] 6.4 Documents.
  - Tick items 11b and 11 of "After Milestone 2: the review round" in `ROADMAP.md`.
  - Mark A4 done in `docs/milestone-2-review-notes.md`, naming both layout changes.
  - Replace any remaining mention of native layout modifiers in `docs/architecture.md`.
- [ ] 6.5 Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` and `cargo test --workspace`, reading the `test result:` lines.
