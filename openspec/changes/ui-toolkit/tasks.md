# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/<scenario-slug>[--<form>]/`, as the header of `fixture_tests.rs` describes. Every `ui` fixture is executable (`// run: <status>`, run under `--check-allocations`), starts with `import /prism/compose/ui`, `import /prism/compose/foundation`, `import /prism/compose/layout` and `import /prism/compose/material3` unless its scenario names other imports, and most run headless with an event script (`// events: events.txt`, task 9.3) and print dumps. Scenarios whose WHEN names a Rust test or a benchmark are tested where their task says. "The counter screen" in a task means the `Counter` composable of the scenario "Counter screen", run by `fun main() { runApp(width = 400, height = 300) { Counter() } }`, and "the typing app" means the `Form` composable of "Typing into a text field", run the same way.

Prerequisites (design, "Baseline"): `state-handles` is archived, which brings `compose-transform`, `compose-runtime` and `standard-library`, and `receiver-lambdas` is archived (D22). Groups 3 to 9 need only `compose-runtime`'s crate, and their tests build function values by hand, so they can start earlier.

## 1. Baseline reconciliation

- [ ] 1.1 Compare the names the design takes from `compose-runtime`, `compose-transform`, `state-handles`, `standard-library` and `receiver-lambdas` (design, "Baseline") with the code those changes left, and fix every task below that names one differently:
  - `Applier` and `Composition::start`;
  - `Composition::run_handler`, and the active pass's composition;
  - `prism_compose_start_node_group`;
  - `FunctionValue`, and the environment equality and duplicate entries of descriptors;
  - the entry-point lookup;
  - the intrinsic catalogs of the type checker and the transform;
  - the root content adapter of `composition`;
  - how intrinsic structs are passed in machine signatures;
  - the runtime module at `prism/compose/runtime.prism` (package `prism.compose`), and the library's support for modules in subdirectories (`MODULES`, the embedded file list of `library_tests.rs`, sibling imports between `prism/compose/` files);
  - the syntax of receiver function types and context parameters, and the error for a missing context, which D22, the scope declarations of task 11.3 and the scenario "A scoped modifier outside its scope" use;
  - whether a struct may have no fields (the scopes of D22), and whether a public struct's fields may name a `package` type (D6 assumes they may not, so `ModifierElement` and `HandlerId` are public).

  Compare the MODIFIED block of `type-system` with the main spec, and the `standard-library` delta with that capability as archived, and reconcile any drift. Verify by recording the result in the commit message. No code changes in this task.

## 2. Companion values (D4)

- [ ] 2.1 In `crates/prism-typechecker/src/scope.rs`, make `define` accept a top-level value symbol when the only other binding of its name at that level is a struct declared in the same file, and the reverse. Keep `E4010` for every other pair. Add `lookup_value(name)`, which skips a struct symbol when a value symbol of the same name is visible at the same level, and `lookup_constructor(name)`, which finds the struct. Make the import lookup (`lookup_import`) keep both kinds of an imported name. Verify with unit tests of `define` and both lookups.
- [ ] 2.2 In `crates/prism-typechecker/src/checker.rs`:
  - resolve a name in a value position (an identifier, a receiver, the base of a field access) through `lookup_value`;
  - resolve the callee of a call through `lookup_constructor` when it names a struct;
  - leave type positions on `lookup_type`.

  Report `E4010` for a `val` named like an imported struct, an enum, a function or an extension. Verify with the `type-system` fixtures for every scenario of "A Struct May Have a Companion Value", "Only a Same-File Struct and Value Share a Name" and "Importing a Struct With Its Companion Value", and both forms of the modified "Type names are not values".
- [ ] 2.3 In `crates/prism-abi/src/signature.rs`, check that `FileAbi::extract` exports both symbols and that the hash covers both, and fix the importer's symbol table if it is keyed by name alone. Verify with the `compilation-and-abi` fixtures for "A struct and its companion value are exported" and "Adding a companion value changes the ABI hash".
- [ ] 2.4 Edit the `E4010` row of `openspec/specs/diagnostics/spec.md` to say that a struct and a top-level `val` of the same file may share a name. Verify with the catalog check of `fixture_tests.rs`.

