# Design: ui-toolkit

## Context

See `proposal.md` for motivation. This is item 31 of "Then: the road to Milestone 2" in `ROADMAP.md`, the UI layer of `docs/architecture.md` §5. It implements `compose-architecture` D1 (the `prism-ui` crate), D17 (toolkit composables in Prism over one node primitive), D19 (single-pass constraint layout), the display-list half of D20, D21 (headless mode with text dumps) and D22 (handlers owned by nodes), and it replaces D18 (modifiers as an order-insensitive property struct) with Compose's ordered chain. It fills the seams that `compose-runtime` D1 and D9, `compose-transform` D1 and `state-handles` D1 and D12 left for it. Each decision is repeated below where this change refines it. Fast compilation is a primary criterion (user, 2026-10-19), so each decision states its compile-time cost next to its Kotlin-likeness, and its run-time cost where it has one.

**Revision (user direction 2026-10-19).** The user asked that the toolkit follow Jetpack Compose's APIs and its split into modules, and that modifiers never copy the deprecated `Modifier.composed { }`. The decisions this direction settles are marked **(user direction 2026-10-19)**: D5 (Compose's signatures in every composable), D6 (an ordered modifier chain), D7 (`Dp` and `TextUnit`), D21 (one module per Compose artifact), D22 (scoped content lambdas), D23 (handlers in a table of `prism-ui`) and D25 (modifiers are plain data). D3, D9 to D11, D13, D14, D16 and D17 were revised to follow them, and D24 (Material written over foundation) was added. Every other decision a reviewer might take differently is marked **(autonomous)**. All of them are logged in `docs/decisions-for-review.md`.

**Baseline.** This change was proposed while `instance-cache` was being archived. `standard-library`, `compose-runtime`, `compose-transform`, `state-handles` and `hot-reload` were proposed but not implemented. It needs `state-handles` archived, which brings the rest, and the new language change `receiver-lambdas` (D22), which `ROADMAP.md` places before this one. It relies on these names from those packets:

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
  - keepable closures, with the environment equality and duplicate entries of their descriptors (its D9);
  - lent composable content lambdas (its D14);
  - root content adapters for `composition` and `setContent` (its D15);
  - `E4030`, and default values that are not composable contexts (its D17);
  - the runtime module at `/prism/compose/runtime`, in the prelude (its D15, as revised for D21 below).
- **`state-handles`:**
  - `State<T>`, `MutableState<T>` and `.value` (a non-`Copy` value borrowable in composable contexts);
  - `Composition::run_handler(handle, f)`, which makes cells created by handlers belong to their composition (its D12);
  - `SideEffect`.
- **`standard-library`:**
  - `crates/prism-library`, `MODULES` and `PRELUDE`;
  - `private external fun` declarations checked against the runtime's tables (`E6005`);
  - the intrinsic catalog (`crates/prism-typechecker/src/intrinsics.rs`, `E4029`), and `library_tests.rs`.
