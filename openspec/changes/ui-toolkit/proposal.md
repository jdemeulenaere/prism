# Proposal: ui-toolkit

## Why

After `compose-runtime`, `compose-transform` and `state-handles`, composables run, remember, skip and react to state. But nothing they compose can be seen or touched: compositions hold only the bare node tree of `compose-runtime`, which keeps kinds and children and nothing else. Prism has no `Text`, no layout, no drawing and no input. This change is item 30 of "Then: the road to Milestone 2" in `ROADMAP.md`, the UI layer planned in `docs/architecture.md` §5 (`compose-architecture` D1 and D17 to D22). It adds the toolkit that Jetpack Compose programmers expect, written in Prism in `/prism/ui` over a native node tree in a new `prism-ui` crate: layout, a display list, input dispatch to handlers owned by nodes, and `runApp`'s frame loop. It also adds a headless mode driven by event scripts, so that every behaviour is pinned by executable fixtures. `desktop-window`, the next item, puts the same display list on screen.

## What Changes

- **A `prism-ui` crate** (D2). It holds the UI node tree, layout, the display list, input dispatch, the frame loop and the headless host. It depends on `prism-runtime` and `prism-compose` only, and generated code reaches it only through `extern "C"` entry points (`UI_FUNCTIONS`), checked like the runtime's (`E6005`). Window code stays out of it, so headless programs and its tests build without `winit`.
- **`/prism/ui`, a bundled module written in Prism** (D3, D5). It is outside the prelude, so programs `import /prism/ui`. It holds `Column`, `Row`, `Box`, `Text`, `Button`, `Spacer`, `Checkbox` and a single-line `TextField`, with Jetpack Compose's names, parameter order and defaults (`Column(modifier, verticalArrangement, horizontalAlignment, content)`), and `runApp(title, width, height, content)`. Each composable is a few lines of Prism over two private intrinsic composables, `Node(kind, update, content)` and `LeafNode(kind, update)`, and private `external` property setters of `prism-ui`. They are ordinary composables, so they skip, restart and hot-reload like user code.
- **Companion values** (D4), a small language rule: a top-level `val` may share its name with a struct of the same file. The name then means the struct in types and calls, and the value everywhere else. This is what makes Compose's spellings exact: `Modifier.padding(16).fillMaxWidth()` and `modifier: Modifier = Modifier`, `Color.Red` beside `Color(0xFF336699)`, and `Arrangement.spacedBy(8)`.
- **`Modifier`, a `Copy` struct** (D6, implementing `compose-architecture` D18). It is built by chained extension calls: `padding` (all sides, horizontal and vertical, or per side; repeated paddings add up), `width`, `height`, `size`, `fillMaxWidth`, `fillMaxHeight`, `fillMaxSize`, `weight`, `background` and `border`, with shapes `RectangleShape`, `CircleShape` and `RoundedCornerShape(r)`. Its properties do not depend on call order, so it allocates nothing, compares by `==`, and never makes a composable unstable. **`Color`** is a `Copy` ARGB struct with Compose's named colours and `Color.Unspecified`. **`Arrangement`** and **`Alignment`** have Compose's values. Sizes are `Int` logical pixels (D7).
- **Text is borrowed** (D8). `Text(text: &String)` and `TextField(value: &String, ...)` accept literals, templates and the borrowed contents of a `State<String>` without `.clone()`, which answers the `state-handles` risk. A node copies a string only when it changed.
- **The node tree** (D10). Nodes live in a per-thread generational arena of `prism-ui` and are addressed by `NodeId`, a private one-word `Copy` intrinsic struct. They are created, moved and removed with their node groups through `prism-ui`'s applier. A node owns its strings and handler closures and drops each one when it is replaced or when the node is released.
- **Layout** (D11, D12, implementing `compose-architecture` D19). Layout is a single measure-and-place pass with constraints in integer logical pixels. It is Compose's model: columns, rows and boxes with arrangements, alignments and weights, padding, fixed and filling sizes, and word-wrapped text. Clean nodes measured with the same constraints reuse their results. Text is measured through a font metrics interface. Headless runs and tests use a fixed-advance metric (half the font size per character, a line height of 5/4 of it), so dumps depend on no font file.
- **A display list** (D13) of clears, filled and rounded rectangles, borders and text runs. It is rebuilt after layout when something visible changed, in reused buffers. Text colours left unspecified take the nearest ancestor's content colour, so a `Button`'s label is white without composition locals.
- **Input** (D14, implementing `compose-architecture` D22). A press hit-tests the laid-out tree topmost first. A release over the same node calls its handler through its function value, as a handler of the composition (`state-handles`): `Button` clicks, `Checkbox` toggles and `TextField` focus. Typed characters and backspace go to the focused field, which calls `onValueChange` with the new text. Disabled controls absorb input. After every event, the frame loop recomposes what the handlers invalidated and lays out again before it dispatches the next event (D14, revising §5's "after the queued events").
- **`runApp`'s frame loop** (D15). `runApp` composes its content once with `prism-ui`'s applier, then loops: wait for an event, dispatch it, recompose, lay out, build the display list when a frame is due, and sleep while nothing is invalid. It returns when the window closes or the script ends, after dropping its composition. A host interface lets `desktop-window` supply the window. Without one, and without `--headless`, `runApp` is a runtime error. A thread runs one app at a time, and `runApp` in a composable context is `E4030`, as `composition` is.
- **Headless runs and event scripts** (D16, implementing `compose-architecture` D21). `prism run --headless[=<width>x<height>] [--events <script>]` runs `runApp` without a window. The script drives it: `click "<text>" [<n>]`, `click <x> <y>`, `press`, `release`, `type "<text>"`, `key backspace|enter`, `resize <w> <h>`, `frame`, `dump` and `dump draw`. `dump` prints the laid-out node tree, one node per line with its number, bounds and non-default properties. `dump draw` prints the display list. The script is parsed before the program runs. Executable fixtures gain `// events:` and `// headless:` headers.
- **Runtime errors without a location** (D17): `ui misuse: <detail>` and `event script line <n>: <detail>`, status 101.
- **Costs** (D19). A full layout and a display list build are linear in the nodes, and a relayout is linear in the dirty nodes, their ancestors and those ancestors' children. A steady-state frame allocates nothing. Benchmarks over trees of 1,000 to 100,000 nodes and a counting-allocator test pin both. The compiler gains constant work per companion lookup, per `Node` call and per `runApp` call, pinned by a `compile_timing.rs` shape.
- **Not in scope.** All of the following are left to other changes or follow-ups:
  - the window, rasterization, the Inter font, `--screenshot` and the main-thread split (`desktop-window`);
  - reloads between frames and the script's `edit` command (`compose-hot-reload`);
  - live node counting and `--frame-stats` (`reactive-app-verification`);
  - `Modifier.clickable` (a handler cannot live in a `Copy` modifier), ordered modifier chains, `fillMaxWidth(fraction)`, scrolling and lazy lists;
  - `Image`, `Icon`, `Surface`, `Card`, `Scaffold`, `Switch`, `Slider` and `RadioButton`;
  - font weights, `TextStyle`, `maxLines`, text alignment and selection, IME, placeholders and keyboard actions, and focus traversal;
  - composition locals, theming and `ButtonColors`;
  - row and column scopes, intrinsic measurements, `Dp` and `sp` units, and nullable `onCheckedChange`;
  - accessibility, and several apps per thread.

## Capabilities

### New Capabilities
- `ui`: the UI toolkit. It covers the `prism-ui` crate's boundary, `runApp` and its frame loop, the composables of `/prism/ui` and their look, `Modifier`, `Color`, shapes, arrangements and alignments, constraint layout and text measurement, the node tree and node ownership, the display list and content colours, pointer and keyboard input, focus, headless runs, event scripts, dumps, UI runtime errors, and the toolkit's costs.

### Modified Capabilities
- `type-system`: companion values. A struct and a top-level `val` of the same file may share a name ("User-Defined Struct and Nominal Type Validation", whose "Type names are not values" scenario gains the exception).
- `compilation-and-abi`: the ABI exports a struct and its companion value under one name.
- `standard-library`: `/prism/ui` among the bundled modules outside the prelude, the UI intrinsics in the catalog, and default values on intrinsic parameters (ADDED, since the capability is still a proposal of `standard-library`).
- `mir`: composed MIR starts a node group for each `Node` and `LeafNode` call.
- `code-generation`: `prism-ui`'s entry points are checked and linked like the runtime's, and one-word intrinsic structs travel as words.
- `execution`: `prism run --headless` and `--events`, with their usage and script errors.

## Impact

- New `crates/prism-ui`, a workspace member:
  - sources: `lib.rs`, `handle.rs`, `thread.rs`, `node.rs`, `tree.rs`, `properties.rs`, `applier.rs`, `text.rs`, `layout.rs`, `draw.rs`, `input.rs`, `app.rs`, `host.rs`, `script.rs`, `headless.rs`, `dump.rs`, `entry.rs`, `functions.rs` and `test_support.rs`;
  - tests: `tests/allocations.rs` and `tests/ui_timing.rs`.
- `crates/prism-runtime`:
  - `PanicKind::UiMisuse` and `PanicKind::EventScriptFailed`;
  - a public `write_output` for dumps;
  - in `function_value.rs`, calls of `() -> Unit`, `(Bool) -> Unit` and `(String) -> Unit` values.
- `crates/prism-compose`: the active pass's composition exposed to `prism-ui`, used to validate setters.
- `crates/prism-typechecker` and `crates/prism-abi`: companion values in `scope.rs`, in name resolution and in import lookup; the catalog entries `NodeId`, `Node`, `LeafNode` and `runApp`; intrinsic parameter defaults; `E4030` for `runApp`.
- `crates/prism-compose-transform` and `crates/prism-mir`:
  - `ComposerOperation::StartNodeGroup`, and the lowering of `Node` and `LeafNode`;
  - a root content adapter for `runApp`, with its dump, validation and codec.
- `crates/prism-codegen`:
  - a dependency on `prism-ui`, and `UI_FUNCTIONS` in `entry_point` and `Linker::new`;
  - the `runApp` call, and intrinsic structs as words in machine signatures.
- `crates/prism-library`: `prism/ui.prism` and its module entry, not in `PRELUDE`.
- `crates/prism-cli`:
  - `--headless` and `--events` in `main.rs` and `run.rs`, and the host configuration before `main` runs;
  - the fixture runner's `// events:` and `// headless:` headers;
  - fixtures under `tests/fixtures/ui/`, `type-system/`, `compilation-and-abi/`, `execution/` and `mir/`;
  - a `compile_timing.rs` shape, and an ignored `ui_run_timing.rs`.
- `examples/demo/ui/ui.prism`: moves to the new API, so it checks.
- Specs:
  - the new `ui` capability and the deltas above;
  - the diagnostics catalog, edited directly: `E4010` allows companion values, and `E4030` names `runApp`.
- `docs/architecture.md` §5: the toolkit requirements move out on archive. The window half of "Rendering Through a Display List" stays for `desktop-window`.
- **Prerequisites:** `state-handles` must be archived first, which brings `compose-transform`, `compose-runtime` and `standard-library` and their baselines (`hot-reload` among them). `escaping-closures`, `collections`, `implicit-it`, `instance-cache` and the trailing-lambda fix (764fc3f) are archived or landed. The Rust core of `prism-ui` (task groups 3 to 8) needs only `compose-runtime` and can start earlier. See `design.md`, "Baseline".
