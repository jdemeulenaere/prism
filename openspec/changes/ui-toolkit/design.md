# Design: ui-toolkit

## Context

See `proposal.md` for motivation. This is item 30 of "Then: the road to Milestone 2" in `ROADMAP.md`, the UI layer of `docs/architecture.md` §5. It implements `compose-architecture` D1 (the `prism-ui` crate), D17 (toolkit composables in Prism over one node primitive), D18 (modifiers as a `Copy` property struct), D19 (single-pass constraint layout), the display-list half of D20, D21 (headless mode with text dumps) and D22 (handlers owned by nodes). It fills the seams that `compose-runtime` D1 and D9, `compose-transform` D1 and `state-handles` D1 and D12 left for it. Each decision is repeated below where this change refines it. The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Fast compilation is a primary criterion (user, 2026-10-19), so each decision states its compile-time cost next to its Kotlin-likeness, and its run-time cost where it has one.

**Baseline.** This change was proposed while `instance-cache` was being archived. `standard-library`, `compose-runtime`, `compose-transform`, `state-handles` and `hot-reload` were proposed but not implemented. It needs `state-handles` archived, which brings the rest. It relies on these names from those packets:

- **`compose-runtime`:**
  - the `prism-compose` crate, `COMPOSE_FUNCTIONS` and `UnitContext::entry_point(symbol)`;
  - the `Applier` trait, with `root`, `create(kind)`, `place(parent, previous, node)`, `remove`, `release` and `dump`;
  - `Composition::start(applier, content)` and its other Rust API;
  - `prism_compose_start_node_group(composer, key, kind) -> u64`;
  - node groups that place their nodes relative to the previous sibling and remove them at the end of the pass;
  - the rule that no Rust reference is live while program code runs (its D4);
  - `FunctionValue` in `prism-runtime/src/function_value.rs`;
  - one pass at a time per thread, and `composition misuse` errors.
- **`compose-transform`:**
  - `intrinsic struct` and its catalog;
  - `Callee::Composer(ComposerOperation)` and `MirPhase::Composed`;
  - region keys chained from the parent's key (its D5), and stability by declared markers (its D7);
  - lent composable content lambdas (its D14);
  - root content adapters for `composition` and `setContent` (its D15);
  - `E4030`, and default values that are not composable contexts (its D17).
- **`state-handles`:**
  - `State<T>`, `MutableState<T>` and `.value` (a non-`Copy` value borrowable in composable contexts);
  - `Composition::run_handler(handle, f)`, which makes cells created by handlers belong to their composition (its D12);
  - `SideEffect`.
- **`standard-library`:**
  - `crates/prism-library`, `MODULES` and `PRELUDE`;
  - `private external fun` declarations checked against the runtime's tables (`E6005`);
  - the intrinsic catalog (`crates/prism-typechecker/src/intrinsics.rs`, `E4029`), and `library_tests.rs`.
- **Archived or landed:**
  - `escaping-closures`: `escaping` parameters and owned captures; one-word inline environments, so a handler capturing one handle allocates nothing;
  - `collections`: `List<T>`, for the keyed list fixtures;
  - `implicit-it`;
  - the trailing-lambda fix (764fc3f): a trailing lambda binds the last parameter, so `Button(onClick = { ... }) { Text("+") }` fills `content`.

The MODIFIED block of `type-system` was written against the main spec as it stood at proposal time. The `standard-library` delta is ADDED, because that capability does not exist in the main specs until its change is archived. Compare each with the main specs at apply time.

What exists today beyond those packets:

- **Type checker.** `scope.rs::define` rejects a second symbol of one name at one level (`E4010`). `lookup` returns the innermost binding. `lookup_type` skips value symbols. A struct name used as a value is `E4002` ("Type names are not values").
- **ABI.** `FileAbi::symbols` is a list of `ExportedSymbol`s, so two symbols of one name can coexist there.
- **The `prism run` command.** `crates/prism-cli/src/main.rs::run_program` parses `--timings`, `--check-allocations`, `--no-cache` and `--explain-cache` before the file, and reports usage errors with status 1. `run.rs::RunOptions` carries them. `Program::run` (`crates/prism-codegen/src/program.rs`) runs `main` on a thread of its own, between `prism_runtime::start` and `finish`.
- **Runtime output.** Program output goes through `prism-runtime`'s buffer (`output.rs::append`, which is crate-private), flushed before runtime errors.

**Baseline update (`standard-library`, applied 2026-10-18).** Names and mechanisms that differ from what this design assumed:
- Default values on intrinsic parameters are already allowed: the catalog compares declarations without their defaults, and a default is checked and generated in the declaring library file (`standard-library` R8).
- Name lookup has a prelude layer below the imports (`ScopeStack::lookup` and `resolve`: scopes, imports, then the shared prelude names), which companion values must also consult; `/prism/ui` is outside the prelude, so it is reached by `import /prism/ui` as any module.
- Runtime entry points are looked up through `prism_codegen::externals::entry_point`, which `UI_FUNCTIONS` extends, not `UnitContext::entry_point`; `E6005` is checked by `check_file` over `MirFile::externals`.
- `library_tests.rs` checks, formats and compiles every module through `prism_cli::check_library`, and requires a `//` comment directly above every declaration line.

## Goals / Non-Goals

**Goals:**
- Jetpack Compose's foundation surface, with its names and shapes, for the composables and modifiers a counter and a todo app need.
- Toolkit composables that are ordinary Prism code: they skip, restart and hot-reload like user composables, and the Rust side is a node tree with property setters.
- Exact, platform-independent text dumps of the laid-out tree and the display list, driven by scripts, so that executable fixtures pin every behaviour under `--check-allocations`.
- Layout and drawing linear in the nodes, incremental relayout, and steady-state frames that allocate nothing.
- A host interface that `desktop-window` implements without changing the toolkit.