- **`receiver-lambdas`** (proposed after this revision; its packet decides the exact syntax):
  - function types with a receiver, `compose ColumnScope.() -> Unit`, and lambdas checked against them with an implicit `this`;
  - extensions with a context parameter (Kotlin's `context(scope: ColumnScope) fun Modifier.weight(...)`), callable only where the innermost implicit receiver has that type, and allowed to share a name and receiver when their contexts differ.
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
- **Visibility.** `package` declarations are visible to the files of exactly one package directory (`type-system`, "Cross-Module Imported Symbol and Visibility Validation"). Struct fields have no visibility of their own (a ROADMAP follow-up), so every field of a public struct is readable wherever the struct is.

**Baseline update (`standard-library`, applied 2026-10-18).** Names and mechanisms that differ from what this design assumed:
- Default values on intrinsic parameters are already allowed: the catalog compares declarations without their defaults, and a default is checked and generated in the declaring library file (`standard-library` R8).
- Name lookup has a prelude layer below the imports (`ScopeStack::lookup` and `resolve`: scopes, imports, then the shared prelude names), which companion values must also consult; the toolkit modules are outside the prelude, so they are reached by imports as any module.
- Runtime entry points are looked up through `prism_codegen::externals::entry_point`, which `UI_FUNCTIONS` extends, not `UnitContext::entry_point`; `E6005` is checked by `check_file` over `MirFile::externals`.
- `library_tests.rs` checks, formats and compiles every module through `prism_cli::check_library`, and requires a `//` comment directly above every declaration line.

**Baseline update (`receiver-lambdas`, archived 2026-10-19).** The names and rules that change left, for task 1.1 (main specs `syntax`, `type-system`, `extension-functions`, `references`, `typed-tree`, `code-generation`, `formatting`):
- A context parameter is written `context(scope: ColumnScope)` before every other modifier, and `prism fmt` prints it on its own line directly above the rest of the declaration (`context(scope: ColumnScope)`, then `public fun Modifier.weight(weight: Double, fill: Bool = true): Modifier = ...`), so D22's one-line sketch is reformatted; `library_tests.rs`'s `//` comment goes above the context line. At most one context parameter, and none on an `external` or `intrinsic` declaration (`E2001`), so `weight` and `align` are ordinary Prism functions with bodies that build their elements.
- A call of a function with a context needs a candidate of exactly its type: the innermost receiver lambda's `this`, else the enclosing function's extension receiver and context parameter; otherwise it is `E4034` (`MissingContext`), the code "A scoped modifier outside its scope" expects. Inside the context function the parameter is a `&ColumnScope` and not an implicit receiver.
- Only the innermost receiver is implicit, for every receiver, not only scopes: inside `fun User.Card() { Column { ... } }`, `User`'s fields need `val user = this` before the lambda (`E4001` with a help naming the outer receiver otherwise).
- A content value is called as `content(ColumnScope())`, `scope.content()` or, where the innermost `this` is a `ColumnScope`, `content()`; a receiver function type converts in place to and from `(&ColumnScope) -> Unit`.
- In the typed tree a lambda's receiver is its first parameter, a local of kind `LocalKind::ClosureReceiver` dumped `(closure receiver)`; a context argument leads a call's arguments, before the receiver, dumped `context`. An extension with a context is named `<module>::<receiver>.<name>#context(<type>)` in dumps and, with the ABI's spelling of type constructors, in symbols (`prism/compose/layout::prism/compose/layout.Modifier.weight#context(prism/compose/layout.ColumnScope)` if both live in that module), its default generators appending `#default(<parameter>)`.
- A composable receiver lambda (`compose ColumnScope.() -> Unit`) is checked by `receiver-lambdas` but runs only with `compose-transform`, which sees it as a composable closure whose parameter 0 is the receiver.

**Apply note (groups 4 to 8 and task 9.1 ahead of their prerequisites, 2026-10-19).** The Rust-only work was implemented while `compose-transform` was being applied and before `state-handles`, touching no crate but `prism-ui`, so group 3 is not done yet, and neither are tasks 9.2 and 9.3, which wire `UI_FUNCTIONS` into the backend and the CLI. What that left, for the integration step:
- **The seams of group 3 live in `crates/prism-ui/src/runtime.rs` and `error.rs`**, one function each, so that tasks 3.1 to 3.4 and `state-handles` change those bodies and none of their callers: `call_unit`, `call_with_bool` and `call_with_string` (task 3.3); `is_keepable`, `duplicate` and `environments_equal`, which until `compose-transform` task 7.4 adds the descriptors' equality and duplicate entries treat a value as keepable exactly when its environment needs no drop (it captures nothing or one inline `Copy` word) and compare environment words; `active_composition`, read from `prism_compose::thread`'s active pass (task 3.4); `run_handler`, which calls the handler directly until `Composition::run_handler` exists; `write_output`, through `prism_rt_print_string` over a borrowed string (task 3.2); and `error::raise`, which writes `panic: <message>` and exits with 101 itself until task 3.1 adds the two `PanicKind`s (it does not yet flush the program's output first).
- **Operations return `UiResult`**, as `prism-compose`'s return `ComposeResult`: the tree, the handler table, the setters, the script runner and the frame loop report misuse as `UiError` values, which Rust tests check in process, and only the entry points (task 9.1) and the applier's `create` (which cannot return an error) raise them.
- **The arena is the crate's own** (`handle.rs`), the same generational design as `prism-compose`'s, because releasing every handler at the end of an app walks the live entries, which `prism_compose::Arena` does not offer.
- **Property updates live on `UiTree`** (`set_text`, `set_chain`, ...), comparing and marking dirty; the setters of task 9.1 validate the handle and the pass and then call them, and `test_support::TreeBuilder` calls them directly, so the builder's trees take the same paths as an app's.
- **The modifier mirror** is a byte image read at the offsets of `code-generation`'s enum layout (the table in `modifier.rs`), since Rust's `#[repr(C, u8)]` starts every payload at the union's alignment where Prism starts it right after the tag. It assumes the declarations task 11.1 writes: `Alignment` is `struct Alignment : Copy { val code: Int }`, its codes the nine two-axis values from `TopStart` to `BottomEnd` row by row, then `Start`, `CenterHorizontally`, `End`, `Top`, `CenterVertically` and `Bottom`; `Role` codes 1 to 7 are `Button`, `Checkbox`, `Switch`, `RadioButton`, `Tab`, `Image` and `DropdownList`; `set_arrangement` takes a code (`Top`, `Bottom`, `Start`, `End`, `Center`, `SpaceBetween`, `SpaceAround`, `SpaceEvenly`, `spacedBy` from 0 to 8) and the spacing. A `Size` axis of exactly -1 is not set and other negative sizes lay out as 0; every negative `defaultMinSize` is unspecified.
- **Handler kinds are checked at claim time (autonomous).** A `HandlerId` can be read out of a public `ModifierElement` by a `when`, so a chain built by hand could name a toggle handler in a `Clickable`, which dispatch would call with the wrong signature. Claiming checks the entry's type and ends the program with `ui misuse: a handler of another type was given to a modifier`, a new D17 detail. Rejected: checking at dispatch and calling nothing (a silent failure); trusting the library (memory safety would rest on code programs can bypass).

- **Drawing and dump details the design left open** (group 6): a text field's caret is drawn in the focused colour of its frame (Material's cursor colour is `primary`), or in its text colour without a frame; empty lines and values draw no text run; a node's own content colour counts as its nearest one, so a text node with a content colour uses it; `fontSize` is dumped when the size drawn is not 16, so `MaterialTheme.typography.bodyLarge` dumps nothing; a shape with both a radius and a percentage dumps as `RoundedCorner(<r> <p>%)`.

- **The app and its errors** (group 8): `App` keeps what only the loop uses (host, measurer, dispatcher, layout buffers, display list, counters), while what the setters and the applier reach (the tree, the handler table, the app's composition) is the thread's state; the setters of task 9.1 are Rust functions in `setters.rs` that the entry points will wrap. A composition misuse that `Composition::start` reports (an app started during a pass) is `UiError::Composition`, raised as `composition misuse: <detail>`. Unknown arrangement and alignment codes, which only the toolkit's own code passes, are `ui misuse: unknown arrangement <c>` and `unknown alignment <c>`, like unknown kinds. The first pass of an app is adopted as the app's composition by the first setter it runs, since `Composition::start` returns the handle only after that pass. `type "<text>"` checks the focus before each character, each followed by a pass and a layout; a script's syntax errors are `unknown command '<word>'`, `expected a string or two numbers after '<command>'`, `unterminated string`, `unknown escape '\<c>'`, `an occurrence counts from 1`, `expected an occurrence after the string of '<command>'`, `unexpected '<token>' after '<command>'`, `expected a string after 'type'`, `expected 'backspace' or 'enter' after 'key'`, `expected two numbers after 'resize'`, `a window size cannot be negative`, `expected 'tree' or 'draw' after 'dump'`, `'edit' is reserved for hot reload` and `expected a command`.

**Baseline reconciliation (task 1.1, 2026-10-19, after `state-handles` was archived).** Compared with the code and main specs those changes left:
- `Applier` (`root`, `create`, `place`, `remove`, `release`, `dump`), `Composition::start(Box<dyn Applier>, FunctionValue)`, `Composition::run_handler(handle, f)` and `prism_compose_start_node_group(composer, key, kind) -> u64` exist as named. The active pass's composition is `prism_compose::thread::with_state(|s| s.active.map(|p| p.composition))`, which task 3.4 wraps as `prism_compose::active_composition()`.
- **Descriptors are four words** (`compose-transform` D9, `unit.rs::descriptor`): the slot, the drop entry, the environment equality entry `(environment word, environment word) -> Bool` (null when the closure is not keepable; `prism_rt_empty_environments_equal` when it captures nothing) and the duplicate entry `(environment word) -> environment word` (null when the environment is inline or empty, so a duplicate copies the word). `prism_runtime::Descriptor` still declares two fields; task 3.3 adds `equals_environment` and `duplicate_environment`, and `prism-ui`'s `is_keepable`, `duplicate` and `environments_equal` read them: keepable when the equality entry is not null, a duplicate through the duplicate entry or a copy of the word, and equal when the descriptors are the same and the entry says so. So a closure owning a `String` is not keepable, as D23 says, and one capturing two `Int`s (an allocated environment) is.
- The entry-point lookup is `prism_codegen::externals::entry_point`, over `RUNTIME_FUNCTIONS` and `COMPOSE_FUNCTIONS`; `entry_points()` feeds the JIT. Task 9.2 chains `UI_FUNCTIONS` into both.
- The type checker's catalog is `intrinsics.rs::CATALOG`; intrinsic structs are `prism_typed_tree::IntrinsicStruct`, all of `/prism/compose/runtime`, all `public`. The transform has no `intrinsics.rs`: its intrinsics are lowered in `rewrite.rs` (`Rewriter::intrinsic`), selected by `BuiltinFunction::is_compose_runtime`, and the composition operations that any body may call by `is_composition_operation`. Tasks 10.1 and 10.2 extend those, and `IntrinsicStruct` gains a module and a visibility per struct (`NodeId` is `package`).
- The root content adapter is `Rewriter::adapter_value`, a body owned by `BodyOwner::RootContent`, as named.
- **Intrinsic structs travel by address** today: `representation` gives every struct `Memory`, and `layout.rs` lays an intrinsic struct out as one hidden `Int`. Task 9.2 changes that: `representation` and `machine_type` make every intrinsic struct an `i64` scalar (its layout stays the one-`Int` struct, so it is stored as an `Int` is), the three composer and state operations that wrote a handle through its destination's address write it as a scalar, and equality compares the words, or calls the struct's catalog equality entry (`IntrinsicStruct::equality_entry`, `prism_ui_handlers_equal` for `HandlerId`), inline and in every glue.
- The runtime module is `prism/compose/runtime.prism` (package `prism.compose`, module `/prism/compose/runtime`, in the prelude). `MODULES` lists files of subdirectories, `library_tests.rs` checks that list against the directory tree, and a sibling import in a library file (`import ui`) names a module of its own package directory (`graph.rs`), so the toolkit's files import each other as D21 says.
- Receiver function types are written `compose ColumnScope.() -> Unit`; a context parameter `context(scope: ColumnScope)` on its own line above the declaration; a call without its context is `E4034` (`MissingContext`), the code "A scoped modifier outside its scope" names. A receiver lambda's `this` is a local of kind `LocalKind::ClosureReceiver`.
- A struct may have no fields (`struct ColumnScope : Copy { }` checks and runs). A public struct's field may name a `package` type: nothing checks field types against the struct's visibility. D6 keeps `ModifierElement` and `HandlerId` public anyway, since programs read chains (field visibility stays a ROADMAP follow-up).
- The MODIFIED block of `type-system` matches the main requirement except for its intended change to "Type names are not values". The `standard-library` delta adds requirements to a capability that now exists, and none of their names clashes with the main spec's; it stays ADDED.

**Apply notes (groups 9 to 12, 2026-10-19).** What implementing the compiler side, the modules and the fixtures settled:
- **Intrinsic properties of `Int` (autonomous).** The checker typed intrinsic properties only on `String` and `List<T>`, by name. `n.dp` and `n.sp` are found through the same member table (`members`, filled from the file's declarations and its imports' ABIs), for any intrinsic property of `Int`, and typed as a call of its built-in with the `Int` copied as its receiver; MIR lowers that call of `DpOfInt` or `TextUnitOfInt` to the struct's aggregate. Cost: one member lookup per field access on an `Int`, as for strings. Rejected: special-casing `dp` and `sp` by name, which the next unit property would repeat.
- **Defaults of an intrinsic.** A call of `runApp` that omits `title`, `width` or `height` calls the generators of its declaration in `/prism/compose/ui`, which `BuiltinFunction::declaration_with_defaults` names, since a built-in callee carries no declaration path.
- **Payloads are laid out largest alignment first.** `code-generation` places a variant's 8-byte payloads before its `Bool`s, so `FillMax`, `Clickable` and `Toggleable` are not laid out in declaration order (`Clickable(HandlerId at 8, Bool at 24, Int at 16)`). `modifier.rs` reads `PAYLOAD_OFFSETS`, and `library_tests.rs` compares that table with `prism dump-layout` of the module.
- **Content passed on from a closure (autonomous).** A function value captured by a closure cannot be moved out of it, and passing it to a lent parameter moves it, so `Button` cannot write `Row(..., content = content)` inside its surface's content lambda, as D24's sketch has it. It writes `Row(...) { content() }`, which calls the scope's receiver-lambda content in the row's own lambda. Cost: one composable lambda, a group per pass. Rejected: borrowed function parameters (`&compose RowScope.() -> Unit`) in the public signature, unlike Compose.
- **No `applyModifier` helper.** The composables call `prism_ui_set_modifier` in their `update` functions, as the other setters; a helper would add a call per node per pass for no saving. `prism_ui_handlers_equal` is not declared as an external: the catalog equality entry of `HandlerId` calls it directly.
- **`ColorScheme` and `ButtonDefaultsValues` hold only what the components use** (`primary`, `onPrimary`, `surface`, `onSurface`, `onSurfaceVariant`, `outline`; `shape`, `MinWidth`, `MinHeight`), as D9 lists them.
- **Fixtures.** The fixture runner reads `// events:` and `// headless:` headers, injects library modules from subdirectories (`lib/prism/compose/ui.prism` is `prism/compose/ui`) and dumps a library case's composed MIR (`prism_cli::dump_library_module_composed`). The prelude scenario is a test of `cache_tests.rs`, beside "The composer module is not in the prelude"; "The public signature of runApp" and the mirror check are tests of `library_tests.rs`; "Events without headless mode" is a test of `run_cli_tests.rs`.

## Goals / Non-Goals

**Goals:**
- Jetpack Compose's surface, split as Compose splits it, with its names, parameter order and defaults, for the composables and modifiers a counter and a todo app need, so that Compose code ports with few edits.
- Toolkit composables that are ordinary Prism code: they skip, restart and hot-reload like user composables, and the Rust side is a node tree with property setters.
- Modifiers that keep Compose's ordered semantics, allocate nothing, compare by value and keep typical composables skippable.
- Exact, platform-independent text dumps of the laid-out tree and the display list, driven by scripts, so that executable fixtures pin every behaviour under `--check-allocations`.
- Layout and drawing linear in the nodes, incremental relayout, and steady-state frames that allocate nothing.
- A host interface that `desktop-window` implements without changing the toolkit.

**Non-Goals:**
- Windows, rasterization, fonts and the main-thread split (`desktop-window`); reloads between frames (`compose-hot-reload`); live node counting and frame statistics (`reactive-app-verification`).
- A public custom-layout or custom-modifier API (`Layout`, `MeasurePolicy`, `Modifier.Node`), and the components and text features that the proposal lists as not in scope.

## Decisions

### D1. The boundary with the other compose changes (autonomous)

| Concern | `ui-toolkit` (this change) | Elsewhere |
|---|---|---|
| Node groups, placement, deferred removal | supplies `UiApplier` | `compose-runtime` |
| `Node`, `LeafNode`, `runApp` lowering | catalog entries, transform lowering, backend | the transform's machinery is `compose-transform`'s |
| Handlers and the cells they create | the handler table (D23) and dispatch through `Composition::run_handler` | ownership of cells is `state-handles`' |
| Receiver function types and context parameters | uses them for scopes (D22) | `receiver-lambdas` |
| The runtime module | moves nothing; reads `/prism/compose/runtime` | `compose-transform` and `state-handles` |
| Display list | built and dumped | rasterized in a window by `desktop-window` |
| Text metrics | the `TextMeasurer` interface and the fixed-advance metric | Inter's metrics in windows, if `desktop-window` chooses (D12) |
| Frame loop | events, passes, layout, display list, idling, hosts | the window host and `--screenshot` (`desktop-window`); the reload point (`compose-hot-reload`); `--frame-stats` (`reactive-app-verification`) |
| Leak checking | nodes own and release their values, so the allocation check covers them | counting live nodes (`reactive-app-verification`) |

Rejected: the window in this change (the ROADMAP split it out, process row P10); a separate `ui-layout` change (layout cannot be tested without the tree and the dumps); scopes written around in this change (D22).

### D2. A `prism-ui` crate behind C entry points, with per-thread state (autonomous; implements `compose-architecture` D1)

`crates/prism-ui` depends on `prism-runtime` and `prism-compose`. It holds the node arena and tree, the handler table, layout, the display list, input, the app and frame loop, the host interface, the headless host and its script parser, the dumps, and the `extern "C"` entry points listed in `UI_FUNCTIONS`, a table of `prism_runtime::RuntimeFunction`s with the same source-reading test as `COMPOSE_FUNCTIONS`. `prism-codegen` depends on it only for that table: the entry-point lookup searches `RUNTIME_FUNCTIONS`, then `COMPOSE_FUNCTIONS`, then `UI_FUNCTIONS`, and `Linker::new` registers all three. The crate has no window dependency (§5 D1), and `prism-desktop` will depend on it, not the reverse.

Like `prism-compose` (`compose-runtime` D4), its state is per thread: a `thread_local!` holds the node arena, the handler table and at most one running app. Nothing in the state is borrowed while program code runs, whether drop entries of released strings and handlers or the handlers themselves. A handler's function value is copied out of its entry before it is called, and a released entry's value is taken out before it is dropped.

Run-time cost: one thread-local access per entry point. Compile-time cost: one more table in the entry-point lookup, once per external a unit declares.

Rejected: UI code in `prism-compose` (§5 D1 keeps it UI-agnostic); a global locked state (a lock per setter); one crate with the window (headless tests would build `winit`).

### D3. Toolkit composables in Prism over `Node` and `LeafNode` (autonomous, revised for D21; implements `compose-architecture` D17)

`/prism/compose/ui` (`crates/prism-library/prism/compose/ui.prism`, `package prism.compose`) declares the node layer that the other toolkit modules build on, with `package` visibility, so that it is shared by the toolkit's modules (D21) and invisible to programs:

```
package intrinsic struct NodeId : Copy
package intrinsic compose fun Node(kind: Int, update: (NodeId) -> Unit, content: compose () -> Unit)
package intrinsic compose fun LeafNode(kind: Int, update: (NodeId) -> Unit)
package external fun prism_ui_set_text(node: NodeId, text: &String)
package external fun prism_ui_set_modifier(node: NodeId, modifier: &Modifier)
...
```

Each public composable is a few lines over it, for instance in `/prism/compose/foundation`:

```
public compose fun BasicText(text: &String, modifier: Modifier = Modifier, style: TextStyle = TextStyle.Default) {
    LeafNode(textKind) { node ->
        prism_ui_set_text(node, text)
        prism_ui_set_text_style(node, style.color.argb, style.fontSize.value)
        prism_ui_set_modifier(node, modifier)
    }
}
```

- **Lowering.** `Node(kind, update, content)` lowers in the transform to `node = start_node_group($composer, key, kind)`, a call of `update(node)`, a call of `content($composer, 0)`, and `end_group($composer)`. Here `key` is `H(parent, "node", <ordinal among the Node and LeafNode calls of the parent region>)`, chained as `compose-transform` D5 chains region keys. `LeafNode` is the same without `content`. Two intrinsics replace Compose's two `ComposeNode` overloads, since Prism has no overloading, and a leaf passes no empty content lambda, which would cost a group per leaf per pass.
- **`update` runs on every pass of its group**, as Compose's `update` block does. Each setter compares the new value with the node's and marks the node dirty only when it differs (D11), so an unchanged recomposition costs a few calls and compares. `update` is lent (not `escaping`), so it borrows the composable's parameters (`text: &String`, `modifier`) and allocates nothing.
- **Kinds** are `package` `Int` constants of `/prism/compose/ui`: column 1, row 2, box 3, text 4, spacer 5, text field 6. The window root, kind 0, is not a composable. Material components have no kinds of their own: they are built from these nodes (D24). The runtime rejects an unknown kind with `ui misuse: unknown node kind <k>`.
- **`NodeId`** is the state-handle invariant's node handle: an intrinsic struct of one word, `Copy`, compared by its word (`compose-transform` D16). It is `package` because no public API hands one out yet. A custom-layout API would make it public.
- **Setters** are split by what they invalidate. `set_text`, `set_text_style`, `set_arrangement` and `set_alignment` invalidate layout. `set_content_color` and `set_enabled` invalidate drawing only. `set_modifier` compares the chain element by element and invalidates layout when a layout element changed, drawing when a drawing element changed, and nothing else (D6, D11). `set_on_value_change` gives a text field its handler entry (D23) and invalidates nothing.
- **Externals are `package`.** `standard-library` forbids `public` externals; `package` ones are reachable from every toolkit module of the package and from no program.

Compile-time cost: four modules (D21) of a few hundred lines in all, parsed and checked only by programs that import them, and cached under the compiler version (`standard-library` D3). Each `Node` call is constant work in the transform.

Rejected:
- Composables implemented in Rust (§5 D17): no hot reload, no skipping, and the type checker would need tables of their signatures.
- A public node layer (`Node`, kinds, setters) so that modules of other packages could use it: it would commit programs to a stopgap API that a Compose-shaped `Layout` and `Modifier.Node` API should replace (D21).
- One setter taking every property: every change would invalidate layout.
- An enum of kinds: the runtime receives a word, and a `when` per call would cost more than a constant.

### D4. Companion values: a struct and a top-level `val` may share a name (autonomous)

A top-level `val` may have the name of a struct declared in the same file. In a type position the name means the struct, and a call `Name(...)` is the struct's constructor. Anywhere else in a value position, `Name.field` and `Name.f()` included, the name means the value. Every other pair of same-named top-level declarations is still `E4010`: two `val`s, a `val` and an enum, function or extension, or a `val` and a struct of another file (an import included). A struct without a companion value is still not a value ("Type names are not values"). The ABI exports both symbols under the name, and an import brings both.

This is how the toolkit gets Compose's exact spellings:
- `public val Modifier: Modifier` is the empty modifier, so `Modifier.padding(16.dp).fillMaxWidth()` and `modifier: Modifier = Modifier` read as in Compose.
- `public val Color: Colors` holds Compose's named colours as fields, so `Color.Red` and `Color(0xFF336699)` both work.
- `public val Arrangement: Arrangements` gives `Arrangement.Top` and, through `fun Arrangements.spacedBy(space: Dp): Arrangement`, `Arrangement.spacedBy(8.dp)`.
- `Dp.Unspecified`, `TextUnit.Unspecified` and `TextStyle.Default` are fields of the companions of `Dp`, `TextUnit` and `TextStyle`.

Values with a capitalized name and no struct of that name, such as `MaterialTheme` and `ButtonDefaults` (D9), are ordinary top-level values and need no rule.

Kotlin reaches the same spellings with companion objects. Here a companion is just a value of a struct type, so no new kind of declaration, no static members and no new lookup rule beyond the name split are needed. User code can use the same pattern for its own themes and constants.

Compile-time cost: one filtered lookup when a name in a value position finds a struct (the scope already filters types for `lookup_type`), and one kind comparison in the duplicate check. Nothing is inferred.

**Apply notes (group 2).**
- The pair is defined by `ScopeStack::define_companion`, which the checker calls for a top-level `val` (structs are collected before values, so the struct is always there first); the value is the name's innermost binding, so `lookup` and `resolve` find it unchanged, `lookup_type` skips it, and `lookup_constructor` finds the struct for a call's callee and a struct pattern. A payload-free enum variant is never a companion (it is defined through `define`), so a struct and a variant of one name stay `E4010`.
- An import keeps both symbols of a module under the name, and counts that module once, as its value.
- **A `val` and an extension of one name coexist (autonomous).** The proposal listed extensions among the pairs that stay `E4010`, but an extension is a member of its receiver type, found by `recv.name`, never by a bare name, and the checker always accepted `val Mode` beside `fun Int.Mode()`, as Kotlin does. Rejected: a new `E4010` for that pair, which would break programs for no ambiguity.
- The prelude layer keeps one symbol per name, so a prelude module cannot give a struct a companion value; `library_tests.rs` now checks that no prelude module exports a name twice. The toolkit's companions are in non-prelude modules.
- The ABI already exports both symbols, sorted by kind before name, so the order of the two declarations does not change the hash.

Rejected:
- `Modifier()` through default values on struct fields: a larger language feature (field defaults in the ABI, generators per field, constructor matching), and still not Compose's spelling.
- Enum colours (`Color.Red`, `Color.Argb(...)`): two representations of one colour, which would compare unequal.
- Lower-case values (`emptyModifier`) or names unlike Compose.
- Kotlin's `companion object` or static members: a new declaration form and member namespace for what a value already does.
- Allowing the pair across files: an import would silently change what a local struct name means.

### D5. The public API: Compose's names, parameters and defaults (user direction 2026-10-19)

Every composable and modifier function has Compose's name, its parameters in Compose's order with Compose's names and defaults, and leaves out the parameters listed below; so a call written for Compose that uses only these parameters compiles here, with `1.0` for `1f`. The module of each is D21's.

| Declaration | Signature (as in Compose, minus the parameters left out) |
|---|---|
| `Box` (layout) | `(modifier: Modifier = Modifier, contentAlignment: Alignment = Alignment.TopStart, content: compose BoxScope.() -> Unit)` |
| `Column` (layout) | `(modifier = Modifier, verticalArrangement: Arrangement = Arrangement.Top, horizontalAlignment: Alignment = Alignment.Start, content: compose ColumnScope.() -> Unit)` |
| `Row` (layout) | `(modifier = Modifier, horizontalArrangement = Arrangement.Start, verticalAlignment = Alignment.Top, content: compose RowScope.() -> Unit)` |
| `Spacer` (layout) | `(modifier: Modifier)` |
| `BasicText` (foundation) | `(text: &String, modifier = Modifier, style: TextStyle = TextStyle.Default)` |
| `BasicTextField` (foundation) | `(value: &String, onValueChange: escaping (String) -> Unit, modifier = Modifier, enabled: Bool = true, textStyle: TextStyle = TextStyle.Default)` |
| `Text` (material3) | `(text: &String, modifier = Modifier, color: Color = Color.Unspecified, fontSize: TextUnit = TextUnit.Unspecified, style: TextStyle = MaterialTheme.typography.bodyLarge)` |
| `Button` (material3) | `(onClick: escaping () -> Unit, modifier = Modifier, enabled: Bool = true, shape: Shape = ButtonDefaults.shape, colors: ButtonColors = ButtonDefaults.buttonColors(), content: compose RowScope.() -> Unit)` |
| `Checkbox` (material3) | `(checked: Bool, onCheckedChange: escaping (Bool) -> Unit, modifier = Modifier, enabled: Bool = true)` |
| `TextField` (material3) | `(value: &String, onValueChange: escaping (String) -> Unit, modifier = Modifier, enabled: Bool = true, textStyle: TextStyle = MaterialTheme.typography.bodyLarge)` |
| `Surface` (material3) | `(modifier = Modifier, shape: Shape = RectangleShape, color: Color = MaterialTheme.colorScheme.surface, contentColor: Color = MaterialTheme.colorScheme.onSurface, content: compose () -> Unit)` |
| `runApp` (ui) | `(title: &String = "Prism", width: Int = 800, height: Int = 600, content: escaping compose () -> Unit)` |
| layout modifiers | `padding(all, horizontal, vertical, start, top, end, bottom)` (D6), `width(width: Dp)`, `height(height: Dp)`, `size(width: Dp, height: Dp = width)`, `defaultMinSize(minWidth: Dp = Dp.Unspecified, minHeight: Dp = Dp.Unspecified)`, `fillMaxWidth(fraction: Double = 1.0)`, `fillMaxHeight(fraction)`, `fillMaxSize(fraction)`; in scopes (D22): `weight(weight: Double, fill: Bool = true)` (rows and columns), `align(alignment: Alignment)` (boxes, rows and columns) |
| foundation modifiers | `background(color: Color, shape: Shape = RectangleShape)`, `border(width: Dp, color: Color, shape: Shape = RectangleShape)`, `clickable(enabled: Bool = true, role: Role? = null, onClick: escaping () -> Unit)`, `toggleable(value: Bool, enabled: Bool = true, role: Role? = null, onValueChange: escaping (Bool) -> Unit)` |
| ui | `Modifier.then(other: Modifier): Modifier` |

Every deviation from Compose that remains, with its reason:
- **Text is `&String`** in `Text`, `BasicText`, `TextField` and `BasicTextField` (D8): the contents of a `State<String>` pass without `.clone()`.
- **No overloading in Prism.** Compose's three `padding` overloads are one signature through defaults that read earlier parameters (D6); `size(size)` and `size(width, height)` are `size(width, height = width)`, so `size(size = 10.dp)` must be written `size(10.dp)`; `RoundedCornerShape(size: Dp)` and `RoundedCornerShape(percent: Int)` are `RoundedCornerShape(size: Dp = 0.dp, percent: Int = 0)`, so the percent form is written `RoundedCornerShape(percent = 50)`; `Surface` has no clickable overload (clickable surfaces use `Modifier.clickable`); and there is no `Modifier.padding(PaddingValues)`, so `Button` has no `contentPadding`.
- **No nested types.** Compose's `Alignment.Horizontal` and `Alignment.Vertical`, and `Arrangement.Horizontal` and `Arrangement.Vertical`, are one `Alignment` and one `Arrangement` type. A column reads the horizontal part of its alignment and a row the vertical one, so `Column(horizontalAlignment = Alignment.Top)` compiles and means `Start`, where Compose rejects it.
- **No composition locals.** `Text`'s default style is `MaterialTheme.typography.bodyLarge` instead of `LocalTextStyle.current`, so a `Button`'s label is 16 and not `labelLarge`'s 14; `LocalContentColor` is replaced by the content colour of D9; `MaterialTheme` is a value, without the `MaterialTheme(colorScheme, typography) { }` composable that would provide another theme; `Surface`'s `contentColor` defaults to `onSurface`, not `contentColorFor(color)`.
- **Units are whole logical pixels** (D7): `Dp` and `TextUnit` hold `Int`s, `16.5.dp` does not exist, and `Dp` has no arithmetic until operators can be declared.
- **`Box`'s content is required**, because a composable lambda default would be built in a generator for nothing, and `Box` has no `propagateMinConstraints`.
- **`onCheckedChange` is not nullable**, and `Checkbox` draws no checkmark, has no `colors`, and no tri-state form.
- **Left out for later changes:** `Button`'s `elevation`, `border`, `contentPadding` and `interactionSource`; `Text`'s `fontWeight`, `fontStyle`, `fontFamily`, `letterSpacing`, `textAlign`, `lineHeight`, `overflow`, `softWrap`, `maxLines`, `minLines` and `onTextLayout`; `TextField`'s `readOnly`, `label`, `placeholder`, `singleLine`, `keyboardOptions`, `keyboardActions` and `colors` (it is always single-line); `clickable`'s `onClickLabel`, `interactionSource` and `indication`; `Surface`'s `tonalElevation`, `shadowElevation` and `border`; `BoxScope.matchParentSize`.
- **`TextField` has an outlined look** (D9): Material 3's filled field draws an indicator line along one side, which the display list's borders do not draw yet.
- **Modifier chains hold at most 16 elements** (D6), and `clickable` and `toggleable` work only while an app runs (D23).
- **`runApp(title, width, height, content)`** stands for Compose Desktop's `application { Window(...) { } }` and `singleWindowApplication`, since one window per thread is all this change runs (D15).
- **Imports are per module**, one per Compose artifact (D21), not per declaration.

`Checkbox` and `TextField` are in scope, as §5 D17 and the ROADMAP item say, because the todo app of `reactive-app-verification` needs them.

`runApp` is an intrinsic, not a Prism function over one, so that `E4030` can name it and so that its content gets a root content adapter where it is called (D15). Its defaults are ordinary default values, which `standard-library` allows on intrinsics: a default is a generator in the declaring file, independent of how the call is lowered.

Compile-time cost: none beyond the modules (D3). Kotlin-likeness: Compose's signatures, so most Compose screens port by editing imports, `16.dp`-style units stay as they are, and `1f` becomes `1.0`.

Rejected: §5's first sketch of per-composable layout parameters (`Column(spacing = 8)`), since Compose spells them `verticalArrangement = Arrangement.spacedBy(8.dp)`; this change's first API, Compose's names with `Int` sizes and no scopes (superseded by this direction); a smaller set without `Checkbox` and `TextField`.

### D6. `Modifier` is an ordered chain of up to 16 elements in a `Copy` struct (user direction 2026-10-19; replaces `compose-architecture` D18)

A modifier is Compose's ordered chain: `Modifier.padding(8.dp).background(Color.Red)` pads first and draws the background inside the padding, `Modifier.background(Color.Red).padding(8.dp)` draws it under the padding too, and the two are not equal. It is a `Copy` value in `/prism/compose/ui`:

```
public enum ModifierElement : Copy {
    None,
    Padding(Int, Int, Int, Int),          // start, top, end, bottom
    Size(Int, Int),                        // width, height; -1: that axis is not set
    DefaultMinSize(Int, Int),              // minimum width, height; -1: unspecified
    FillMax(Bool, Bool, Double),           // width, height, fraction
    Weight(Double, Bool),                  // weight, fill
    Align(Alignment),
    Background(Color, Shape),
    Border(Int, Color, Shape),
    Clickable(HandlerId, Bool, Int),       // handler, enabled, role code (0: none)
    Toggleable(HandlerId, Bool, Bool, Int), // handler, value, enabled, role code
    TextFieldFrame(Color, Color),          // unfocused and focused frame colours
}
public struct Modifier : Copy { val count: Int, val first: ModifierElements, val second: ModifierElements }
public struct ModifierElements : Copy { val e0: ModifierElement, ..., val e7: ModifierElement }
public val Modifier: Modifier = Modifier(0, emptyElements, emptyElements)
public fun Modifier.then(other: Modifier): Modifier
```

- **Building.** Each modifier function is an ordinary, non-composable extension `fun Modifier.f(...): Modifier` that returns a copy with one element appended through the `package` helper `Modifier.append(element)`, which writes slot `count` of the two blocks of eight. `then` appends the other chain's elements in order, as Compose's `then` does.
- **Capacity.** Sixteen elements. Compose screens rarely chain more than five, and the Material components append at most four of their own to the caller's chain (`Checkbox`: `toggleable`, `size`, `background`, `border`). Appending a seventeenth ends the program with `ui misuse: a modifier chain holds more than 16 elements` (D17), through a `package` external that never returns. No limit can be checked at compile time, since chains are values built across calls and files, and a spill to the heap would make `Modifier` move-only.
- **Element payloads are whole pixels, colours and codes**, so the chain holds no `Dp` wrappers, and the `role` is stored as its code.
- **Equality** is the struct's equality glue, element by element: two chains are equal when they hold equal elements in the same order. A handler element compares its `HandlerId`s by the handlers they hold (D23), so `Modifier.clickable { count.value += 1 }` built on two passes is equal to itself.
- **Stability.** `Modifier` is `Copy` and holds no borrow, so a `Modifier` parameter is stable (`compose-transform` D7): a skippable composable keeps it in a slot and skips when the caller passes an equal chain. Since handlers live in the handler table and the element holds only their `HandlerId` (D23), a chain with `clickable` is as stable as one without.
- **Applying.** A node's `update` passes its modifier to `set_modifier` by address. `prism-ui` reads it through `#[repr(C)]` mirrors of the two types' layouts (`code-generation`, "Struct Layouts" and "Enum Layouts"), which a Rust test checks against `prism dump-layout` of the module, and stores the decoded chain in the node (D10).
- **Order semantics** are D11's (layout), D13's (drawing) and D14's (input): each element applies to the layout of the elements after it.
- **Padding** is `padding(all: Dp = 0.dp, horizontal: Dp = all, vertical: Dp = all, start: Dp = horizontal, top: Dp = vertical, end: Dp = horizontal, bottom: Dp = vertical)`, Compose's three overloads in one signature through defaults that read earlier parameters, so `padding(16.dp)`, `padding(horizontal = 8.dp)` and `padding(start = 4.dp)` all mean what they mean in Compose. Chained paddings add up because each pads what the next one lays out, as in Compose. Negative paddings and sizes are laid out as 0.
- **Shapes.** `Shape` is `struct Shape : Copy { val cornerRadius: Int, val cornerPercent: Int }`, with `RectangleShape` in ui and `CircleShape` (50 percent) and `RoundedCornerShape(size, percent)` in foundation. The radius drawn is the larger of the two, the percentage taken of the smaller side of the bounds the element draws in.

A modifier is about 650 bytes: two blocks of eight elements of five words, and the count. Passing one costs a copy, and setting it on a node one entry call that compares the chain.

Compile-time cost: none beyond the module: the appending helper's two `when`s of eight cases are library code, checked once per compiler version. Run-time cost: each chain call copies the struct, with no allocation; a handler element also costs one entry call (D23).

Rejected:
- This change's first design, an order-insensitive property struct of 16 words (`compose-architecture` D18): smaller, but `padding(8).background(c)` and `background(c).padding(8)` were equal, which no Compose user expects (user direction).
- An owned list of elements: an allocation per chain, a modifier moved by its first use, no companion value `Modifier` (a top-level value cannot be moved out of), and parameters that are not `Copy` and so never stable.
- Interned chains behind a one-word handle into `prism-ui`: unbounded chains and word compares, but chains built from changing values (an animated width) would pile up until the app ends, since nothing owns an interned chain short of reference counting (the zero-cost memory safety invariant forbids hidden counting).
- A capacity of 8 (a `Checkbox` would leave its caller four elements) or 32 (every copy and kept slot doubles for chains Compose code does not write).
- Owned handler closures inside elements, as Compose's `ClickableElement(onClick)` holds its lambda: a struct holding an owned closure cannot be `Copy` (D23).
- Fixed-size arrays for the element blocks: a language feature of its own; the two blocks can become one array behind the same API when arrays exist.

### D7. Sizes are `Dp`, font sizes `TextUnit`, both in whole logical pixels (user direction 2026-10-19; replaces this change's first D7)

Compose writes `16.dp` and `14.sp`, and so does Prism:

```
public struct Dp : Copy { val value: Int }
public val Dp: Dps                         // Dp.Unspecified
public intrinsic val Int.dp: Dp
public struct TextUnit : Copy { val value: Int }
public val TextUnit: TextUnits             // TextUnit.Unspecified
public intrinsic val Int.sp: TextUnit
```

`Int.dp` and `Int.sp` are intrinsic extension properties of `/prism/compose/ui`, since Prism has no extension properties outside the library (a `standard-library` follow-up). Each is lowered in MIR as the construction `Dp(n)` or `TextUnit(n)`, so its cost is a constructor's. They are members of `Int` wherever the module is visible ("Intrinsic Extensions Are Members"), so `(n * 8).dp` works and a workspace `fun Int.dp()` beside an import of the module is `E4010`. A `Dp` is one logical pixel, scaled by `desktop-window` to the window's scale factor, and a `TextUnit` one logical pixel of font size, as §5 fixed so that dumps are exact. `Dp.Unspecified` and `TextUnit.Unspecified` hold `-1`, and any negative value is unspecified where a parameter allows it (`defaultMinSize`, `fontSize`); elsewhere negative sizes lay out as 0.

The types catch what `Int` sizes let through: a font size passed as a padding, or a count passed as a width. A typed size costs nothing at run time beyond a one-field struct, passed as aggregates are.

Compile-time cost: two catalog entries, and a constant lowering per use. Kotlin-likeness: Compose's spelling, without fractional units or unit arithmetic (D5).

Rejected:
- `Int` logical pixels (this change's first D7): `padding(16)` reads unlike Compose and mixes sizes with font sizes and counts.
- `Double` units: layout is integer so that dumps are exact (D11), and a fraction of a logical pixel would be rounded away anyway.
- `fun Int.dp(): Dp`, written `16.dp()`: unlike Compose for no saving, since the catalog entry is constant work.

### D8. Text parameters are borrowed (autonomous; answers the `state-handles` risk)

`Text`, `BasicText`, `TextField` and `BasicTextField` take `&String`. A literal, a template temporary or a place is then borrowed at the argument site (`references`, "Implicit Shared Borrow at Argument Sites"). So `Text("+")`, `Text("Count: ${c.value}")` and `Text(name.value)` for a `State<String>` all work without `.clone()`, where an owned `String` parameter would make `Text(name.value)` `E4017`. `prism_ui_set_text` compares the bytes and copies them into the node's own string only when they differ, so an unchanged text costs one comparison and no allocation. A template still allocates its result, as it must. A `&String` parameter makes `Text` not skippable (`compose-transform` D7), and it would not be skippable with a `String` either.

Rejected: an owned `String` (`E4017` for state contents, and a move into the node that still needs a compare); both through two names.

### D9. Colours, the Material baseline, and content colour inherited at draw time (autonomous, revised for D5 and D24)

`public struct Color : Copy { val argb: Int }` in `/prism/compose/ui` holds an ARGB value in the low 32 bits. Its companion `Color: Colors` (D4) holds Compose's `Black`, `DarkGray`, `Gray`, `LightGray`, `White`, `Red`, `Green`, `Blue`, `Yellow`, `Cyan`, `Magenta`, `Transparent` and `Unspecified`, with Compose's values. `Unspecified` is `Color(0x1_0000_0000)`, outside the ARGB range, and any colour outside that range is unspecified.

- **Content colour.** Compose resolves a `Text`'s default colour through `LocalContentColor`, a composition local. Prism has none. Instead, every node has a content colour, and the display list resolves an unspecified text colour to the nearest ancestor's content colour in its top-down walk, or to black at the root. `Surface`, and so `Button`, sets its node's content colour through a `package` setter. This is O(1) per node in a walk that already happens, and it gives `Button { Text("+") }` a readable label.
- **Text style.** `TextStyle` (ui) is `struct TextStyle : Copy { val color: Color, val fontSize: TextUnit }`, with `TextStyle.Default` holding both unspecified. Material's `Text` takes `color` when it is specified, else the style's colour, and the same for `fontSize`, and passes the result to `BasicText`. An unspecified font size is 16 (D12).
- **The theme.** `/prism/compose/material3` declares `public val MaterialTheme: MaterialThemeValues`, whose `colorScheme: ColorScheme` holds Material 3's baseline light colours (`primary` `#FF6750A4`, `onPrimary` `#FFFFFFFF`, `surface` `#FFFEF7FF`, `onSurface` `#FF1D1B20`, `onSurfaceVariant` `#FF49454F`, `outline` `#FF79747E`), and whose `typography: Typography` holds `bodyLarge` (16) and `labelLarge` (14). `MaterialTheme.colorScheme.primary` reads as in Compose. Providing another theme waits for composition locals.
- **`ButtonDefaults`** is a value holding `shape` (`CircleShape`), `MinWidth` (58.dp) and `MinHeight` (40.dp), with `fun ButtonDefaultsValues.buttonColors(containerColor = primary, contentColor = onPrimary, disabledContainerColor = #1F1D1B20, disabledContentColor = #611D1B20): ButtonColors`, the disabled colours being `onSurface` at Compose's 12 and 38 percent alpha.

The components' look, from those values (D24 says how they are built):
- **`Button`:** container `primary` in `ButtonDefaults.shape`, content colour `onPrimary`; disabled `#1F1D1B20` and `#611D1B20`; content padding 24 horizontally and 8 vertically, at least 58 × 40.
- **`Checkbox`:** 20 × 20, corner radius 2; unchecked, a border of 2 in `onSurfaceVariant`; checked, filled and bordered in `primary`; disabled, `#611D1B20`.
- **`TextField`:** at least 200 wide, inner padding 8; a frame of 1 in `outline`, of 2 in `primary` when focused, of 1 in `#1F1D1B20` when disabled; a 1-pixel caret at the end of the text when focused.
- **`Surface`:** its `color` in its `shape`, and its `contentColor`.
- **The window:** cleared to white.

Rejected:
- Composition locals now: a compose-runtime feature of its own.
- A black default text colour everywhere: unreadable on a button.
- Colours computed with `copy(alpha = ...)`: a Prism `Color` has no float arithmetic, so the derived values are constants with their derivation in a comment.

### D10. The node tree: a generational arena, linked children, owned values (autonomous, revised for D6 and D23)

- **Records.** Each node record holds:
  - its kind, creation number, tree, parent, first and last child, previous and next sibling;
  - its properties: text, text style, arrangement, alignment, enabled, content colour;
  - its modifier chain, decoded (D6), with the `HandlerId` of each handler element it has claimed (D23), and a text field's value-change handler;
  - its layout state: the last constraints, size and position relative to its parent, the bounds at each element of its chain (D11), and its dirty flags.
- **Handles.** A `NodeId` is a 32-bit index and a 32-bit generation (`handle.rs`, as in `prism-compose`). Releasing a node bumps its generation, and an index whose generation reaches the maximum is retired.
- **Placement.** `UiApplier` implements `compose-runtime`'s `Applier`:
  - `create(kind)` allocates a node;
  - `place(parent, previous, node)` links it after `previous`, and does nothing when it is already there;
  - `remove` unlinks it at the end of the pass and marks the parent's layout dirty;
  - `release` takes its strings out and gives up its handler entries, dropping both after the arena is let go, children first, as `compose-runtime` D7 orders releases.
  Linked siblings make placement after a sibling O(1) and keep traversal in order. Creation numbers count from 1 per app and are never reused, so dumps show identity: a moved node keeps its number, and a recreated one gets a new one.
- **Validation.** A setter validates the `NodeId`'s generation, and also that the thread's active pass belongs to the running app's composition. That rejects a toolkit composable composed in a plain `composition { }`, whose node tree applier made the handle, and a setter called through a remembered `NodeId` outside composition (D17).
- **Ownership.** A node owns its text (a `PrismString` from the runtime's allocator, so `--check-allocations` tracks it) and the handler entries it claimed from the handler table (D23), so handlers are still owned by the nodes that use them (`compose-architecture` D22), held in the table so that a `Copy` modifier can name them.

Rejected:
- Children as vectors: placement after a sibling would be O(children).
- Reusing compose's node tree applier with side tables: two arenas to keep in step.

### D11. Constraint layout over modifier chains, measure policies and the cache (autonomous, revised for D6; implements `compose-architecture` D19)

Constraints are minimum and maximum widths and heights in `i64` logical pixels, and a maximum may be unbounded. Measuring a node under constraints `c` walks its modifier chain from the first element, Compose's outermost, to the last, each layout element changing the constraints it passes on and the size it gets back:

| Element | Effect |
|---|---|
| `padding(s, t, e, b)` | measures the rest with `c` deflated by `s + e` and `t + b`, floored at 0; returns the result plus the padding, clamped into `c`; offsets the rest by `(s, t)` |
| `size(w, h)`, `width`, `height` | measures the rest with an exact size on each set axis, clamped into `c` |
| `defaultMinSize(w, h)` | on each axis whose minimum in `c` is 0 and whose value is specified, raises the minimum to that value, at most the maximum |
| `fillMaxWidth(f)`, `fillMaxHeight(f)`, `fillMaxSize(f)` | on each axis with a bounded maximum, an exact size of the maximum times `f`, rounded down and clamped into `c` |
| `weight`, `align` | none here: the parent reads them (parent data); with several, the first in the chain counts, as in Compose |
| `background`, `border`, `clickable`, `toggleable`, `TextFieldFrame` | none: each records the bounds at its position, the offset of the elements before it and the size the elements after it returned, for drawing and input |

After the last element, the kind's policy measures the content under what remains:

| Kind | Policy |
|---|---|
| window (root) | constraints 0..width × 0..height; children like a `Box` aligned `TopStart` |
| `text` | lines from word wrapping at the maximum width (D12); width of the widest line, height of the lines times the line height, at least one line, clamped into the constraints |
| `spacer` | 0 × 0 clamped into the constraints, so its size comes from its modifier |
| `box` | children measured with the constraints loosened to minimum 0; size the largest child, at least the minimum; each child placed by its `align`, else by `contentAlignment` |
| `column` | children without a weight measured in order, with width 0..maximum and height 0..what remains; then weighted children share what remains of a bounded maximum by weight (each `remaining * w / total` rounded down, the last taking the rest), as exact heights with `fill` and as maximums without it; width the widest child, height the sum plus the gaps; children arranged on the main axis and aligned by their `align`, else by the horizontal part of `horizontalAlignment` |
| `row` | the same, along the other axis, with vertical parts |
| `textfield` | one unwrapped line of its value, clamped into the constraints |

Arrangements with free space `f` and `n` children:
- `Top` and `Start` place the children from 0, and `Bottom` and `End` from `f`.
- `Center` places them from `f / 2`.
- `SpaceBetween` puts `f / (n - 1)` between them, and places a single child first.
- `SpaceAround` puts `f / n` between them, with half that before the first.
- `SpaceEvenly` puts `f / (n + 1)` before each one.
- `spacedBy(s)` puts `s` between them and places them from the start.

Remainders of the integer divisions are dropped, so positions are exact integers on every platform. An alignment has a horizontal and a vertical part (`TopStart` is start and top, `CenterHorizontally` is centre and top), and a child is placed at 0, `(free) / 2` or `free` on each axis.

- **The cache.** Every setter that changes a layout property or a layout element marks its node `needs_measure` and its ancestors `child_needs_measure`, stopping at the first ancestor already marked. Layout starts at the root. A node that is neither, and is measured with its last constraints, returns its last size without visiting its subtree. Positions are relative to the parent, so an unchanged subtree needs no placement. Absolute bounds are computed on the walks that need them: hit testing, the display list and dumps.
- **Relayout climbs from the dirty nodes (autonomous, apply).** The tree also records each node it marks. When the root is clean and its constraints unchanged, layout re-measures each recorded node with its last constraints, which stay its own while its siblings and ancestors keep their sizes, and climbs to the parent, measured again with its policy, only when the size changed, stopping at the first node whose size stays; a node never measured is measured by its parent's policy. A node that is only `child_needs_measure`, reached from above, re-measures its dirty children with their last constraints and keeps its own size and its children's positions when they keep theirs. A change of a node's `weight` or `align` marks its parent, which reads them. So a text change that keeps the text's size (a counter's) costs O(depth), and one that changes it O(the children of each ancestor whose size changes), as Compose's relayout does. A randomized Rust test edits texts, chains, arrangements, structure and the window size and compares every incremental layout with a full one. Rejected: always running each dirty ancestor's policy from the root, the first implementation, whose relayout of one text of the 10,000-node benchmark tree took a quarter of a full layout, since the column's policy visits its 3,333 rows; a scan of each ancestor's children for dirty flags, which still took a ninth.
- **Cost.** A full layout is O(nodes + elements). A relayout is O(the dirty nodes, their ancestors, and the children of each re-measured node), each clean child costing one comparison. Text measurement is O(characters) for a node whose text or constraints changed.

Rejected:
- A flexbox engine such as `taffy` (§5 D19).
- Floating-point layout: dumps would depend on rounding.
- Re-measuring every node every frame.
- Compose's intrinsic measurements (§5 D19, a follow-up).
- Layout policies written in Prism (Compose's `MeasurePolicy`): a call into program code per node per layout, against D2's rule that no Rust reference is live while program code runs; a follow-up with a public `Layout`.

### D12. Text measurement through an interface; a fixed-advance metric headless (autonomous; revises `compose-architecture` D19)

Layout measures text through `trait TextMeasurer { fn advance(&self, character: char, font_size: i64) -> i64; fn line_height(&self, font_size: i64) -> i64; }`. Word wrapping is the toolkit's own, the same for every measurer. Each `\n` starts a line. Lines break greedily at spaces, and a space at a break is not drawn. A word wider than the line stays whole and overflows.

This change ships `FixedAdvanceMetrics`. Every character advances `font_size / 2`, and the line height is `font_size * 5 / 4`, so 16-pixel text is 8 pixels per character and 20 pixels high. Headless runs, and therefore every fixture, use it. §5 D19 planned the bundled font's metrics in every renderer, so that headless layouts equal windowed ones. That would make each dump depend on the font file, its version and kerning, and on a font that only `desktop-window` brings, and dumps would stop being readable arithmetic. The interface leaves `desktop-window` the choice: lay out windows with Inter's metrics through `fontdue`, so a window looks right but its layout differs from the headless one, or with the fixed metric and draw Inter glyphs in its boxes. Its design records the choice. The app takes its measurer when it starts, so one run uses one measurer throughout.

Rejected:
- Inter's metrics headless (above).
- A metrics table generated from Inter now: it needs the font that `desktop-window` adds, and it ties fixtures to it.
- Proportional test metrics: harder to compute in a scenario.

### D13. The display list (autonomous, revised for D6; implements the display-list half of `compose-architecture` D20)

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
- for each node, its drawing elements in chain order, each in the bounds recorded at its position (D11): `background` a rectangle, `border` a border, `TextFieldFrame` a border of 1 in its first colour, or of 2 in its second when the node is focused;
- then its kind's own commands: one text command per line for a text, the value and, when focused, the caret for a text field, with content colours resolved (D9);
- then its children.

Drawing the chain in order is Compose's: the first element draws outermost and first, so a later background covers an earlier border. The vectors are cleared and reused, so once they have grown, a rebuild allocates nothing. Rebuilding the whole list is O(nodes + elements) and happens at most once per frame. A partial rebuild by dirty region is a follow-up, if frame times ask for it. `desktop-window` rasterizes the list. The headless host prints it on `dump draw` (D16). It is also the place a GPU backend would start from (§5 D20).

Rejected:
- Drawing directly from the tree in the renderer: the renderer would need the toolkit's styling rules.
- An ASCII-art rasterizer for snapshots: a character grid loses exact positions and sizes, which the text dump of the list keeps.

### D14. Input: hit testing elements, clicks on release, focus, and a pass after every event (autonomous, revised for D6 and D24; implements `compose-architecture` D22, revises the batching of §5 "Input Events Dispatch to Node Handlers")

- **Hit testing** walks the laid-out tree in reverse drawing order: last child first, children before their parent, and within a node its input elements from the last to the first. It returns the first input target whose bounds contain the point: a `clickable` or `toggleable` element, in the bounds recorded at its position (D11), or a text field node, in its bounds; enabled or not.
- **Clicks.** A press records the target. A release calls it when the release hits the same target and the target is enabled: a `clickable`'s `onClick()`, a `toggleable`'s `onValueChange(!value)`, or focusing a text field. A disabled target absorbs the press and the release and calls nothing, so a click never falls through to what is under it. A press that hits no text field clears the focus. The pressed target and the focus are state of `prism-ui`, never of the composition (D25).
- **Keys** go to the focused, enabled text field, which builds the new value from its current one and calls its `onValueChange` with it, by move:
  - a typed character is appended to the value;
  - backspace removes the last character, and does nothing when the value is empty;
  - enter is consumed and does nothing until keyboard actions exist.
  The field shows its `value` parameter, never a text of its own. If the program does not write the state, the field does not change, as in Compose's value-based `TextField`.
- **Handlers run through `Composition::run_handler`** (`state-handles` D12), so the cells they create belong to the app's composition, and their writes are allowed, since no pass is running. The function value is copied out of its handler entry first (D2).
- **A pass after every event.** After dispatching one event, the loop recomposes the invalid scopes and lays out again before it takes the next event. §5 planned one pass after all queued events. But then the second character of a fast `type "ab"` would be appended to the field's value from before the first one, and `click "<text>"` could not find a node that the previous click created. Per-event passes make each event see the previous event's result, which is what a user sees. They cost one pass per event, and events arrive at human rates. Pointer moves are not dispatched, since nothing uses hover.

Run-time cost: hit testing is O(nodes + elements) in the worst case and O(depth) when bounds prune. A dispatch allocates only what the handler allocates, plus the new `String` of a text-field edit.

Rejected:
- Batched passes (above).
- Clicks on press (§5 D22).
- Hit testing by node kind (this change's first design): a `clickable` `Box` or `Text` could not be clicked.
- A field-owned text buffer that updates before the state does: it diverges when the program rejects an edit.
- Disabled controls that pass clicks through.

### D15. `runApp`, the frame loop and hosts (autonomous)

`runApp` is declared in `/prism/compose/ui`, the module that also holds Compose's `ui.window` package. Its lowering builds a root content adapter around `content`, as `composition` does (`compose-transform` D15), and calls `prism_ui_run_app(title, width, height, content)`. On the program thread, the entry:

1. Rejects a second app on the thread: `ui misuse: runApp called while an app is running`. During a pass, `Composition::start` raises `composition misuse: composition operation during a pass` anyway.
2. Takes the host that the CLI configured (`host.rs::configure`, D16). Without one, it raises `ui misuse: no window system is available; run with --headless`, until `desktop-window` installs a window host.
3. Creates the tree with a window root of the host's size, and starts the composition with `UiApplier` (first pass).
4. Loops:
   - while a scope is invalid, run a pass, then release the handler entries no node claimed (D23);
   - while layout is dirty, lay out;
   - otherwise block on `host.next_event()`, and dispatch what it returns (D14, followed by a pass and layout);
   - on `Resize`, set the root's size and mark it dirty;
   - on `Frame`, rebuild the display list if it is stale and call `host.present(&list)`;
   - on `Dump`, print;
   - on `Close` or the end of events, stop.
5. Drops the composition, which removes every group and node and releases what nodes own, then the handler table and the tree, and returns.

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

### D16. Headless runs, event scripts and dumps (autonomous, revised for D6 and D24; implements `compose-architecture` D21)

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
- **Text targets.** `"<text>" [<n>]` names the n-th node (default 1), in tree order, whose text is exactly the string: a text node's text or a text field's value. The command acts at the centre of that node's bounds, rounded down, and hit-tests normally from there, so `click "+"` clicks the button around the `+`. A target that matches no node ends the program with `event script line <n>: no node shows the text "<text>"`. `type` and `key` without a focused field end it with `event script line <n>: no text field is focused`. Scripts name targets by text so that they survive layout changes (§5 D21).
- **`frame`** builds the display list as a window frame would, and prints nothing.
- **`dump`** prints the window line `window <width>x<height>`, then one line per node in tree order, indented two spaces per depth: `<kind> #<number> <x>,<y> <width>x<height>`, with absolute bounds and the kinds `column`, `row`, `box`, `text`, `spacer` and `textfield`. Then come its non-default properties in this fixed order:
  - the text, quoted;
  - `color <c>`, `fontSize <n>`, `contentColor <c>`, `disabled` and `focused`;
  - `arrangement <a>` and `alignment <a>`, with Compose's names and `spacedBy(<s>)`;
  - its modifier elements in chain order: `padding(<all>)` or `padding(<s>,<t>,<e>,<b>)`; `width(<w>)`, `height(<h>)` and `size(<w>x<h>)`; `defaultMinSize(<w>x<h>)`, with `-` for an unspecified axis; `fillMaxWidth`, `fillMaxHeight` and `fillMaxSize`, followed by `(<fraction>)` when it is not 1; `weight(<w>)`, or `weight(<w> nofill)`; `align(<alignment>)`; `background(<c>)` and `border(<w> <c>)`, with a shape other than the rectangle after the colour as `RoundedCorner(<r>)` or `RoundedCorner(<p>%)`; `clickable` and `toggleable(on)` or `toggleable(off)`, with their role and `disabled` inside the parentheses (`clickable(Button)`, `toggleable(Checkbox on)`, `clickable(disabled)`); and `textFieldFrame(<c> <c>)`.
  Colours print as `#AARRGGBB`.
- **`dump draw`** prints one display list command per line: `clear`, `rect`, `border` and `text`.
- **Output.** Dumps go through the runtime's output buffer (`prism_runtime::write_output`, made public), so they interleave with `println` in program order and are flushed before a runtime error.
- **Fixtures.** The runner reads `// events: <file>` and `// headless: <width>x<height>` header lines. Either one adds `--headless[=<size>]`, and `// events:` adds `--events <file>` (relative to the case directory, the working directory), to the usual `prism run --check-allocations` of executable cases. A script error is a case with `// run: 1`.

Rejected:
- PNG goldens (§5 D21).
- A Prism-level test API (§5 D21: it needs `prism test`).
- Scripts parsed by the runtime at `runApp`: syntax errors would surface after the program's earlier output.
- Silently ignoring missing targets: a script that clicks nothing is a broken test.
- Dumping a node's resolved properties instead of its chain: two chains that lay out alike would dump alike, and a dump would not show what order the program wrote.

### D17. UI runtime errors are without a location (autonomous, revised for D6 and D23)

Two new `PanicKind`s of `prism-runtime`, after `state-handles`' kinds, both raised with `panic_without_location`, status 101:

- `UiMisuse`, `ui misuse: <detail>`, with the details:
  - `toolkit composables run only inside runApp`: a setter during a pass of another composition;
  - `node property set outside composition`: a setter outside any pass;
  - `stale node handle`;
  - `unknown node kind <k>`;
  - `runApp called while an app is running`;
  - `no window system is available; run with --headless`;
  - `a modifier chain holds more than 16 elements` (D6);
  - `modifier handlers are made only while runApp runs`: `clickable`, `toggleable` or a text field registering a handler with no app on the thread (D23);
  - `stale modifier handler`: a chain naming a handler entry that was released (D23);
  - `a handler that owns captures that are not Copy was given to two nodes` (D23).
- `EventScriptFailed`, `event script line <n>: <detail>`.

An entry point has no caller location (`standard-library` D10, `compose-runtime` D12). The misuse that ordinary programs can reach (`runApp` in a handler, toolkit composables in a plain composition, an overlong chain, a shared handler) is clear without one, because the detail names the construct.

Rejected: one kind for both (a script failure is not a program bug); locations through extra arguments on every setter and modifier call (code on every call for errors that only misuse reaches).

### D18. Hot reload interplay (implements the UI side of `compose-architecture` D23)

- **Nodes belong to node groups.** Their keys come from the toolkit composable's body and the `Node` call's position in it (D3), under the user composable's body key (`compose-transform` D5). An edit that keeps a composable's construct path keeps its groups, and therefore its nodes, with their focus and layout caches.
- **Kind changes.** A node group whose kind changed gets a new node (`compose-runtime` D9). Calling `Button` where `Text` was is a different body key anyway.
- **Handlers.** They are function values called through their descriptors' slots, so a reloaded handler body runs on the next click. A handler whose captures changed has a new slot (`hot-reload` D5) and a new descriptor, so it no longer compares equal to the entry its node holds (D23): the next pass of its author gives the node the new entry, and the node releases the old one.
- **The reload point** is at the top of the frame loop, between events (D15), which `compose-hot-reload` fills.

### D19. Costs, and how they are measured

| Run-time operation | Cost |
|---|---|
| toolkit composable per pass | one node group, 3 to 6 setter calls each doing an O(1) compare (text: O(length); modifier: O(elements)), no allocation when nothing changed |
| modifier chain call | a copy of about 650 bytes, no allocation; a handler element also one entry call to register |
| handler comparison | one entry call: two table lookups, then the descriptor and environment equality of `compose-transform` D9 |
| full layout | O(nodes + elements + characters of text measured) |
| relayout | O(dirty nodes + their ancestors + the children of each re-measured node) |
| display list | O(nodes + elements), rebuilt only when something drawn or laid out changed, into reused buffers |
| hit test | O(nodes + elements) worst case |
| dispatch | O(1) plus the handler, plus a `String` per text-field edit |
| idle | nothing |
| memory | one node record (about 300 bytes with its decoded chain) per node, plus its text; one handler entry (about 40 bytes) per handler; about 650 bytes per kept `Modifier` slot |

- **`crates/prism-ui/tests/allocations.rs`** installs a counting `#[global_allocator]` and drives an app from Rust through `test_support.rs` (hand-built content and handler function values, as `prism-compose`'s tests do). It checks that a steady-state iteration allocates nothing: an event whose handler writes nothing, then a pass that registers and compares handlers, a clean layout and a display list rebuild.
- **`crates/prism-ui/tests/ui_timing.rs`** is an ignored benchmark in the style of `compile_timing.rs`. It builds trees of 1,000, 10,000 and 100,000 nodes (a column of rows, each holding a text and a clickable box with a background) through the tree's Rust API. It reports full layout, relayout after one text change, a display list build and a hit test. Under `PRISM_ASSERT_TIMING=1` it asserts roughly linear growth, and that relaying out one leaf of the 10,000-node tree costs under a tenth of a full layout. Medians at apply time (release build, 2026-10-19, a loaded machine, so ±50% between runs), for 1,000, 10,000 and 100,000 nodes: full layout 0.048, 0.443 and 2.79 ms; relayout after one same-width text change under 1 µs at every size (0.0002 of a full layout at 10,000); display list build 0.027, 0.246 and 1.87 ms; a hit test of the first row, which the reverse walk reaches last, 0.007, 0.063 and 0.685 ms. Growth per tenfold size: layout ×9.1 and ×6.3, display list ×9.2 and ×7.6. A text change that changes the text's width re-runs its row's and column's policies, O(rows), which the benchmark does not measure.
- **`crates/prism-cli/tests/ui_run_timing.rs`** (ignored) runs a generated app of 1,000 `Text`s and a counter headless, with a script of 100 clicks, and reports the median per click. Its release-build target is `reactive-app-verification`'s. A process cannot time its own events from outside, so the time per click is the difference between a run of 100 clicks and one of none, over 100, from a warm build cache: at apply time (release build, 2026-10-19) the median over 10 pairs was 0.19 ms per click, against the 1 ms that `reactive-app-verification` plans.

| Compile-time stage | Cost |
|---|---|
| companion lookup (D4) | one filtered lookup when a value name finds a struct; one kind compare per duplicate |
| `Int.dp` and `Int.sp` (D7) | a member lookup and a constant lowering per use |
| `Node` and `LeafNode` lowering (D3) | constant per call, one key hash |
| `runApp` (D15) | one adapter per calling body, one `E4030` flag check |
| receiver lambdas and context parameters (D22) | `receiver-lambdas`' cost: an implicit first parameter, and a look at the innermost implicit receiver per scoped call |
| the toolkit modules (D21) | four library modules, each checked once per compiler version and cached; a program parses only those it imports, directly or through the others |

`compile_timing.rs` gains `ui-calls`, one composable calling `n` toolkit composables with modifier chains inside a `Column`, at its usual sizes, and asserts roughly linear growth under `PRISM_ASSERT_TIMING=1`. The benchmark checks the generated file against the toolkit modules' ABIs (`prism_cli::library_abis`), as a cached build loads them. At apply time (release build, 2026-10-19), for 256, 1,024 and 4,096 calls, in ms: type 4.3, 13.0 and 53.2; MIR 5.2, 16.5 and 73.3; borrow 39.5, 155.5 and 692.8; compose 8.5, 32.0 and 171.5; lowering 24.1, 87.2 and 359.9; Cranelift 18.3, 66.5 and 276.4. Every stage grows by x3.6 to x5.4 per x4 (compose the most, at 4,096). The borrow check is the largest stage, about 0.17 ms per call: each toolkit call omits arguments, and each omitted argument's generator call borrows every earlier argument, so a call costs its parameters squared, constant per call. The startup benchmark of `run_timing.rs`, with `UI_FUNCTIONS` in the entry-point lookup, gave a cold total to `main` of 12.5 ms (median of 20, p90 12.5 ms), under its 15 ms target, and 2.4 ms warm.

### D20. A new `ui` capability, and how the change is tested

The toolkit's behaviour is new and is a library over compose, so it is a new `ui` capability rather than a part of `compose`. The language rule goes to `type-system` and `compilation-and-abi`, and the library entries to `standard-library`. The node-group operation goes to `mir`, the entry tables, word-sized intrinsic structs and intrinsic equality entries to `code-generation`, and the CLI options to `execution`. The diagnostics catalog is edited directly: `E4010` gains the companion exception, and `E4030` names `runApp`. No error code is added: a scoped modifier outside its scope is rejected with the code `receiver-lambdas` gives a missing context.

- **Fixtures.** Behaviour a program can observe is pinned by executable fixtures under `crates/prism-cli/tests/fixtures/ui/`, nearly all headless with scripts and `dump`, under `--check-allocations`. Companion values have `type-system` and `compilation-and-abi` fixtures, and node groups a `mir` fixture with `composed-mir.snap`.
- **Rust tests.** What a program cannot observe is pinned by unit tests in `prism-ui`: the layout cache, the chain fold, the handler table's lifecycle, release order, re-entrancy, measurer independence and the allocation-free steady state.

### D21. One module per Compose artifact, under `/prism/compose/` (user direction 2026-10-19)

The toolkit is split as Jetpack Compose splits its artifacts, one bundled module per artifact, all files of the package directory `prism/compose/` (package `prism.compose`):

| Module | Compose artifact (packages) | Declares |
|---|---|---|
| `/prism/compose/runtime`, in the prelude | `compose-runtime` (`androidx.compose.runtime`) | `Composition`, `composition`, `setContent`, `recompose`, `dump`, `remember`, `key`, `RecomposeScope`, `currentRecomposeScope`, `invalidate` (`compose-transform`); `State`, `MutableState`, `mutableStateOf`, `value`, `read`, `update`, `edit`, `SideEffect` (`state-handles`) |
| `/prism/compose/ui` | `compose-ui`, with `ui-graphics`, `ui-unit` and `ui-text` (`androidx.compose.ui`, `.ui.graphics`, `.ui.unit`, `.ui.text`, `.ui.semantics`, `.ui.window`) | `Modifier`, `then`, `ModifierElement`, `ModifierElements`, `Alignment`, `Color`, `Shape`, `RectangleShape`, `Dp`, `dp`, `TextUnit`, `sp`, `TextStyle`, `Role`, `HandlerId`, `runApp`; with `package` visibility, the node layer of D3 and the handler externals of D23 |
| `/prism/compose/foundation` | `compose-foundation` (`androidx.compose.foundation`, `.foundation.shape`, `.foundation.text`, `.foundation.selection`) | `background`, `border`, `clickable`, `toggleable`, `RoundedCornerShape`, `CircleShape`, `BasicText`, `BasicTextField` |
| `/prism/compose/layout` | `compose-foundation-layout` (`androidx.compose.foundation.layout`) | `Box`, `Column`, `Row`, `Spacer`, `BoxScope`, `ColumnScope`, `RowScope`, `Arrangement`, `padding`, `width`, `height`, `size`, `defaultMinSize`, `fillMaxWidth`, `fillMaxHeight`, `fillMaxSize`, `weight`, `align` |
| `/prism/compose/material3` | `compose-material3` (`androidx.compose.material3`) | `Text`, `Button`, `ButtonColors`, `ButtonDefaults`, `Checkbox`, `TextField`, `Surface`, `MaterialTheme`, `ColorScheme`, `Typography` |

`/prism/composer`, `compose-runtime`'s protocol module for fixtures written in lowered form (its D10), stays where it is: it is no Compose artifact, and the test-facing runtime module is still useful to the runtime's fixtures.

- **Why one package directory.** `package` visibility reaches exactly one package directory, so files of `prism/compose/` share the node layer (D3) and the handler externals (D23) without making them public. Compose's `foundation.layout` package would be `/prism/compose/foundation/layout`, in the package `prism.compose.foundation`, which could reach that layer only through public declarations; so the module is `/prism/compose/layout`, named after the artifact `compose-foundation-layout`. Nothing in Prism is called `layout` otherwise, since Compose's `ui.layout` (`Layout`, `MeasurePolicy`) has no public counterpart yet.
- **Why sub-packages are folded into their artifact.** Prism imports a module, not a declaration, so a module per Compose package (`/prism/compose/ui/graphics`, `/prism/compose/ui/unit`, `/prism/compose/foundation/shape`, ...) would only add import lines, at the same compile cost, and would split the node layer across packages again.
- **The prelude holds only the runtime.** `compose fun` is part of the language (the first-class compose and hot reload invariant), and `remember` and `mutableStateOf` are in almost every composable, so `/prism/compose/runtime` stays the prelude module that `compose-transform` made `/prism/compose`, renamed. The toolkit modules are imported as Compose's are, so a program that shows no UI never reaches them. A screen typically imports `/prism/compose/ui`, `/prism/compose/layout` and `/prism/compose/material3`, and `/prism/compose/foundation` when it uses `background`, `border` or `clickable`; material3 reaches the other three itself, so its program parses all four, each checked once per compiler version and cached.
- **Dependencies** run one way, as Compose's artifacts do: `ui` imports nothing of the toolkit, `foundation` and `layout` import `ui`, and `material3` imports the three. Library files import what they use, with sibling imports (`import ui`).

Compile-time cost: the same declarations as one module, in four files; a program that imports none of them pays nothing, and one that imports material3 pays four cached ABI loads. Kotlin-likeness: Compose's artifact boundaries and names, with one import line per artifact instead of one per declaration.

Rejected:
- One `/prism/ui` module (this change's first design): Compose code would not find its names where Compose puts them, and the toolkit's layering (material over foundation over ui) would not be visible.
- A module per Compose package (above).
- `/prism/compose/foundation/layout` with a public node layer in `/prism/compose/ui/node`: a stopgap API in programs' reach (D3).
- Keeping the runtime at `/prism/compose`: the runtime would be the one compose module outside `/prism/compose/<artifact>`.
- No compose module in the prelude: every composable file would import the runtime, against the first-class compose invariant's spirit, for the saving of one small intrinsic module that every program's cache already holds; and the scenarios of `compose-transform`, `state-handles` and `compose-hot-reload` are written without the import.
- Toolkit modules in the prelude: every program would reach four more modules, and names such as `Text`, `Box` and `Color` would be in scope in non-UI code.
- `material` instead of `material3`: Compose's current Material library is `material3`, whose baseline colours and sizes D9 uses; `material` is Material 2.

### D22. Scoped content lambdas, through a separate `receiver-lambdas` change (user direction 2026-10-19)

`Column`, `Row` and `Box` take their content as `compose ColumnScope.() -> Unit`, `compose RowScope.() -> Unit` and `compose BoxScope.() -> Unit`, and `Button` as `compose RowScope.() -> Unit`, as in Compose. `weight` and `align` are extensions of `Modifier` with a context parameter of the scope:

```
public struct ColumnScope : Copy { }
context(scope: ColumnScope) public fun Modifier.weight(weight: Double, fill: Bool = true): Modifier
context(scope: ColumnScope) public fun Modifier.align(alignment: Alignment): Modifier
public compose fun Column(..., content: compose ColumnScope.() -> Unit) {
    Node(columnKind, update = { node -> ... }) { content(ColumnScope()) }
}
```

So `Modifier.weight(1.0)` resolves only inside a row's or column's content, and `align` means the alignment of the innermost scope, as Compose's `ColumnScope`, `RowScope` and `BoxScope` members do.

Prism has extension functions, whose bodies have an implicit `this`, but no function types with receivers and no member extensions. This change therefore depends on a new language change, `receiver-lambdas`, which `ROADMAP.md` places before it, with this scope (its own packet decides the details and the syntax):
- function types with a receiver, `T.(A) -> R`, `compose` and `escaping` included, assignable from lambdas checked with `this: T` as in extension bodies, and called as `receiver.f(a)` or `f(receiver, a)`;
- extensions with a context parameter, Kotlin's `context(scope: T)`, callable only where the innermost implicit receiver (an extension's or a receiver lambda's `this`) has type `T`, the scope then passed as a leading argument; two extensions of one name and receiver may coexist when their contexts differ;
- only the innermost implicit receiver is consulted, as Compose's `@LayoutScopeMarker` makes outer scopes' members unavailable, so a `Box` inside a `Column` sees `BoxScope.align`, never `ColumnScope.align`.

Its compile-time cost is low: a receiver lambda is a lambda with an implicit first parameter, its `this` resolved as an extension body's, and a context-parameter call looks at one implicit receiver, constant per call, with no inference and no search through outer scopes.

Rejected:
- Plain `weight` and `align` modifiers with a run-time `ui misuse` when no row or column reads them: Compose code that Compose rejects would compile, and the mistake would show at run time, as silently ignored layout.
- Kotlin's member extensions in an interface: Prism has no interfaces.
- An explicit scope argument (`Modifier.weight(this, 1.0)`): unlike Compose.
- Building receiver lambdas inside this change: a language feature with its own scenarios in `syntax`, `type-system` and `typed-tree`, which other APIs (builders, DSLs) will use too.

### D23. Handlers live in a table of `prism-ui`, named by a `Copy` `HandlerId` (user direction 2026-10-19)

A `Copy` modifier cannot hold an owned closure, and modifiers may not run composition (D25). So a handler given to `clickable`, `toggleable` or `BasicTextField` is moved into the thread's handler table of `prism-ui`, and the element holds its `HandlerId`, a `public intrinsic struct HandlerId : Copy` of `/prism/compose/ui` of one word (a 32-bit index and a 32-bit generation) that no program can construct:

- **Registering.** `clickable(enabled, role, onClick)` calls the `package` external `prism_ui_register_click(onClick)`, which moves the function value into a free entry and returns its id; `toggleable` uses `prism_ui_register_toggle` for `(Bool) -> Unit`, and `BasicTextField` `prism_ui_register_text` for `(String) -> Unit`. The entry is pending. Registering with no app on the thread is `ui misuse: modifier handlers are made only while runApp runs`.
- **Claiming.** When a node's chain or `set_on_value_change` names a pending entry, the node claims it: the entry is the node's, and the entry the node held in that position, if another, is released. Naming an entry the node already holds changes nothing. Naming an entry another node holds gives this node a duplicate when the handler is keepable (its descriptor's duplicate entry, `compose-transform` D9, or a copy of the word when that is null), and otherwise ends the program with `a handler that owns captures that are not Copy was given to two nodes`. Naming a released entry is `stale modifier handler`.
- **Releasing.** An entry is released, dropping its function value, when its node takes another in its place or is released, and, while still pending, at the end of the pass during which it was registered; one registered between passes, by a handler, at the end of the next pass. The app releases every entry when it ends. So every handler has one owner at a time and is freed exactly once, without counting.
- **Equality.** `HandlerId`'s equality is a catalog entry that calls `prism_ui_handlers_equal(a, b)`: an id is equal to itself, two live entries are equal when their handlers have the same descriptor and equal environments by the descriptor's environment equality entry (`compose-transform` D9), and any other pair, a handler that owns non-`Copy` captures or a released entry included, is unequal. So `Modifier.clickable { count.value += 1 }` made on two passes compares equal, a composable that receives it skips, and its node keeps the first closure, which does what the second would. A closure owning a `String` never compares equal, so a composable receiving it reruns with its caller and its node takes the new entry, as Compose reruns for a lambda that captures a changed value.

Run-time cost: one entry call per registration, per comparison and per claim; the table is a reused vector, so a steady-state pass whose handlers capture `Copy` values in one word allocates nothing. Compile-time cost: three externals and one catalog entry with an equality entry.

**Review fixes (2026-10-19).** Two defects found by headless apps after the integration:
- **Kept ids must stay live (autonomous).** A skippable composable keeps its old argument when the new one is equal, and stores the new one when it is not. The first implementation's node always kept the entry it held when the new entry was equal, so a `Card(modifier)` receiving `Modifier.toggleable(value = t.value) { ... }` stored the new chain after a toggle while its node kept the old entry and released the new one at the end of the pass: from then on `Card` compared every new chain with a released id and never skipped again. Two rules now keep every kept id live. Registering a handler whose environment is a word owning nothing (no drop and no duplicate entry: a closure capturing nothing or one `Copy` word, the common case) gives the id of a live entry with the same kind, code and word, after the equality entry confirms it, so such handlers built on any number of passes are one id. For other keepable handlers a node keeps its entries when the whole new chain equals its own, element by element with handlers compared by equality, and otherwise takes the entries the new chain names: exactly what the composable directly above the node keeps, which for every toolkit composable is the chain it hands to its node. Cost: one hash lookup and at most one equality call per registration; no allocation once the table has grown. Rejected: forwarding released ids to the entry that superseded them (tombstones that nothing can free), a hash entry in descriptors so that allocated environments are interned too (a `compose-transform` change; a ROADMAP follow-up), and having skipped bodies continue with their kept arguments (a change to every skippable body).
- **Equality runs program code outside the state.** An environment equality entry is generated code, and one that compares a captured `Modifier` calls `prism_ui_handlers_equal` again, which borrowed the thread's state a second time and aborted the process. Comparisons now copy the two function values out of their entries (`Comparable`), let the state go, and compare (D2); the setters compute the equalities of a chain before claiming.

Rejected:
- Closures inside the elements (Compose's `ClickableElement(onClick)`): `Modifier` would not be `Copy` (D6).
- A slot of the composition holding the handler, kept by a composable `clickable`: `Modifier.composed` under another name (D25).
- Ids that never compare equal: simpler, but every composable receiving a clickable modifier would rerun whenever its caller did, losing the skipping Compose gets from memoized lambdas.
- Sharing one entry between nodes by counting: the zero-cost memory safety invariant forbids hidden reference counts.
- Keeping pending entries until the app ends: a chain built on every pass would grow the table without bound.

### D24. Material components are written over foundation and layout (autonomous)

As in Compose, material3 has no nodes of its own:
- `Surface` is a box node, through a `package` helper of material3 that also sets its content colour, with `modifier.background(color, shape)`;
- `Button` is that box with `modifier.background(container, shape).clickable(enabled, Role.Button, onClick)` and content colour, holding a `Row(Modifier.defaultMinSize(ButtonDefaults.MinWidth, ButtonDefaults.MinHeight).padding(horizontal = 24.dp, vertical = 8.dp), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically, content)`;
- `Checkbox` is a `Box` with `modifier.toggleable(checked, enabled, Role.Checkbox, onCheckedChange).size(20.dp)` followed by its fill and border;
- `TextField` is a `BasicTextField` with `modifier.defaultMinSize(minWidth = 200.dp).textFieldFrame(...).padding(8.dp)`, `textFieldFrame` being a `package` element of ui drawn by the node from its focus;
- `Text` resolves its colour and size against its style (D9) and calls `BasicText`.

So the Rust side has six kinds and no component policies, any node can be clickable, and the dumps show the nodes Compose's own tree would hold: a button is a `box` holding a `row`. Roles (`Role.Button`, `Role.Checkbox`) are Compose's semantics roles, printed in dumps and available to scripts.

Rejected: a Rust kind and layout policy per Material component (this change's first design): material would not exercise the foundation API it stands on, and a user component could not do what `Button` does.

### D25. Modifiers are plain data; no `Modifier.composed` (user direction 2026-10-19)

No modifier function is composable, remembers, starts a group or reads state: a modifier element is a value, and applying it creates or updates state in the UI node it lands on, owned by `prism-ui`. This is the design Compose moved to with `Modifier.Node` and `ModifierNodeElement` after deprecating `Modifier.composed { }`, which composes on every use of the modifier, keeps slots per element and makes chains compare unequal, so the composables receiving them never skip. Here:
- an element is compared by equality on recomposition (D6), and a chain that did not change costs one comparison;
- the state a stateful modifier needs lives in its node: the pressed target and the focus (D14), the handler entries (D23), the bounds at each element (D11);
- nothing in the composition is created by a modifier, so a modifier built outside a pass, in a handler or a top-level value, means what it means inside one.

A future public API for custom modifiers follows `ModifierNodeElement` (an element that creates and updates a node), not composition. `compose-transform`, `state-handles` and `compose-hot-reload` plan no composed-modifier mechanism (checked 2026-10-19).

Rejected: a composable `Modifier.clickable` remembering its handler slot (see D23); `Modifier.composed` for user modifiers.

## Risks / Trade-offs

- **[Headless and windowed layouts may differ]** if `desktop-window` lays out with Inter's metrics (D12). → Fixtures pin the headless layout, and the window decision is `desktop-window`'s, recorded there.
- **[Companion values are a language rule added by a library change]** → It is small, Kotlin-shaped and general (D4). Its scenarios are in `type-system`, and it can be revisited before release.
- **[A new language prerequisite]** This change waits for `receiver-lambdas` (D22). → Its scope is small and independent of compose, so it can land while the compose changes are implemented; task groups 3 to 9 do not need it.
- **[A modifier is about 650 bytes]** Each chain call copies it, and every skippable composable taking one keeps a copy. → Copies are cheap next to a pass and allocate nothing; a packed representation, or arrays, can come later behind the same API.
- **[A capacity of 16 elements]** A program chaining more fails at run time. → The error names the limit; Compose code rarely comes near it.
- **[Element internals are public]** Struct fields have no visibility, so `ModifierElement` and the blocks are readable and constructible. → An element built by hand means what the function building it means, and `HandlerId` cannot be forged; field visibility, a ROADMAP follow-up, will hide them.
- **[Handlers outside the node]** A modifier with `clickable` kept past its pass in a top-level value or a remembered value can name a released entry. → `stale modifier handler` ends the program, never a call of freed code.
- **[A pass per event]** A burst of events recomposes once per event (D14). → Events arrive at human rates, and pointer moves are not dispatched.
- **[Full display list rebuilds]** Every change rebuilds the whole list (D13). → O(nodes) with reused buffers; dirty regions are a follow-up for large trees.
- **[Prerequisites unimplemented]** Every name taken from `compose-runtime`, `compose-transform`, `state-handles`, `standard-library` and `receiver-lambdas` comes from their packets. → Task group 1 checks them against the code those changes leave.

## Migration Plan

No program that runs today changes meaning. A same-file pair of a struct and a top-level `val` with one name was `E4010` and is now accepted. A struct name used as a value without a companion is still `E4002`. `examples/demo/ui/ui.prism` moves from `Column(padding = 16)` to `Column(modifier = Modifier.padding(16.dp))` with `import /prism/compose/ui`, `/prism/compose/layout` and `/prism/compose/material3`, and checks. `prism run` gains options and no new default behaviour.

## Open Questions

None blocking. The follow-ups go to `ROADMAP.md` at archive:
- a public `Layout` with measure policies in Prism, and a `ModifierNodeElement`-style API for custom modifiers, which would make `NodeId` public;
- `LazyColumn`, scrolling, `Modifier.clip` and `matchParentSize`;
- intrinsic measurements;
- composition locals, `MaterialTheme { }`, `LocalContentColor` and `ProvideTextStyle`;
- `TextStyle`'s other fields, font weights and text-field options, and Material 3's filled `TextField` with `OutlinedTextField` beside it;
- `Dp` arithmetic once operators can be declared, and fixed-size arrays for the modifier's blocks;
- dirty-region display lists, and several apps per thread.
