# Proposal: state-handles

## Why

`compose-runtime` gives Prism a slot table with restart scopes and invalidation, and `compose-transform` makes composables run, skip and restart, but nothing a program does can make a scope invalid except `currentRecomposeScope().invalidate()`. Compose's model is reactive state: a composable reads `count.value`, an event handler writes `count.value += 1`, and exactly the scopes that read it recompose. This change is item 29 of "Then: the road to Milestone 2" in `ROADMAP.md`, the state layer planned in `docs/architecture.md` §3 ("Reactive Tree Copy State Handles") and §5 (`compose-architecture` D12 to D16). It adds `State<T>` and `MutableState<T>` as `Copy` 64-bit handles into a runtime-owned cell arena, the Kotlin-style API over them, read tracking and invalidation, the static and run-time rules that keep state read-only during composition, and the ownership that frees every cell exactly once. `ui-toolkit`'s handlers, the counter and the todo app of `reactive-app-verification` are written against it.

## What Changes

- **Handles** (D3, D8, D15). `State<T>` and `MutableState<T>` are intrinsic structs of `/prism/compose/runtime`, `Copy` for every `T`, invariant in `T`, one 64-bit word each: a 32-bit cell index and a 32-bit generation. `MutableState<T>` converts to `State<T>` as a subtype step that keeps the word, and members and extensions of `State<T>` are found on a `MutableState<T>`. A handle whose cell was freed is stale: using it ends the program with `panic: stale state handle` and its location.
- **The API** (D2, D4, D5, D6), with Kotlin's names. `mutableStateOf(value)` makes a cell. `s.value` reads a copy of a `Copy` value anywhere; `s.value = v` writes any value outside composable contexts, and `s.value += 1` reads and writes a `Copy` one. A non-`Copy` value (`State<List<Todo>>`) is a place that only a composable context can borrow, as `for (todo in todos.value)` does; anywhere else it is read with `todos.read { it.size }`, replaced with `s.update { next(it) }` and changed in place with `todos.edit { it.add(Todo("x")) }`. `value` is declared with a new `intrinsic var` property form, writable on `MutableState<T>` only.
- **Run-time borrow flags** (D6). While `read`, `update` or `edit` runs, its cell is borrowed shared, moved out or borrowed mutably, and a conflicting access to that cell ends the program with `panic: state accessed while borrowed`.
- **Read tracking and invalidation** (D9, D10). A read of a state during a pass registers the innermost restart scope as a reader, also through plain functions and content lambdas; a read outside a pass registers nothing. A write invalidates the cell's readers for the next pass, except a `value =` write equal to the current value by `==`, which keeps the old value and invalidates nothing. `update` and `edit` always invalidate. Recomposition stays explicit: `c.recompose()` (and later `runApp`'s frame loop) runs the next pass.
- **State is read-only during composition** (D7, D11). New type errors: `E4031` (`StateWriteInComposable`) for a write in a composable context, `E4032` (`UnrememberedState`) for `mutableStateOf` in one, with help `remember { mutableStateOf(0) }`, and `E4033` (`StateValueOutsideComposable`) outside composable contexts for a use of a non-`Copy` `s.value` other than assigning it, and for a mutable borrow or part write of any `s.value`, with help naming `read`, `edit` or `update`. New runtime errors, with locations: `state written during composition` and `state created during composition outside remember`, for what plain functions called from composition do.
- **Ownership** (D12). A cell belongs to the `remember` slot whose calculation created it, to the composition whose handler or side effect created it, or to the program (also when a top-level value's initializer creates it). It is freed, its value dropped, when its slot is forgotten or its group removed (at the end of the pass), when its composition is dropped, or, for program cells, before `--check-allocations` reports. A cell released while `read`, `update` or `edit` borrows it is freed when that call ends.
- **`SideEffect`** (D13, brought forward from `compose-architecture` D25). `SideEffect { ... }` runs its closure after the pass that composed it, outside composition, as Jetpack Compose's does. It is the one way, before `ui-toolkit`'s handlers, for code to act on behalf of a composition, and the executable fixtures of composition-owned cells and stale handles need it.
- **Dumps** (D19). A composition's dump lists each cell under the slot that owns it (`cell String`).
- **Costs** (D21). The type checker's work is a member lookup, a marker lookup and a composable-flag test per state access, with no inference beyond the generic call checking intrinsics already get. MIR lowering expands each access into a constant number of calls. At run time each access is one entry call, a generation compare and, during a pass, an amortized O(1) reader registration; writes cost one equality call and O(readers) invalidations. `compile_timing.rs` and `runtime_timing.rs` gain shapes.
- **Not in scope.** `derivedStateOf`, `snapshotFlow`, `mutableStateListOf`, mutation policies, `by` delegation of `value` (Kotlin's `var count by remember { ... }`), effects other than `SideEffect`, clearing a scope's old reads when it recomposes, threads and snapshots (`compose-architecture` D16, D25). Event dispatch to node handlers and the frame loop are `ui-toolkit`'s, which runs handlers through this change's Rust API so that the cells they create belong to their composition.

## Capabilities

### New Capabilities
None. State is part of the `compose` capability that `compose-runtime` creates and `compose-transform` extends.

### Modified Capabilities
- `compose`: state handles, `mutableStateOf`, reading, writing and borrowing a state's value, `read`, `update` and `edit`, run-time borrow flags, read tracking, invalidation and equal writes, the static and run-time rules of state during composition, cell ownership and release, stale handles, `SideEffect`, cells in dumps, and the costs.
- `standard-library`: the state declarations of `/prism/compose/runtime`, and writable intrinsic properties.
- `syntax`: `intrinsic var` property declarations.
- `formatting`: `prism fmt` prints `intrinsic var` declarations.
- `type-system`: `MutableState<T>` converts to `State<T>`, and lookups on a `MutableState<T>` receiver fall back to `State<T>`.
- `typed-tree`: state places and state operations in the typed tree.
- `mir`: state operations as calls, and their dump.
- `references`: borrows of a state's value, and function values written into states escape.
- `code-generation`: the layout of handles, state operations as calls of the compose runtime, and the slot type records of cell types.
- `execution`: the state runtime errors and their locations, and program-owned cells released before the allocation check.

## Impact

- `crates/prism-compose` (from `compose-runtime`): a new `state.rs` (the cell arena, borrow flags, readers, creation owners), additions to `thread.rs`, `slot.rs`, `composition.rs`, `dump.rs`, `entry.rs` and `functions.rs` (the `prism_compose_state_*` entries, `prism_compose_begin_remember`, `prism_compose_end_remember`, `prism_compose_side_effect`, the program-scope entries), and Rust tests and benchmark shapes.
- `crates/prism-runtime`: four `PanicKind`s and the finish hook ordering.
- `crates/prism-library/prism/compose/runtime.prism` (from `compose-transform`): the state declarations and `SideEffect`.
- `crates/prism-parser`, `crates/prism-ast`, `crates/prism-abi`, `crates/prism-formatter`: `intrinsic var`.
- `crates/prism-typechecker`: the catalog entries, `.value` resolution, the subtype step and lookup fallback, `E4031` to `E4033`, ownership roots for written values.
- `crates/prism-typed-tree`: `ExpressionKind::StatePlace` and five `BuiltinFunction`s.
- `crates/prism-mir`: `Callee::State(StateOperation)`, lowering of state places, assignments and the three closure members; dump, validation, instantiation.
- `crates/prism-borrowck`: state place borrows treated like top-level value borrows.
- `crates/prism-compose-transform` (from `compose-transform`): the remember window and `SideEffect`.
- `crates/prism-codegen`: handle layouts, state operations, cell records with equality glue for every type with `==`, the program scope around top-level initializers.
- `crates/prism-cli`: fixtures under `tests/fixtures/compose/`, `type-system/`, `references/` and `execution/`, and timing shapes.
- Specs: the deltas above; the diagnostics catalog (`E4031` to `E4033`) is edited directly. `docs/architecture.md`: §3 "Reactive Tree Copy State Handles" and §5 "State Handles Are Generational Copy Handles", "Reading and Writing State", "State Is Read-Only During Composition", "State Cells Belong to Their Creator" and the reads of "Restart Scopes and Invalidation" move out on archive; D25's `SideEffect` exception is noted.
- **Prerequisites:** `compose-transform` is archived first, which brings `compose-runtime`, `standard-library`, `collections`, `escaping-closures` (archived), `monomorphization` (archived), `instance-cache` and `hot-reload`. None of `compose-runtime`, `compose-transform`, `standard-library`, `instance-cache` or `hot-reload` was implemented when this change was proposed, and `collections` was being implemented. See `design.md`, "Baseline".