**Non-Goals:**
- Windows, rasterization, fonts and the main-thread split (`desktop-window`); reloads between frames (`compose-hot-reload`); live node counting and frame statistics (`reactive-app-verification`).
- The components and text features that the proposal lists as not in scope.

## Decisions

### D1. The boundary with the other compose changes (autonomous)

| Concern | `ui-toolkit` (this change) | Elsewhere |
|---|---|---|
| Node groups, placement, deferred removal | supplies `UiApplier` | `compose-runtime` |
| `Node`, `LeafNode`, `runApp` lowering | catalog entries, transform lowering, backend | the transform's machinery is `compose-transform`'s |
| Handlers and the cells they create | dispatch through `Composition::run_handler` | ownership is `state-handles`' |
| Display list | built and dumped | rasterized in a window by `desktop-window` |
| Text metrics | the `TextMeasurer` interface and the fixed-advance metric | Inter's metrics in windows, if `desktop-window` chooses (D12) |
| Frame loop | events, passes, layout, display list, idling, hosts | the window host and `--screenshot` (`desktop-window`); the reload point (`compose-hot-reload`); `--frame-stats` (`reactive-app-verification`) |
| Leak checking | nodes own and release their values, so the allocation check covers them | counting live nodes (`reactive-app-verification`) |

Rejected: the window in this change (the ROADMAP split it out, process row P10), and a separate `ui-layout` change (layout cannot be tested without the tree and the dumps).

### D2. A `prism-ui` crate behind C entry points, with per-thread state (autonomous; implements `compose-architecture` D1)

`crates/prism-ui` depends on `prism-runtime` and `prism-compose`. It holds the node arena and tree, layout, the display list, input, the app and frame loop, the host interface, the headless host and its script parser, the dumps, and the `extern "C"` entry points listed in `UI_FUNCTIONS`, a table of `prism_runtime::RuntimeFunction`s with the same source-reading test as `COMPOSE_FUNCTIONS`. `prism-codegen` depends on it only for that table: `UnitContext::entry_point` searches `RUNTIME_FUNCTIONS`, then `COMPOSE_FUNCTIONS`, then `UI_FUNCTIONS`, and `Linker::new` registers all three. The crate has no window dependency (§5 D1), and `prism-desktop` will depend on it, not the reverse.

Like `prism-compose` (`compose-runtime` D4), its state is per thread: a `thread_local!` holds the node arena and at most one running app. Nothing in the state is borrowed while program code runs, whether drop entries of released strings and handlers or the handlers themselves. A handler's function value is copied out of its node before it is called, and a released node's values are taken out before they are dropped.

Run-time cost: one thread-local access per entry point. Compile-time cost: one more table in the entry-point lookup, once per external a unit declares.

Rejected: UI code in `prism-compose` (§5 D1 keeps it UI-agnostic); a global locked state (a lock per setter); one crate with the window (headless tests would build `winit`).

### D3. Toolkit composables in Prism over `Node` and `LeafNode` (autonomous; implements `compose-architecture` D17)

`crates/prism-library/prism/ui.prism` (`package prism`, module `/prism/ui`, in `MODULES` but not in `PRELUDE`) declares the toolkit. Its private core is:

```
private intrinsic struct NodeId : Copy
private intrinsic compose fun Node(kind: Int, update: (NodeId) -> Unit, content: compose () -> Unit)
private intrinsic compose fun LeafNode(kind: Int, update: (NodeId) -> Unit)
private external fun prism_ui_set_text(node: NodeId, text: &String)
private external fun prism_ui_set_padding(node: NodeId, start: Int, top: Int, end: Int, bottom: Int)
...
```

and each public composable is a few lines over it, for instance:

```
public compose fun Text(text: &String, modifier: Modifier = Modifier, color: Color = Color.Unspecified, fontSize: Int = 16) {
    LeafNode(textKind) { node ->
        prism_ui_set_text(node, text)
        prism_ui_set_text_style(node, color.argb, fontSize)
        applyModifier(node, modifier)
    }
}
```

- **Lowering.** `Node(kind, update, content)` lowers in the transform to `node = start_node_group($composer, key, kind)`, a call of `update(node)`, a call of `content($composer, 0)`, and `end_group($composer)`. Here `key` is `H(parent, "node", <ordinal among the Node and LeafNode calls of the parent region>)`, chained as `compose-transform` D5 chains region keys. `LeafNode` is the same without `content`. Two intrinsics replace Compose's two `ComposeNode` overloads, since Prism has no overloading, and a leaf passes no empty content lambda, which would cost a group per leaf per pass.
- **`update` runs on every pass of its group**, as Compose's `update` block does. Each setter compares the new value with the node's and marks the node dirty only when it differs (D11), so an unchanged recomposition costs a few calls and compares. `update` is lent (not `escaping`), so it borrows the composable's parameters (`text: &String`) and allocates nothing.
- **Kinds** are private `Int` constants: column 1, row 2, box 3, text 4, button 5, spacer 6, checkbox 7, text field 8. The window root, kind 0, is not a composable. The runtime rejects an unknown kind with `ui misuse: unknown node kind <k>`.
- **`NodeId`** is the state-handle invariant's node handle: an intrinsic struct of one word, `Copy`, compared by its word (`compose-transform` D16). It is private because no public API hands one out yet. A custom-layout or test API would make it public.
- **Setters** are split by what they invalidate. `set_text`, `set_text_style`, `set_padding`, `set_size` and `set_arrangement` invalidate layout. `set_background`, `set_border`, `set_enabled` and `set_checked` invalidate drawing only. `set_on_click`, `set_on_checked_change` and `set_on_value_change` replace a handler and invalidate nothing.