## 3. Runtime support (D16, D17, D23)

- [ ] 3.1 In `crates/prism-runtime/src/panic.rs`, add `PanicKind::UiMisuse` (`ui misuse: <detail>`) and `PanicKind::EventScriptFailed` (`event script line <detail>`), with the next free codes after `state-handles`' kinds, and extend `ALL`. Verify with the `panic_message` unit tests.
- [ ] 3.2 In `output.rs`, add `pub fn write_output(text: &str)` over `append`. Verify with a unit test that the text joins the buffer in order with `prism_rt_print_string`.
- [ ] 3.3 In `function_value.rs`, add `unsafe fn call_unit(&self)`, `call_with_bool(&self, value: bool)` and `call_with_string(&self, value: PrismString)`, with the closure code signatures of `closures` (environment first; the `String` by address, owned by the callee), and `is_keepable`, `duplicate` and `equals` over the descriptor's environment entries (`compose-transform` D9). Verify with unit tests over hand-built descriptors, including a `String` the callee frees under the allocation map, and equal and unequal environments.
- [ ] 3.4 In `crates/prism-compose`, expose `active_composition() -> Option<Handle>` (the composition of the thread's active pass) for `prism-ui`. Verify with a unit test inside and outside a pass.

## 4. The `prism-ui` crate: handles, nodes, tree, handlers, applier (D2, D3, D10, D23)

- [x] 4.1 Create `crates/prism-ui` (`Cargo.toml` depending on `prism-runtime` and `prism-compose` only) and add it to the workspace members. Write `src/lib.rs` with a crate comment summarizing D1 to D3, D23 and D25, citing the first-class compose and hot reload invariant and the zero-cost memory safety invariant by name. Add a test that reads `Cargo.toml` and finds no other dependency. Verify with the scenario "Headless programs link no window code".
- [x] 4.2 Add `src/handle.rs` (generational `NodeId` and `HandlerId` words, retirement at the maximum generation) and `src/thread.rs` (the per-thread arena, handler table and running app, behind `with_state`, with D2's rule that no reference is held while program code runs). Verify with unit tests: a released handle is stale, a reused index has a new generation, and two threads have independent states.
- [x] 4.3 Add `src/node.rs` (`NodeKind` from the kind codes, the node record of D10 with its decoded chain, claimed handler entries and dirty flags) and `src/properties.rs` (arrangement, alignment and text style records decoded from the setter words, with negative sizes laid out as 0). Verify with unit tests of the decoding and of an unknown kind.
- [x] 4.4 Add `src/modifier.rs`: `#[repr(C)]` mirrors of `Modifier`, `ModifierElements` and `ModifierElement`, a decoder into the node's chain (`Element` values with whole-pixel payloads), and the element comparison that tells layout, drawing and handler changes apart. Verify with unit tests over hand-built chains; the layout check against the Prism declarations is task 11.1.
- [x] 4.5 Add `src/handlers.rs`, the handler table of D23: register (pending), claim by a node (taking a pending entry, keeping one already held, duplicating a keepable one held by another node, the misuse for a shared one that is not keepable, and `stale modifier handler`), release by a node, release of unclaimed entries at the end of a pass (and of those registered between passes at the end of the next), `equal`, and release of everything at the end of the app. Verify with unit tests for every rule, using recording drop entries from `test_support.rs`.
- [x] 4.6 Add `src/tree.rs`:
  - `UiTree`, with a window root, linked children, creation numbers from 1, and `place_after(parent, previous, node)` that does nothing when already in place;
  - `remove`, which marks the parent dirty;
  - `release`, which takes the values out, gives up the node's handler entries and drops them after letting go of the state;
  - `mark_needs_measure`, which propagates `child_needs_measure` upwards and stops at a marked ancestor.

  Verify with unit tests: insert, move, unchanged placement, removal and release order (children first), using recording drop entries.
- [x] 4.7 Add `src/applier.rs` with `UiApplier: prism_compose::Applier` over the thread's `UiTree`, and its `dump` delegating to the tree dump (task 6.3). Verify with a Rust test driving a `Composition::start(UiApplier, ...)` built with `prism-compose`'s `test_support` content functions: node groups create, move and remove UI nodes.
- [x] 4.8 Add `src/test_support.rs`: handler function values from Rust closures (boxed environments, trampolines, drop entries, keepable or not), recording drop entries, and a builder for trees with modifier chains without a composition, for layout tests and benchmarks. Verify by using it in the tests of groups 4 to 8.

## 5. Text measurement and layout (D11, D12)

- [ ] 5.1 Add `src/text.rs`: `TextMeasurer`, `FixedAdvanceMetrics` (advance `font_size / 2`, line height `font_size * 5 / 4`, 16 for an unspecified size), and the line breaker of D12 (newlines, greedy breaks at spaces, overlong words kept whole) returning line ranges and widths into a reused buffer. Verify with unit tests: wrapping at an exact width, a trailing space, an overlong word, an empty text with one line, and newlines.
- [ ] 5.2 Add `src/layout.rs`:
  - `Constraints` and `measure(node, constraints)`, folding the chain as D11's first table says (padding, sizes, minimum sizes, fills), recording the bounds at each drawing and input element, then the kind's policy with its result clamped into what remains;
  - the policies of every kind, the arrangements and the alignments;
  - parent data: weights with `fill`, the remainder going to the last weighted child, and per-child `align`, the first in a chain counting;
  - placement relative to the parent, and the cache, reusing the last size of a clean node measured with its last constraints.

  Verify with Rust unit tests through the tree builder, one per element and per policy, for the numbers of the `ui` scenarios "Column stacks children", "Space between two texts", "Centred content in a box", "A weighted text fills a row", "Aligning one child", "Padding around a column", "A size larger than the window is clamped", "Padding before or after a size", "Minimum and fractional sizes", "Chained paddings add up" and "A button's size and label position".
- [ ] 5.3 Add the Rust test of "One changed text is measured alone", with a measurer that counts the characters it measures.

## 6. Display list and dumps (D9, D13, D16)

- [ ] 6.1 Add `src/draw.rs` with `DrawCommand` and `DisplayList`, reused between builds. Implement `build_display_list(tree)` in one walk, parents first:
  - the clear;
  - each node's drawing elements in chain order at their recorded bounds, the text field frame by the node's focus, and corner radii resolved from shapes against those bounds;
  - each kind's own commands (text lines, a text field's value and caret);
  - content colours resolved through the walk.

  Verify with unit tests for each element, element order, a focused text field's frame and caret, and an unspecified text colour inside and outside a node with a content colour.
- [ ] 6.2 In `src/dump.rs`, add `dump_display_list` in the format of "Dumping the Display List". Verify with unit tests over built display lists, including rounded corners.
- [ ] 6.3 In `src/dump.rs`, add `dump_tree` in the format of "Dumping the Laid-Out Tree" and design D16: absolute bounds, properties in their fixed order, the chain's elements in order with their arguments (`padding(<all>)` or four values, `size(<w>x<h>)`, `defaultMinSize` with `-`, fractions, shapes, roles, `disabled`), colours as `#AARRGGBB`, `arrangement spacedBy(8)` and Compose's names for the others, quoted text with `\"`, `\\` and `\n` escapes. Verify both dumps with unit tests over built trees.

## 7. Input dispatch (D14)

- [ ] 7.1 Add `src/input.rs`:
  - `hit_test(tree, x, y)`, in reverse drawing order, for `clickable` and `toggleable` elements at their recorded bounds (later elements of a chain first) and for text fields;
  - a `Dispatcher` holding the pressed target (node and element) and the focused node;
  - `press`, `release`, `character` and `key`, with the rules of D14.

  Handlers are called through `Composition::run_handler(app composition, ...)` and `FunctionValue::call_unit`, `call_with_bool` or `call_with_string`, after copying the function value out of its handler entry. Verify with Rust unit tests for the topmost hit, an element's bounds inside padding, release elsewhere, disabled targets, a toggle calling with the opposite value, focus set and cleared, backspace on an empty value, and enter.

## 8. The app, the frame loop and hosts (D15, D16)

- [ ] 8.1 Add `src/host.rs` with `Host`, `HostEvent`, `Key`, `DumpKind`, and `configure(HostConfiguration)`, a process-wide setting read by the program thread. The configurations are `None`, `Headless { size, commands }` and a window factory for `desktop-window`. Verify with a unit test that a configuration set on one thread is read on another.
- [ ] 8.2 Add `src/script.rs`:
  - `parse_script(text) -> Result<Vec<ScriptLine>, ScriptError>`, with the grammar of D16 (strings with escapes, optional occurrences, `#` comments, blank lines);
  - `ScriptError { line, message }`, with messages such as `unknown command 'clik'`, `expected a string or two numbers after 'click'` and `unterminated string`.

  Verify with unit tests for every command and every error.
- [ ] 8.3 Add `src/headless.rs`, a `Host` that yields the script's lines in order and resolves text targets against the tree at the time of the command: the n-th text or text field value in tree order, at its centre, rounded down. It raises `EventScriptFailed` for a missing target and for `type` or `key` without focus, and prints dumps through `prism_runtime::write_output`. Verify with Rust tests over a built tree: an occurrence index, rounding of the centre, and both errors in a child process.
- [ ] 8.4 Add `src/app.rs`:
  - `App` (the composition handle, tree, handler table, dispatcher, measurer, display list and its stale flag);
  - `run_app(title, width, height, content)`, with the loop of D15: passes while invalid, each followed by the release of unclaimed handler entries, layout while dirty, otherwise the next event; one pass and layout after each dispatched event; frames, dumps, resizes, close;
  - disposal of the composition before the handler table and the tree are dropped;
  - the misuse errors for a running app and for no host.

  Verify with Rust tests through `test_support.rs`: an idle loop runs nothing (counters of passes, layouts and builds), an event is followed by exactly one pass, unclaimed handlers are released after their pass, and disposal releases every node and handler.
- [ ] 8.5 Add `tests/allocations.rs`, with the scenario "Allocation-counting test", and `tests/ui_timing.rs`, `#[ignore]`d, with the measurements of design D19 and the assertions of "Layout timing benchmark" under `PRISM_ASSERT_TIMING=1`. Run `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-ui --test ui_timing -- --ignored --nocapture` and record the medians in design.md, D19.

## 9. Entry points, backend and CLI (D2, D15, D16, D17, D23)

- [ ] 9.1 Add `src/entry.rs` with every `#[no_mangle] pub extern "C" fn prism_ui_*`:
  - the setters `set_text(node, *const PrismString)`, `set_text_style`, `set_modifier(node, *const Modifier)`, `set_arrangement`, `set_alignment`, `set_content_color`, `set_enabled` and `set_on_value_change(node, handler)`;
  - `register_click`, `register_toggle` and `register_text`, each taking a `*mut FunctionValue` by move and returning a `HandlerId` word, and `handlers_equal(a, b) -> u8`;
  - `modifier_overflow()`, which never returns;
  - `run_app(title, width, height, content: *mut FunctionValue)`.

  Each setter validates the node and that the active pass belongs to the running app (D10, D17), compares, claims handler entries, and marks the node dirty. Add `src/functions.rs` with `UI_FUNCTIONS`, and the source-reading test of `COMPOSE_FUNCTIONS`. Verify with the Rust child-process test of "A stale node handle", and tests for `unknown node kind`, `node property set outside composition` and registering with no app.
- [ ] 9.2 In `crates/prism-codegen`, depend on `prism-ui`, search `UI_FUNCTIONS` in the entry-point lookup, and register it in `link.rs::Linker::new`. Make one-word intrinsic structs travel as `i64` words in `representation.rs`, if task 1.1 found that they do not already, and make the equality glue of an intrinsic struct with a catalog equality entry call it. Verify with `lowering_tests.rs` tests over injected library modules for "A UI entry point declared with the wrong signature", "An external taking a node handle" and "Comparing handler ids", and a link test that every UI entry point resolves.
- [ ] 9.3 In `crates/prism-cli/src/main.rs::run_program`, accept `--headless`, `--headless=<w>x<h>` and `--events <path>` before the file, with `--events` without `--headless` a usage error, and update the usage and help texts. In `run.rs`:
  - add `headless: Option<HeadlessOptions>` to `RunOptions`;
  - read and parse the script before compiling, reporting `Error: <path>:<line>: <message>` with status 1;
  - call `prism_ui::configure` before `Program::run`.

  In `tests/fixture_tests.rs`, read `// events: <file>` and `// headless: <w>x<h>`, adding the options to the spawned `prism run`, and document both in the header comment. Verify with the `execution` fixtures for every scenario of "Headless Runs" and "Event Scripts Are Parsed Before the Program Runs".

## 10. The compiler side of the toolkit (D3, D5, D7, D15, D23)

- [ ] 10.1 In `crates/prism-typechecker/src/intrinsics.rs`, add the catalog entries of "The UI Intrinsics":
  - `prism/compose/ui::NodeId` and `prism/compose/ui::HandlerId`, intrinsic structs of one word, `Copy`, the second with the equality entry `prism_ui_handlers_equal`;
  - `Node` and `LeafNode`, signature-typed intrinsic composables;
  - `runApp`, signature-typed;
  - `Int.dp` and `Int.sp`, intrinsic properties.

  Each function gets a `BuiltinFunction` (`UiNode`, `UiLeafNode`, `RunApp`, `DpOfInt`, `TextUnitOfInt`). Extend `E4030` to `runApp`, and edit the `E4030` row of the diagnostics catalog to name it. Verify with the `standard-library` fixtures "A UI intrinsic in another library module" (a `// library:` case) and "Unit properties are members of Int", and the `ui` fixture "runApp inside a composable" (`// expect: E4030`).
- [ ] 10.2 In `crates/prism-mir`, add `ComposerOperation::StartNodeGroup { key }`, which takes a kind operand and returns the node word, to the composed MIR, its dump (`composer start node group`), validation, instantiation and the `instance-cache` codec, and lower `DpOfInt` and `TextUnitOfInt` reads as aggregate constructions. In `crates/prism-compose-transform/src/intrinsics.rs`, lower `UiNode` and `UiLeafNode` as D3 says, with the key `H(parent, "node", ordinal)` from `keys.rs`. Extend the root content adapter of `composition` to `RunApp`'s content (`BodyOwner::RootContent`). Verify with the `mir` fixtures "A toolkit composable starts a node group" (a `// library:` case with `composed-mir.snap`), "The adapter of an app" and "A unit in MIR".
- [ ] 10.3 In `crates/prism-codegen`, lower `StartNodeGroup` as a call of `prism_compose_start_node_group`, and `RunApp` as a call of `prism_ui_run_app` with the adapter moved by address. Verify with the fixtures of task 12.1.

## 11. The toolkit modules (D3 to D9, D21 to D25)

Each module is a file of `crates/prism-library/prism/compose/` with `package prism.compose`, a comment per declaration, sibling imports of the modules it uses, an entry in `MODULES` and none in `PRELUDE`. Verify each with `library_tests.rs` (check, format, compile and externals) and `cargo run -q -p prism-cli -- fmt --check crates/prism-library/prism`.

- [ ] 11.1 Add `ui.prism`:
  - `Color` with its companion `Colors`, `Shape` and `RectangleShape`, `Alignment`, `Role`;
  - `Dp` and `TextUnit` with their companions and the intrinsic properties `dp` and `sp`, and `TextStyle` with `TextStyle.Default`;
  - `HandlerId`, `ModifierElement`, `ModifierElements`, `Modifier`, its companion, `then`, and the `package` helpers `append` and `textFieldFrame`;
  - with `package` visibility: `NodeId`, `Node`, `LeafNode`, the kinds, the setter, registration and overflow externals, and `applyModifier`;
  - `runApp`.

  Add a Rust test that compares the `#[repr(C)]` mirrors of task 4.4 with `prism dump-layout` of `Modifier`, `ModifierElements` and `ModifierElement`. Verify also with the `ui` fixtures "Modifier order matters", "A seventeenth element", "Building units" and "Named and constructed colours are equal".
- [ ] 11.2 Add `foundation.prism` (imports `ui`): `background`, `border`, `clickable` and `toggleable` (registering their handlers, D23), `RoundedCornerShape` and `CircleShape`, `BasicText`, and `BasicTextField` (registering its handler before its node call). Verify also with the fixtures "Names live in their Compose module", "A rounded background around padding" and "A clickable box".
- [ ] 11.3 Add `layout.prism` (imports `ui`): `BoxScope`, `ColumnScope` and `RowScope`; `Arrangement` with its companion `Arrangements` and `spacedBy`; `padding`, `width`, `height`, `size`, `defaultMinSize`, `fillMaxWidth`, `fillMaxHeight` and `fillMaxSize`; `weight` and `align` with their scope contexts (D22); `Box`, `Column`, `Row` and `Spacer`, calling their content with their scope. Verify also with the fixtures "UI toolkit is imported", "A weighted text fills a row" and "A scoped modifier outside its scope".
- [ ] 11.4 Add `material3.prism` (imports `ui`, `foundation` and `layout`): `ColorScheme`, `Typography`, `MaterialThemeValues` and `MaterialTheme`; `ButtonColors`, `ButtonDefaultsValues`, `ButtonDefaults` and `buttonColors`; the `package` surface helper; `Surface`, `Text`, `Button`, `Checkbox` and `TextField`, built as D24 says. Verify with the CLI test of "The UI modules are not in the prelude" in `cli_workspace_tests.rs`, and with the fixtures "Counter screen" and "A button's size and label position".

## 12. Fixtures

Read each snapshot against its scenario's THEN. Verify each task with `cargo test -p prism-cli --test fixture_tests`.

- [ ] 12.1 Add the `ui` fixtures for modules, running apps and their errors:
  - "Headless programs link no window code" is the Rust test of task 4.1, and needs no fixture;
  - "UI toolkit is imported", with the forms `--without-import` (`// expect: E4001`) and `--with-import`;
  - "Names live in their Compose module", with the forms `--without-foundation` (`// expect: E4006`) and `--with-foundation`;
  - "runApp returns and frees everything" (`// headless: 200x100`, no events);
  - "runApp from a handler" (`// run: 101`);
  - "No window and no headless mode" (`// run: 101`, no headless header);
  - "runApp inside a composable" (`// expect: E4030`);
  - "A toolkit composable in a plain composition" (`// run: 101`);
  - "Counter screen";
  - "Text of a state's string";
  - "Calling runApp with its defaults", "The public signature of runApp" and "Unit properties are members of Int", under `standard-library`.
- [ ] 12.2 Add the `ui` fixtures for the composables:
  - "An explicit colour wins over the style";
  - "A button's size and label position" and "A button label is drawn in its content colour";
  - "Toggling a checkbox";
  - "Typing into a text field" and "A field the program does not update";
  - "A surface sets its content colour";
  - "A spacer between texts";
  - "Wrapped text" and "A larger font".
- [ ] 12.3 Add the `ui` fixtures for modifiers, units, colours and layout:
  - "Modifier order matters" and "Order decides what a background covers";
  - "A seventeenth element" (`// run: 101`);
  - "A card with an equal modifier is skipped" and "A card with an equal clickable modifier is skipped";
  - "Chained paddings add up" and "Horizontal padding";
  - "A size larger than the window is clamped", "Padding before or after a size" and "Minimum and fractional sizes";
  - "A weighted text fills a row", "Aligning one child" and "A scoped modifier outside its scope" (with the code `receiver-lambdas` gives);
  - "A rounded background around padding";
  - "A clickable box", "The clickable area follows the chain" and "A toggleable text";
  - "A Copy handler shared by two nodes", "A non-Copy handler cannot be shared" and "Handlers need a running app" (both `// run: 101`);
  - "Building units" and "Named and constructed colours are equal";
  - "A button label is white";
  - "Column stacks children", "Space between two texts" and "Centred content in a box";
  - "Padding around a column".
- [ ] 12.4 Add the `ui` fixtures for nodes, input, the loop, scripts and dumps:
  - "Recomposition updates the node in place", "A removed text is recreated as a new node" and "Keyed items move their nodes";
  - "Replaced handlers are freed";
  - "Click increments a counter", "Releasing elsewhere does not click" and "The topmost button is clicked";
  - "A disabled button";
  - "Backspace on an empty field" and "Two typed characters both arrive";
  - "A handler creates and writes a state";
  - "Frames without changes compose nothing";
  - "A display list dump";
  - "Scripted counter session" and "Resizing the window";
  - "A missing target" and "Typing without a focused field" (`// run: 101`);
  - "A dump of the counter screen";
  - "A rounded border";
  - "Prints and dumps interleave";
  - "Inserting a button before a text keeps the text's node".

## 13. Measurements and integration

- [ ] 13.1 Add the `ui-calls` shape to `crates/prism-cli/tests/compile_timing.rs` (design D19). Run `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` and check that it grows roughly linearly, for "Compile timing of a large screen". Add `crates/prism-cli/tests/ui_run_timing.rs` (`#[ignore]`d): a generated app of 1,000 `Text`s and a counter, run headless with 100 clicks, reporting the median per click. Record both in design.md, D19.
- [ ] 13.2 Move `examples/demo/ui/ui.prism` to the new API: imports of `/prism/compose/ui`, `/prism/compose/layout` and `/prism/compose/material3`, and `Column(modifier = Modifier.padding(16.dp)) { Text("Hello Prism UI") }`. Check that `prism check` accepts it.
- [ ] 13.3 Run the integration checks:
  - `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (no new warnings) and `cargo test --workspace`;
  - `cargo run -q -p prism-cli -- check` on every file under `examples/`, and `cargo run -q -p prism-cli -- fmt --check examples/`;
  - `cargo run -q -p prism-cli -- run --check-allocations examples/run/main.prism`, which must behave as before;
  - the startup benchmark of `run_timing.rs`, which must stay under its 15 ms target with the UI table registered. Record its medians in design.md, D19.

## Workflow follow-up

- At archive, remove from `docs/architecture.md` §5 what this change implements:
  - the `prism-ui` half of "Compose Runtime Crates Behind C Entry Points";
  - "UI Node Tree Owned by the Runtime";
  - "Built-in Composables and Modifiers";
  - "Constraint Layout in Logical Pixels";
  - "Input Events Dispatch to Node Handlers";
  - the display-list sentence of "Rendering Through a Display List", whose window part stays for `desktop-window`;
  - "Frame Loop of runApp", except the reload step;
  - "Headless Rendering and Scripted Input", except `edit` and `--screenshot`.

  In the design notes, record that D17 now uses two node intrinsics and that material is written over foundation (this change's D24), that D18's property struct became Compose's ordered chain (D6) with handlers in a table (D23) and no composed modifiers (D25), that D19's text metrics are an interface with fixed-advance metrics headless (D12), that D22's handlers are owned by nodes through the handler table (D23), and that §5's batching of events became a pass per event (D14).
- At archive, promote D3 to D25 into a "Design Rationale" section of the new `ui` spec, and D4 into `type-system`'s.
- At archive, add the follow-ups of the proposal and of design.md's Open Questions to `ROADMAP.md`, naming `ui-toolkit`. Note for `desktop-window` that it implements `prism_ui::Host` and chooses the window's text metrics (D12).
