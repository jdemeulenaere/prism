# Proposal: ui-toolkit

## Why

After `compose-runtime`, `compose-transform` and `state-handles`, composables run, remember, skip and react to state. But nothing they compose can be seen or touched: compositions hold only the bare node tree of `compose-runtime`, which keeps kinds and children and nothing else. Prism has no `Text`, no layout, no drawing and no input. This change is item 30 of "Then: the road to Milestone 2" in `ROADMAP.md`, the UI layer planned in `docs/architecture.md` §5 (`compose-architecture` D1, D17 and D19 to D22). It adds the toolkit that Jetpack Compose programmers expect, with Compose's APIs and its split into modules (user direction 2026-10-19): `/prism/compose/ui`, `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3`, written in Prism over a native node tree in a new `prism-ui` crate, with layout, a display list, input dispatch to handlers owned by nodes, and `runApp`'s frame loop. It also adds a headless mode driven by event scripts, so that every behaviour is pinned by executable fixtures. `desktop-window`, the next item, puts the same display list on screen.

## What Changes

- **A `prism-ui` crate** (D2). It holds the UI node tree, the handler table, layout, the display list, input dispatch, the frame loop and the headless host. It depends on `prism-runtime` and `prism-compose` only, and generated code reaches it only through `extern "C"` entry points (`UI_FUNCTIONS`), checked like the runtime's (`E6005`). Window code stays out of it, so headless programs and its tests build without `winit`.
- **One bundled module per Compose artifact** (D21, user direction), all in the package directory `prism/compose/`, outside the prelude:
  - `/prism/compose/ui` (`compose-ui` with `ui-graphics`, `ui-unit` and `ui-text`): `Modifier` and `then`, `Alignment`, `Color`, `Shape` and `RectangleShape`, `Dp` and `16.dp`, `TextUnit` and `14.sp`, `TextStyle`, `Role` and `runApp`;
  - `/prism/compose/foundation`: `background`, `border`, `clickable`, `toggleable`, `RoundedCornerShape`, `CircleShape`, `BasicText` and `BasicTextField`;
  - `/prism/compose/layout` (`compose-foundation-layout`): `Box`, `Column`, `Row`, `Spacer`, their scopes, `Arrangement`, and `padding`, `width`, `height`, `size`, `defaultMinSize`, `fillMaxWidth`, `fillMaxHeight`, `fillMaxSize`, `weight` and `align`;
  - `/prism/compose/material3`: `Text`, `Button`, `Checkbox`, a single-line `TextField`, `Surface`, `ButtonDefaults`, and a `MaterialTheme` value with Material 3's baseline `colorScheme` and `typography`.
  The prelude keeps only the runtime, which `compose-transform` and `state-handles` now create at `/prism/compose/runtime`. `/prism/composer` stays the runtime's test-facing protocol module.