Compile-time cost: one module of about 400 lines, parsed and checked only by programs that import it, and cached under the compiler version (`standard-library` D3). Each `Node` call is constant work in the transform.

Rejected:
- Composables implemented in Rust (§5 D17): no hot reload, no skipping, and the type checker would need tables of their signatures.
- A public `Node`: kinds are a closed set implemented in Rust.
- One setter taking every property: every change would invalidate layout.
- An enum of kinds: the runtime receives a word, and a `when` per call would cost more than a constant.

### D4. Companion values: a struct and a top-level `val` may share a name (autonomous)

A top-level `val` may have the name of a struct declared in the same file. In a type position the name means the struct, and a call `Name(...)` is the struct's constructor. Anywhere else in a value position, `Name.field` and `Name.f()` included, the name means the value. Every other pair of same-named top-level declarations is still `E4010`: two `val`s, a `val` and an enum, function or extension, or a `val` and a struct of another file (an import included). A struct without a companion value is still not a value ("Type names are not values"). The ABI exports both symbols under the name, and an import brings both.

This is how `/prism/ui` gets Compose's exact spellings:
- `public val Modifier: Modifier` is the empty modifier, so `Modifier.padding(16).fillMaxWidth()` and `modifier: Modifier = Modifier` read as in Compose.
- `public val Color: Colors` holds Compose's named colours as fields, so `Color.Red` and `Color(0xFF336699)` both work.
- `public val Arrangement: Arrangements` gives `Arrangement.Top` and, through `fun Arrangements.spacedBy(space: Int): Arrangement`, `Arrangement.spacedBy(8)`.

Kotlin reaches the same spellings with companion objects. Here a companion is just a value of a struct type, so no new kind of declaration, no static members and no new lookup rule beyond the name split are needed. User code can use the same pattern for its own themes and constants.

Compile-time cost: one filtered lookup when a name in a value position finds a struct (the scope already filters types for `lookup_type`), and one kind comparison in the duplicate check. Nothing is inferred.

Rejected:
- `Modifier()` through default values on struct fields: a larger language feature (field defaults in the ABI, generators per field, constructor matching), and still not Compose's spelling.
- Enum colours (`Color.Red`, `Color.Argb(...)`): two representations of one colour, which would compare unequal.
- Lower-case values (`emptyModifier`) or names unlike Compose.
- Kotlin's `companion object` or static members: a new declaration form and member namespace for what a value already does.
- Allowing the pair across files: an import would silently change what a local struct name means.

### D5. The public API: Compose's names, parameters and defaults (autonomous)

| Composable | Signature (defaults as in Compose where they exist) |
|---|---|
| `Column` | `(modifier: Modifier = Modifier, verticalArrangement: Arrangement = Arrangement.Top, horizontalAlignment: Alignment = Alignment.Start, content: compose () -> Unit)` |
| `Row` | `(modifier = Modifier, horizontalArrangement = Arrangement.Start, verticalAlignment = Alignment.Top, content)` |
| `Box` | `(modifier = Modifier, contentAlignment = Alignment.TopStart, content: compose () -> Unit)` |
| `Text` | `(text: &String, modifier = Modifier, color: Color = Color.Unspecified, fontSize: Int = 16)` |
| `Button` | `(onClick: escaping () -> Unit, modifier = Modifier, enabled: Bool = true, content: compose () -> Unit)` |
| `Spacer` | `(modifier: Modifier)` |
| `Checkbox` | `(checked: Bool, onCheckedChange: escaping (Bool) -> Unit, modifier = Modifier, enabled: Bool = true)` |
| `TextField` | `(value: &String, onValueChange: escaping (String) -> Unit, modifier = Modifier, enabled: Bool = true)` |
| `runApp` | `(title: &String = "Prism", width: Int = 800, height: Int = 600, content: escaping compose () -> Unit)` |

Differences from Compose, each a follow-up:
- `Box` requires its content, because a composable lambda default would be built in a generator for nothing.
- `onCheckedChange` is not nullable.
- There are no scopes (`RowScope`, `ColumnScope`), so `weight` is a plain modifier that only rows and columns read.
- `Button` has no `colors` or `contentPadding`.
- `TextField` has no `placeholder`, `label`, `singleLine` or `keyboardActions`: it is always single-line.
- `Text` has no `fontWeight`, `textAlign` or `maxLines`.

`Checkbox` and `TextField` are in scope, as §5 D17 and the ROADMAP item say, because the todo app of `reactive-app-verification` needs them.

`runApp` is an intrinsic, not a Prism function over one, so that `E4030` can name it and so that its content gets a root content adapter where it is called (D15). Its defaults are ordinary default values, which the standard-library delta allows on intrinsics: a default is a generator in the declaring file, independent of how the call is lowered.

Compile-time cost: none beyond the module (D3). Kotlin-likeness: Compose's parameter names, order and defaults, so a call written for Compose compiles here when it uses only these parameters, with `Int` for `Dp` (D7) and `1.0` for `1f`.

Rejected: §5's first sketch of per-composable layout parameters (`Column(spacing = 8)`), since Compose spells them `verticalArrangement = Arrangement.spacedBy(8)`; a smaller set without `Checkbox` and `TextField`; Material extras now.

### D6. `Modifier` is a `Copy` property struct built by chained calls (autonomous; implements `compose-architecture` D18)

```
public struct Modifier : Copy {
    val paddingStart: Int, val paddingTop: Int, val paddingEnd: Int, val paddingBottom: Int,
    val width: Int, val height: Int,                 // -1: unspecified
    val fillMaxWidth: Bool, val fillMaxHeight: Bool,
    val weight: Double,                              // 0.0: none
    val background: Color, val backgroundShape: Shape,
    val borderWidth: Int, val borderColor: Color, val borderShape: Shape,
}
public val Modifier: Modifier = Modifier(0, 0, 0, 0, -1, -1, false, false, 0.0, Color.Transparent, RectangleShape, 0, Color.Transparent, RectangleShape)
```

The modifier functions are `public fun Modifier.f(...): Modifier` extensions with shared receivers, each returning a copy with one property set:
- **Padding.** `padding(all: Int = 0, horizontal: Int = all, vertical: Int = all, start: Int = horizontal, top: Int = vertical, end: Int = horizontal, bottom: Int = vertical)`. This is Compose's three `padding` overloads in one signature, through defaults that read earlier parameters, so `padding(16)`, `padding(horizontal = 8)` and `padding(start = 4)` all mean what they mean in Compose.
- **Size.** `width(width)`, `height(height)`, `size(width, height = width)`, `fillMaxWidth()`, `fillMaxHeight()`, `fillMaxSize()` and `weight(weight: Double)`.
- **Drawing.** `background(color, shape = RectangleShape)` and `border(width, color, shape = RectangleShape)`.

Order does not matter (§5 D18): `padding(8).background(c)` equals `background(c).padding(8)`, and the background always covers the padding area. Paddings add up, as nested paddings do in Compose: `padding(8).padding(4)` is 12. Every other property takes its last value. Negative paddings and sizes are laid out as 0.

`Shape` is `struct Shape : Copy { val cornerRadius: Int, val cornerPercent: Int }`, with `RectangleShape`, `CircleShape` (50 percent) and `RoundedCornerShape(radius: Int)`. The radius drawn is the larger of the two, the percentage taken of the smaller side.

A modifier is 16 words. It is stable as a parameter (`compose-transform` D7) and compared through its equality glue. Passing one costs a copy, and setting it on a node costs four setter calls (padding, size, background, border).

`Modifier.clickable` is left out: a handler is an owned closure, which a `Copy` struct cannot hold. Clickable areas are `Button`s, and `clickable` is a follow-up for when function values can be kept in values that compare.

Compile-time cost: none beyond the module. Run-time cost: a modifier chain of k calls copies k structs of 128 bytes, with no allocation.

Rejected:
- Compose's ordered chain (§5 D18): an allocated list per call, and unstable parameters.
- Overload-like separate names (`paddingHorizontal`).
- Nullable fields (`Int?`): two words per field and a tag in the layout, for what a sentinel says.
- Last-wins padding: Compose users expect chained paddings to add up.

### D7. Sizes are `Int` logical pixels, without `Dp` or `sp` (autonomous)

Compose writes `16.dp` and `14.sp`. Prism has no extension properties outside the library and no unit types, and §5 already fixed integer logical pixels so that dumps are exact. Sizes, paddings, spacings and font sizes are `Int`s, and `desktop-window` scales them by the window's scale factor. Rejected: a `Dp` struct with an `intrinsic val Int.dp` (a wrapper on every number for no checking benefit today; it can be added later with unit types).

### D8. Text parameters are borrowed (autonomous; answers the `state-handles` risk)

`Text` and `TextField` take `&String`. A literal, a template temporary or a place is then borrowed at the argument site (`references`, "Implicit Shared Borrow at Argument Sites"). So `Text("+")`, `Text("Count: ${c.value}")` and `Text(name.value)` for a `State<String>` all work without `.clone()`, where an owned `String` parameter would make `Text(name.value)` `E4017`. `prism_ui_set_text` compares the bytes and copies them into the node's own string only when they differ, so an unchanged text costs one comparison and no allocation. A template still allocates its result, as it must. A `&String` parameter makes `Text` not skippable (`compose-transform` D7), and it would not be skippable with a `String` either.

Rejected: an owned `String` (`E4017` for state contents, and a move into the node that still needs a compare); both through two names.

### D9. Colours, and content colour inherited at draw time (autonomous)

`public struct Color : Copy { val argb: Int }` holds an ARGB value in the low 32 bits. Its companion `Color: Colors` (D4) holds Compose's `Black`, `DarkGray`, `Gray`, `LightGray`, `White`, `Red`, `Green`, `Blue`, `Yellow`, `Cyan`, `Magenta`, `Transparent` and `Unspecified`, with Compose's values. `Unspecified` is `Color(0x1_0000_0000)`, outside the ARGB range, and any colour outside that range is unspecified.

Compose resolves a `Text`'s default colour through `LocalContentColor`, a composition local. Prism has none. Instead, every node has a content colour, and the display list resolves an unspecified text colour to the nearest ancestor's content colour in its top-down walk, or to black at the root. `Button` sets white (disabled: `#61000000`), and the other nodes inherit. This is O(1) per node in a walk that already happens, and it gives `Button { Text("+") }` a readable label.

The default look follows Material 3's baseline where a value is needed:
- **`Button`:** container `#FF6750A4`, disabled `#1F000000`; full pill shape; content padding 24 horizontal and 8 vertical; minimum size 58×40.
- **`Checkbox`:** 20×20, border 2 `#FF49454F`, checked fill `#FF6750A4`.
- **`TextField`:** minimum size 200×(line height + 16), inner padding 8; border 1 `#FF79747E`, focused 2 `#FF6750A4`; a 1-pixel caret at the end of the text when focused.
- **The window:** cleared to white.

Rejected:
- Composition locals now: a compose-runtime feature of its own.
- A black default text colour: unreadable on a button.
- `ButtonColors` now: a follow-up with theming.

### D10. The node tree: a generational arena, linked children, owned values (autonomous)