- **Compose's signatures** (D5, user direction): every composable and modifier has Compose's name, parameter order and defaults (`Column(modifier, verticalArrangement, horizontalAlignment, content)`, `Button(onClick, modifier, enabled, shape, colors, content)`), with the deviations D5 lists, each with its reason (no overloading, no nested types, no composition locals, `&String` text, whole-pixel units).
- **The toolkit composables are Prism code** (D3, D24) over two `package` intrinsic composables of `/prism/compose/ui`, `Node(kind, update, content)` and `LeafNode(kind, update)`, and `package` `external` setters of `prism-ui`. Material components are built from foundation and layout, as in Compose: a `Button` is a clickable `Surface` box holding a `Row`. They are ordinary composables, so they skip, restart and hot-reload like user code.
- **Companion values** (D4), a small language rule: a top-level `val` may share its name with a struct of the same file. The name then means the struct in types and calls, and the value everywhere else. This is what makes Compose's spellings exact: `Modifier.padding(16.dp).fillMaxWidth()` and `modifier: Modifier = Modifier`, `Color.Red` beside `Color(0xFF336699)`, `Arrangement.spacedBy(8.dp)` and `TextStyle.Default`.
- **`Modifier` is Compose's ordered chain** (D6, user direction, replacing `compose-architecture` D18): a `Copy` value holding up to 16 elements in order, so `Modifier.padding(8.dp).background(Color.Red)` differs from `Modifier.background(Color.Red).padding(8.dp)` in layout, drawing and `==`. It allocates nothing and compares element by element, so a `Modifier` parameter stays stable. A seventeenth element is a runtime error.
- **Modifiers are plain data** (D25, user direction): no modifier is composable or remembers anything, unlike the deprecated `Modifier.composed { }`. An element creates or updates state in its node: the pressed target and the focus, and the handlers of `clickable` and `toggleable`, which live in a table of `prism-ui` and are named by a `Copy` `HandlerId` whose equality compares the handlers (D23). So `Modifier.clickable { count.value += 1 }` built on two passes is equal, and a composable receiving it skips.
- **`Dp` and `TextUnit`** (D7, user direction): `16.dp` and `14.sp` through two intrinsic extension properties of `Int`, holding whole logical pixels.
- **Scoped content lambdas** (D22, user direction): `Column`'s content is `compose ColumnScope.() -> Unit`, and `Modifier.weight(1.0)` and `align` resolve only inside a row's, column's or box's content. This needs function types with receivers and context parameters, which Prism lacks; they come from a new language change, `receiver-lambdas`, inserted before this item in `ROADMAP.md`.
- **Text is borrowed** (D8). `Text(text: &String)` and `TextField(value: &String, ...)` accept literals, templates and the borrowed contents of a `State<String>` without `.clone()`, which answers the `state-handles` risk. A node copies a string only when it changed.
- **The node tree** (D10). Nodes live in a per-thread generational arena of `prism-ui` and are addressed by `NodeId`, a `package` one-word `Copy` intrinsic struct. They are created, moved and removed with their node groups through `prism-ui`'s applier. A node owns its strings and the handler entries it claimed, and drops each one when it is replaced or when the node is released.
- **Layout** (D11, D12, implementing `compose-architecture` D19). Layout is a single measure-and-place pass with constraints in integer logical pixels, Compose's model: each node's modifier chain is folded outermost first (padding, sizes, minimum sizes, fills), then its kind's policy measures columns, rows and boxes with arrangements, alignments, weights and per-child `align`, word-wrapped text and text fields. Clean nodes measured with the same constraints reuse their results. Text is measured through a font metrics interface. Headless runs and tests use a fixed-advance metric (half the font size per character, a line height of 5/4 of it), so dumps depend on no font file.
- **A display list** (D13) of clears, filled and rounded rectangles, borders and text runs, each drawing element at the bounds of its place in the chain. It is rebuilt after layout when something visible changed, in reused buffers. Text colours left unspecified take the nearest ancestor's content colour, which `Surface` sets, so a `Button`'s label is white without composition locals.
- **Input** (D14, implementing `compose-architecture` D22). A press hit-tests `clickable` and `toggleable` elements and text fields, topmost first. A release over the same target calls its handler through its function value, as a handler of the composition (`state-handles`). Typed characters and backspace go to the focused field, which calls `onValueChange` with the new text. Disabled targets absorb input. After every event, the frame loop recomposes what the handlers invalidated and lays out again before it dispatches the next event (D14, revising §5's "after the queued events").
- **`runApp`'s frame loop** (D15). `runApp` composes its content once with `prism-ui`'s applier, then loops: wait for an event, dispatch it, recompose, release unclaimed handlers, lay out, build the display list when a frame is due, and sleep while nothing is invalid. It returns when the window closes or the script ends, after dropping its composition. A host interface lets `desktop-window` supply the window. Without one, and without `--headless`, `runApp` is a runtime error. A thread runs one app at a time, and `runApp` in a composable context is `E4030`, as `composition` is.
- **Headless runs and event scripts** (D16, implementing `compose-architecture` D21). `prism run --headless[=<width>x<height>] [--events <script>]` runs `runApp` without a window. The script drives it: `click "<text>" [<n>]`, `click <x> <y>`, `press`, `release`, `type "<text>"`, `key backspace|enter`, `resize <w> <h>`, `frame`, `dump` and `dump draw`. `dump` prints the laid-out node tree, one node per line with its number, bounds, non-default properties and its modifier chain in order. `dump draw` prints the display list. The script is parsed before the program runs. Executable fixtures gain `// events:` and `// headless:` headers.
- **Runtime errors without a location** (D17): `ui misuse: <detail>` and `event script line <n>: <detail>`, status 101.
- **Costs** (D19). A full layout and a display list build are linear in the nodes and elements, and a relayout is linear in the dirty nodes, their ancestors and those ancestors' children. A steady-state frame allocates nothing. Benchmarks over trees of 1,000 to 100,000 nodes and a counting-allocator test pin both. The compiler gains constant work per companion lookup, per `.dp`, per `Node` call and per `runApp` call, pinned by a `compile_timing.rs` shape.
- **Not in scope.** All of the following are left to other changes or follow-ups:
  - the window, rasterization, the Inter font, `--screenshot` and the main-thread split (`desktop-window`);
  - reloads between frames and the script's `edit` command (`compose-hot-reload`);
  - live node counting and `--frame-stats` (`reactive-app-verification`);
  - function types with receivers and context parameters (`receiver-lambdas`);
  - a public `Layout`, `MeasurePolicy` and custom modifier elements, scrolling, `LazyColumn` and `Modifier.clip`;
  - `Image`, `Icon`, `Card`, `Scaffold`, `Switch`, `Slider`, `RadioButton` and `OutlinedTextField`;
  - font weights, `TextStyle`'s other fields, `maxLines`, text alignment and selection, IME, placeholders and keyboard actions, and focus traversal;
  - composition locals, `MaterialTheme { }` and other themes;
  - intrinsic measurements, fractional and computed `Dp`s, and nullable `onCheckedChange`;
  - accessibility, and several apps per thread.

## Capabilities

### New Capabilities
- `ui`: the UI toolkit. It covers the `prism-ui` crate's boundary, the toolkit modules and what each declares, `runApp` and its frame loop, the composables and their look, `Modifier` chains and their order, `Dp` and `TextUnit`, `Color`, shapes, arrangements and alignments, scoped modifiers, constraint layout and text measurement, the node tree and node ownership, the handler table, the display list and content colours, pointer and keyboard input, focus, headless runs, event scripts, dumps, UI runtime errors, and the toolkit's costs.

### Modified Capabilities
- `type-system`: companion values. A struct and a top-level `val` of the same file may share a name ("User-Defined Struct and Nominal Type Validation", whose "Type names are not values" scenario gains the exception).
- `compilation-and-abi`: the ABI exports a struct and its companion value under one name.
- `standard-library`: the four toolkit modules among the bundled modules outside the prelude, and the UI intrinsics in the catalog (`NodeId`, `HandlerId`, `Node`, `LeafNode`, `runApp`, `Int.dp`, `Int.sp`) (ADDED, since the capability is still a proposal of `standard-library`).
- `mir`: composed MIR starts a node group for each `Node` and `LeafNode` call.
- `code-generation`: `prism-ui`'s entry points are checked and linked like the runtime's, one-word intrinsic structs travel as words, and an intrinsic struct may be compared through a runtime entry.
- `execution`: `prism run --headless` and `--events`, with their usage and script errors.

## Impact

- New `crates/prism-ui`, a workspace member:
  - sources: `lib.rs`, `handle.rs`, `thread.rs`, `node.rs`, `tree.rs`, `properties.rs`, `modifier.rs`, `handlers.rs`, `applier.rs`, `text.rs`, `layout.rs`, `draw.rs`, `input.rs`, `app.rs`, `host.rs`, `script.rs`, `headless.rs`, `dump.rs`, `entry.rs`, `functions.rs` and `test_support.rs`;
  - tests: `tests/allocations.rs` and `tests/ui_timing.rs`.
- `crates/prism-runtime`:
  - `PanicKind::UiMisuse` and `PanicKind::EventScriptFailed`;
  - a public `write_output` for dumps;
  - in `function_value.rs`, calls of `() -> Unit`, `(Bool) -> Unit` and `(String) -> Unit` values, and duplication and comparison through the descriptor's entries.
- `crates/prism-compose`: the active pass's composition exposed to `prism-ui`, used to validate setters.
- `crates/prism-typechecker` and `crates/prism-abi`: companion values in `scope.rs`, in name resolution and in import lookup; the catalog entries `NodeId`, `HandlerId` (with its equality entry), `Node`, `LeafNode`, `runApp`, `Int.dp` and `Int.sp`; `E4030` for `runApp`.
- `crates/prism-compose-transform` and `crates/prism-mir`:
  - `ComposerOperation::StartNodeGroup`, and the lowering of `Node` and `LeafNode`;
  - a root content adapter for `runApp`, with its dump, validation and codec;
  - the lowering of `Int.dp` and `Int.sp` as constructions.
- `crates/prism-codegen`:
  - a dependency on `prism-ui`, and `UI_FUNCTIONS` in the entry-point lookup and `Linker::new`;
  - the `runApp` call, intrinsic structs as words in machine signatures, and `HandlerId`'s equality glue calling its entry.
- `crates/prism-library`: `prism/compose/ui.prism`, `foundation.prism`, `layout.prism` and `material3.prism`, and their module entries, not in `PRELUDE`.
- `crates/prism-cli`:
  - `--headless` and `--events` in `main.rs` and `run.rs`, and the host configuration before `main` runs;
  - the fixture runner's `// events:` and `// headless:` headers;
  - fixtures under `tests/fixtures/ui/`, `type-system/`, `compilation-and-abi/`, `standard-library/`, `execution/` and `mir/`;
  - a `compile_timing.rs` shape, and an ignored `ui_run_timing.rs`.
- `examples/demo/ui/ui.prism`: moves to the new API, so it checks.
- Specs:
  - the new `ui` capability and the deltas above;
  - the diagnostics catalog, edited directly: `E4010` allows companion values, and `E4030` names `runApp`.
- `docs/architecture.md` §5: the toolkit requirements move out on archive. The window half of "Rendering Through a Display List" stays for `desktop-window`.
- **Prerequisites:** `state-handles` must be archived first, which brings `compose-transform`, `compose-runtime` and `standard-library` and their baselines (`hot-reload` among them), and `receiver-lambdas` (D22). `escaping-closures`, `collections`, `implicit-it`, `instance-cache` and the trailing-lambda fix (764fc3f) are archived or landed. The Rust core of `prism-ui` (task groups 3 to 9) needs only `compose-runtime` and can start earlier. See `design.md`, "Baseline".