- **Records.** Each node record holds:
  - its kind, creation number, tree, parent, first and last child, previous and next sibling;
  - its properties: text, text style, the modifier fields, arrangement, alignment, enabled, checked, content colour;
  - its handlers, as `Option<FunctionValue>` per kind;
  - its layout state: the last constraints, size and position relative to its parent, and its dirty flags.
- **Handles.** A `NodeId` is a 32-bit index and a 32-bit generation (`handle.rs`, as in `prism-compose`). Releasing a node bumps its generation, and an index whose generation reaches the maximum is retired.
- **Placement.** `UiApplier` implements `compose-runtime`'s `Applier`:
  - `create(kind)` allocates a node;
  - `place(parent, previous, node)` links it after `previous`, and does nothing when it is already there;
  - `remove` unlinks it at the end of the pass and marks the parent's layout dirty;
  - `release` takes its strings and handlers out and drops them after the arena is let go, children first, as `compose-runtime` D7 orders releases.
  Linked siblings make placement after a sibling O(1) and keep traversal in order. Creation numbers count from 1 per app and are never reused, so dumps show identity: a moved node keeps its number, and a recreated one gets a new one.
- **Validation.** A setter validates the `NodeId`'s generation, and also that the thread's active pass belongs to the running app's composition. That rejects a `/prism/ui` composable composed in a plain `composition { }`, whose node tree applier made the handle, and a setter called through a remembered `NodeId` outside composition (D17).
- **Ownership.** A node owns its text (a `PrismString` from the runtime's allocator, so `--check-allocations` tracks it) and its handler function values (`compose-architecture` D22). A new handler replaces the old one, which is released at once, after the setter has let go of the state: it is not running, since handlers run only between passes.

Rejected:
- Children as vectors: placement after a sibling would be O(children).
- Reusing compose's node tree applier with side tables: two arenas to keep in step.
- Comparing a new handler with the old one to keep the old: replacing is one release, and the comparison would need the descriptor equality entry for no gain.

### D11. Constraint layout, measure policies and the cache (autonomous; implements `compose-architecture` D19)

Constraints are minimum and maximum widths and heights in `i64` logical pixels, and a maximum may be unbounded. Measuring a node under constraints `c` takes four steps:

1. **Its own size.** A fixed `width` becomes an exact width clamped into `c`. Otherwise, `fillMaxWidth` under a bounded maximum becomes an exact width of that maximum. The same holds for the height.
2. **Padding.** The padding deflates the result into the inner constraints, floored at 0.
3. **Content.** The kind's policy measures the content.
4. **The result.** The size is the content plus the padding, clamped into the constraints of step 1.

| Kind | Policy |
|---|---|
| window (root) | constraints 0..width × 0..height; children like a `Box` aligned `TopStart` |
| `Text` | lines from word wrapping at the inner maximum width (D12); width of the widest line, height of the lines times the line height, at least one line |
| `Spacer` | content 0×0, so its size comes from its modifier |
| `Box` | children measured with the inner constraints loosened to minimum 0; size the largest child, at least the inner minimum; each child placed by `contentAlignment` |
| `Column` | children without a weight measured in order, with width 0..inner maximum and height 0..what remains; then weighted children share what remains of a bounded maximum by weight, as exact heights (each `remaining * w / total` rounded down, the last taking the rest); width the widest child, height the sum plus the gaps; children arranged on the main axis and aligned by the horizontal part of `horizontalAlignment` |
| `Row` | the same, along the other axis |
| `Button` | its content measured as a `Row` centred on both axes inside a content padding of 24×8, at least 58×40 |
| `Checkbox` | content 20×20 |
| `TextField` | one unwrapped line of its value inside an inner padding of 8, at least 200 wide |

Arrangements with free space `f` and `n` children:
- `Top` and `Start` place the children from 0, and `Bottom` and `End` from `f`.
- `Center` places them from `f / 2`.
- `SpaceBetween` puts `f / (n - 1)` between them, and places a single child first.
- `SpaceAround` puts `f / n` between them, with half that before the first.
- `SpaceEvenly` puts `f / (n + 1)` before each one.
- `spacedBy(s)` puts `s` between them and places them from the start.

Remainders of the integer divisions are dropped, so positions are exact integers on every platform. An alignment has a horizontal and a vertical part (`TopStart` is start and top, `CenterHorizontally` is centre and top), and a child is placed at 0, `(free) / 2` or `free` on each axis.

- **The cache.** Every setter that changes a layout property marks its node `needs_measure` and its ancestors `child_needs_measure`, stopping at the first ancestor already marked. Layout starts at the root. A node that is neither, and is measured with its last constraints, returns its last size without visiting its subtree. Positions are relative to the parent, so an unchanged subtree needs no placement. Absolute bounds are computed on the walks that need them: hit testing, the display list and dumps.
- **Cost.** A full layout is O(nodes). A relayout is O(the dirty nodes, their ancestors, and the children of each re-measured node), each clean child costing one comparison. Text measurement is O(characters) for a node whose text or constraints changed.

Rejected:
- A flexbox engine such as `taffy` (§5 D19).
- Floating-point layout: dumps would depend on rounding.
- Re-measuring every node every frame.
- Compose's intrinsic measurements (§5 D19, a follow-up).

### D12. Text measurement through an interface; a fixed-advance metric headless (autonomous; revises `compose-architecture` D19)

Layout measures text through `trait TextMeasurer { fn advance(&self, character: char, font_size: i64) -> i64; fn line_height(&self, font_size: i64) -> i64; }`. Word wrapping is the toolkit's own, the same for every measurer. Each `\n` starts a line. Lines break greedily at spaces, and a space at a break is not drawn. A word wider than the line stays whole and overflows.

This change ships `FixedAdvanceMetrics`. Every character advances `font_size / 2`, and the line height is `font_size * 5 / 4`, so 16-pixel text is 8 pixels per character and 20 pixels high. Headless runs, and therefore every fixture, use it. §5 D19 planned the bundled font's metrics in every renderer, so that headless layouts equal windowed ones. That would make each dump depend on the font file, its version and kerning, and on a font that only `desktop-window` brings, and dumps would stop being readable arithmetic. The interface leaves `desktop-window` the choice: lay out windows with Inter's metrics through `fontdue`, so a window looks right but its layout differs from the headless one, or with the fixed metric and draw Inter glyphs in its boxes. Its design records the choice. The app takes its measurer when it starts, so one run uses one measurer throughout.

Rejected:
- Inter's metrics headless (above).
- A metrics table generated from Inter now: it needs the font that `desktop-window` adds, and it ties fixtures to it.
- Proportional test metrics: harder to compute in a scenario.

### D13. The display list (autonomous; implements the display-list half of `compose-architecture` D20)

```
pub enum DrawCommand {
    Clear { width, height, color },
    Rectangle { bounds, color, corner_radius },
    Border { bounds, width, color, corner_radius },
    Text { x, y, font_size, color, text: Range<usize> },    // y: the top of the line box
}
pub struct DisplayList { commands: Vec<DrawCommand>, text: String }
```

After layout, when any property that is drawn or any layout result changed, the app rebuilds the list in one tree walk, parents before children:
- the clear;
- for each node: its background, its border and its kind's own commands (a button's container, a checkbox's box and fill, a text field's frame and caret, one text command per line), with content colours resolved (D9).

The vectors are cleared and reused, so once they have grown, a rebuild allocates nothing. Rebuilding the whole list is O(nodes) and happens at most once per frame. A partial rebuild by dirty region is a follow-up, if frame times ask for it. `desktop-window` rasterizes the list. The headless host prints it on `dump draw` (D16). It is also the place a GPU backend would start from (§5 D20).

Rejected:
- Drawing directly from the tree in the renderer: the renderer would need the toolkit's styling rules.
- An ASCII-art rasterizer for snapshots: a character grid loses exact positions and sizes, which the text dump of the list keeps.

### D14. Input: hit testing, clicks on release, focus, and a pass after every event (autonomous; implements `compose-architecture` D22, revises the batching of §5 "Input Events Dispatch to Node Handlers")

- **Hit testing** walks the laid-out tree in reverse drawing order (last child first, children before their parent). It returns the first node whose bounds contain the point and that takes input: a `Button`, a `Checkbox` or a `TextField`, enabled or not.
- **Clicks.** A press records the hit node. A release calls that node's handler when the release hits the same node and the node is enabled: `onClick()`, `onCheckedChange(!checked)`, or focusing a text field. A disabled control absorbs the press and the release and calls nothing, so a click never falls through to what is under it. A press that hits no text field clears the focus.
- **Keys** go to the focused, enabled text field, which builds the new value from its current one and calls `onValueChange` with it, by move:
  - a typed character is appended to the value;
  - backspace removes the last character, and does nothing when the value is empty;
  - enter is consumed and does nothing until keyboard actions exist.
  The field shows its `value` parameter, never a text of its own. If the program does not write the state, the field does not change, as in Compose's value-based `TextField`.
- **Handlers run through `Composition::run_handler`** (`state-handles` D12), so the cells they create belong to the app's composition, and their writes are allowed, since no pass is running.
- **A pass after every event.** After dispatching one event, the loop recomposes the invalid scopes and lays out again before it takes the next event. §5 planned one pass after all queued events. But then the second character of a fast `type "ab"` would be appended to the field's value from before the first one, and `click "<text>"` could not find a node that the previous click created. Per-event passes make each event see the previous event's result, which is what a user sees. They cost one pass per event, and events arrive at human rates. Pointer moves are not dispatched, since nothing uses hover.

Run-time cost: hit testing is O(nodes) in the worst case and O(depth) when bounds prune. A dispatch allocates only what the handler allocates, plus the new `String` of a text-field edit.

Rejected:
- Batched passes (above).
- Clicks on press (§5 D22).
- A field-owned text buffer that updates before the state does: it diverges when the program rejects an edit.
- Disabled controls that pass clicks through.

### D15. `runApp`, the frame loop and hosts (autonomous)

`runApp`'s lowering builds a root content adapter around `content`, as `composition` does (`compose-transform` D15), and calls `prism_ui_run_app(title, width, height, content)`. On the program thread, the entry:

1. Rejects a second app on the thread: `ui misuse: runApp called while an app is running`. During a pass, `Composition::start` raises `composition misuse: composition operation during a pass` anyway.
2. Takes the host that the CLI configured (`host.rs::configure`, D16). Without one, it raises `ui misuse: no window system is available; run with --headless`, until `desktop-window` installs a window host.
3. Creates the tree with a window root of the host's size, and starts the composition with `UiApplier` (first pass).
4. Loops:
   - while a scope is invalid, run a pass;
   - while layout is dirty, lay out;
   - otherwise block on `host.next_event()`, and dispatch what it returns (D14, followed by a pass and layout);
   - on `Resize`, set the root's size and mark it dirty;
   - on `Frame`, rebuild the display list if it is stale and call `host.present(&list)`;
   - on `Dump`, print;
   - on `Close` or the end of events, stop.
5. Drops the composition, which removes every group and node and releases what nodes own, then the tree, and returns.

```
pub trait Host {
    fn size(&self) -> (i64, i64);
    fn next_event(&mut self) -> Option<HostEvent>;     // None ends the app
    fn present(&mut self, list: &DisplayList);
}
pub enum HostEvent { Press(i64, i64), Release(i64, i64), Character(char), Key(Key), Resize(i64, i64), Frame, Dump(DumpKind), Script(ScriptCommand) }
```

- **Idling.** The loop blocks only when nothing is invalid or dirty, so an idle app runs no pass, layout or display list build (§5 "Idle apps do no work").
- **Script steps.** A window host asks for a frame after each change. The headless host returns script steps. Those that name text (`click "+"`) are resolved by the loop against the current tree (D16) as `HostEvent::Script`.
- **Side effects that keep invalidating** recompose on every iteration, as they would recompose on every frame in Compose.
- **The seams for later changes.** `compose-hot-reload` applies reloads at the top of the loop, between events. `desktop-window` implements `Host` over channels to the main thread.
- **`E4030`.** It extends to `runApp` in composable contexts, the same check of the composable-context flag as for `composition`, constant per call.

Rejected:
- `runApp` as a Prism function over a private intrinsic: `E4030` would not see it.
- Several apps per thread: multiple windows are a follow-up.
- A frame loop in Prism: it needs blocking, the host and timers, all runtime concerns.
- Running headless by default without `--headless`: a program that wants a window would silently do nothing.

### D16. Headless runs, event scripts and dumps (autonomous; implements `compose-architecture` D21)

- **Options.** `prism run --headless[=<width>x<height>] [--events <script>] <file>`. The size overrides `runApp`'s. Without `--events`, the app composes, lays out and returns. `--events` without `--headless` is a usage error.
- **Parsing before running.** The CLI reads and parses the script before it compiles the program. A syntax error is reported as `Error: <script>:<line>: <message>` with status 1, and nothing runs.
- **The grammar.** One command per line. Blank lines are allowed, `#` starts a comment, and strings are double-quoted with `\"`, `\\` and `\n` escapes. The commands:
  - `click "<text>" [<n>]` and `click <x> <y>`;
  - `press "<text>" [<n>]`, `press <x> <y>`, `release "<text>" [<n>]` and `release <x> <y>`;
  - `type "<text>"`;
  - `key backspace` and `key enter`;
  - `resize <width> <height>`;
  - `frame`;
  - `dump`, `dump tree` and `dump draw`.
  `edit` is reserved for `compose-hot-reload`.
- **Text targets.** `"<text>" [<n>]` names the n-th node (default 1), in tree order, whose text is exactly the string: a `Text`'s text or a `TextField`'s value. The command acts at the centre of that node's bounds, rounded down, and hit-tests normally from there, so `click "+"` clicks the button around the `+`. A target that matches no node ends the program with `event script line <n>: no node shows the text "<text>"`. `type` and `key` without a focused field end it with `event script line <n>: no text field is focused`. Scripts name targets by text so that they survive layout changes (§5 D21).
- **`frame`** builds the display list as a window frame would, and prints nothing.
- **`dump`** prints the window line `window <width>x<height>`, then one line per node in tree order, indented two spaces per depth: `<kind> #<number> <x>,<y> <width>x<height>`, with absolute bounds, followed by its non-default properties in this fixed order:
  - the text, quoted;
  - `color`, `size`, `checked`, `disabled` and `focused`;
  - `arrangement` and `alignment`;
  - `padding <all>` or `padding <s>,<t>,<e>,<b>`, `width`, `height`, `fillMaxWidth`, `fillMaxHeight`, `weight`, `background` (with its shape) and `border`.
  Colours print as `#AARRGGBB`.
- **`dump draw`** prints one display list command per line: `clear`, `rect`, `border` and `text`.
- **Output.** Dumps go through the runtime's output buffer (`prism_runtime::write_output`, made public), so they interleave with `println` in program order and are flushed before a runtime error.
- **Fixtures.** The runner reads `// events: <file>` and `// headless: <width>x<height>` header lines. Either one adds `--headless[=<size>]`, and `// events:` adds `--events <file>` (relative to the case directory, the working directory), to the usual `prism run --check-allocations` of executable cases. A script error is a case with `// run: 1`.

Rejected:
- PNG goldens (§5 D21).
- A Prism-level test API (§5 D21: it needs `prism test`).
- Scripts parsed by the runtime at `runApp`: syntax errors would surface after the program's earlier output.
- Silently ignoring missing targets: a script that clicks nothing is a broken test.

### D17. UI runtime errors are without a location (autonomous)

Two new `PanicKind`s of `prism-runtime`, after `state-handles`' kinds, both raised with `panic_without_location`, status 101:

- `UiMisuse`, `ui misuse: <detail>`, with the details:
  - `/prism/ui composables run only inside runApp`: a setter during a pass of another composition;
  - `node property set outside composition`: a setter outside any pass;
  - `stale node handle`;
  - `unknown node kind <k>`;
  - `runApp called while an app is running`;
  - `no window system is available; run with --headless`.
- `EventScriptFailed`, `event script line <n>: <detail>`.

An entry point has no caller location (`standard-library` D10, `compose-runtime` D12). The misuse that ordinary programs can reach (`runApp` in a handler, toolkit composables in a plain composition) is clear without one, because the detail names the construct.

Rejected: one kind for both (a script failure is not a program bug); locations through extra arguments on every setter (code on every call for errors that only misuse reaches).

### D18. Hot reload interplay (implements the UI side of `compose-architecture` D23)

- **Nodes belong to node groups.** Their keys come from the toolkit composable's body and the `Node` call's position in it (D3), under the user composable's body key (`compose-transform` D5). An edit that keeps a composable's construct path keeps its groups, and therefore its nodes, with their focus and layout caches.
- **Kind changes.** A node group whose kind changed gets a new node (`compose-runtime` D9). Calling `Button` where `Text` was is a different body key anyway.
- **Handlers.** They are function values called through their descriptors' slots, so a reloaded handler body runs on the next click. A handler whose captures changed has a new slot (`hot-reload` D5), and the node keeps the old value until its composable recomposes and replaces it.
- **The reload point** is at the top of the frame loop, between events (D15), which `compose-hot-reload` fills.

### D19. Costs, and how they are measured

| Run-time operation | Cost |
|---|---|
| toolkit composable per pass | one node group, 4 to 7 setter calls each doing an O(1) compare (text: O(length)), no allocation when nothing changed |
| full layout | O(nodes + characters of text measured) |
| relayout | O(dirty nodes + their ancestors + the children of each re-measured node) |
| display list | O(nodes), rebuilt only when something drawn or laid out changed, into reused buffers |
| hit test | O(nodes) worst case |
| dispatch | O(1) plus the handler, plus a `String` per text-field edit |
| idle | nothing |
| memory | one node record (about 200 bytes) per node, plus its text |

- **`crates/prism-ui/tests/allocations.rs`** installs a counting `#[global_allocator]` and drives an app from Rust through `test_support.rs` (hand-built content and handler function values, as `prism-compose`'s tests do). It checks that a steady-state iteration allocates nothing: an event whose handler writes nothing, then a pass that has nothing to do, a clean layout and a display list rebuild.
- **`crates/prism-ui/tests/ui_timing.rs`** is an ignored benchmark in the style of `compile_timing.rs`. It builds trees of 1,000, 10,000 and 100,000 nodes (a column of rows, each holding a text and a button) through the tree's Rust API. It reports full layout, relayout after one text change, a display list build and a hit test. Under `PRISM_ASSERT_TIMING=1` it asserts roughly linear growth, and that relaying out one leaf of the 10,000-node tree costs under a tenth of a full layout. Medians are recorded here at apply time.
- **`crates/prism-cli/tests/ui_run_timing.rs`** (ignored) runs a generated app of 1,000 `Text`s and a counter headless, with a script of 100 clicks, and reports the median per click. Its release-build target is `reactive-app-verification`'s.

| Compile-time stage | Cost |
|---|---|
| companion lookup (D4) | one filtered lookup when a value name finds a struct; one kind compare per duplicate |
| `Node` and `LeafNode` lowering (D3) | constant per call, one key hash |
| `runApp` (D15) | one adapter per calling body, one `E4030` flag check |
| `/prism/ui` | one library module, checked once per compiler version and cached; parsed only when imported |

`compile_timing.rs` gains `ui-calls`, one composable calling `n` toolkit composables with modifier chains inside a `Column`, at its usual sizes, and asserts roughly linear growth under `PRISM_ASSERT_TIMING=1`.

### D20. A new `ui` capability, and how the change is tested

The toolkit's behaviour is new and is a library over compose, so it is a new `ui` capability rather than a part of `compose`. The language rule goes to `type-system` and `compilation-and-abi`, and the library entries to `standard-library`. The node-group operation goes to `mir`, the entry tables and word-sized intrinsic structs to `code-generation`, and the CLI options to `execution`. The diagnostics catalog is edited directly: `E4010` gains the companion exception, and `E4030` names `runApp`. No error code is added.

- **Fixtures.** Behaviour a program can observe is pinned by executable fixtures under `crates/prism-cli/tests/fixtures/ui/`, nearly all headless with scripts and `dump`, under `--check-allocations`. Companion values have `type-system` and `compilation-and-abi` fixtures, and node groups a `mir` fixture with `composed-mir.snap`.
- **Rust tests.** What a program cannot observe is pinned by unit tests in `prism-ui`: the layout cache, release order, re-entrancy, measurer independence and the allocation-free steady state.

## Risks / Trade-offs

- **[Headless and windowed layouts may differ]** if `desktop-window` lays out with Inter's metrics (D12). → Fixtures pin the headless layout, and the window decision is `desktop-window`'s, recorded there.
- **[Companion values are a language rule added by a library change]** → It is small, Kotlin-shaped and general (D4). Its scenarios are in `type-system`, and it can be revisited before release.
- **[`Modifier` is 16 words]** Each chain call copies 128 bytes, and skippable composables keep a modifier in a slot block. → Copies are cheap next to a pass, and a packed representation can come later behind the same API.
- **[A pass per event]** A burst of events recomposes once per event (D14). → Events arrive at human rates, and pointer moves are not dispatched.
- **[Full display list rebuilds]** Every change rebuilds the whole list (D13). → O(nodes) with reused buffers; dirty regions are a follow-up for large trees.
- **[No `Modifier.clickable`]** Clickable areas must be `Button`s. → A follow-up once function values can sit in comparable values.
- **[Prerequisites unimplemented]** Every name taken from `compose-runtime`, `compose-transform`, `state-handles` and `standard-library` comes from their packets. → Task group 1 checks them against the code those changes leave.

## Migration Plan

No program that runs today changes meaning. A same-file pair of a struct and a top-level `val` with one name was `E4010` and is now accepted. A struct name used as a value without a companion is still `E4002`. `examples/demo/ui/ui.prism` moves from `Column(padding = 16)` to `Column(modifier = Modifier.padding(16))` with `import /prism/ui`, and checks. `prism run` gains options and no new default behaviour.

## Open Questions

None blocking. The follow-ups go to `ROADMAP.md` at archive:
- `Modifier.clickable`, `fillMaxWidth(fraction)` and ordered modifier chains;
- scopes, intrinsic measurements and `Dp` units;
- composition locals and theming;
- `TextStyle`, font weights and text-field options;
- dirty-region display lists, and several apps per thread;
- a public `NodeId` with a custom-layout API.
