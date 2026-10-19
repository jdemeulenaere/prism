# Proposal: owned-state

## Why

State is a `Copy` handle today (`state-handles`): `MutableState<T>` names a cell that a `remember` slot, a composition or the program owns, so state cannot live inside data. A todo item cannot hold its own `done` state, so the todo app keeps one `MutableState<List<Todo>>` and every toggle rewrites the list (and recomposes every row, review finding F3). Outside composables a non-`Copy` value is reached only through `read`, `update` and `edit` closures (`E4033`), which Kotlin never needs. The post–Milestone 2 review agreed to make `MutableState<T>` an owning state object, as in Compose (A2′), with no stale-state errors in UI code (A2‴), and to have the library, not a type name in the compiler, declare how closures capture state (L5 of A4) (`docs/milestone-2-review-notes.md`, "Agreed changes"; user, 2026-10-20). It is item 4 of "After Milestone 2: the review round" in `ROADMAP.md` and builds on `path-captures` (item 3), whose captured paths stop before a state's `value` and leave a fourth capture mode to this change (its D11).

## What Changes

- **`MutableState<T>` owns its state** (user direction 2026-10-20). It becomes a move-only value, one word naming a runtime cell whose contents never move, so it can be a struct field (`struct Todo { val id: Int, val title: String, val done: MutableState<Bool> }`), a list element or a local. Dropping it retires the state. `==` compares identity. `State<T>` stays the read-only face of the same object: `&MutableState<T>` converts to `&State<T>`. **BREAKING**: state is no longer `Copy`; composables and functions take `&State<T>` or `&MutableState<T>` (passed with an implicit borrow, `Label(count)`), and copying a handle (`val t = s`) moves it.
- **A state's value is a place that borrows normally.** `todos.value.add(x)`, `for (t in todos.value)` and `todos.value[i].title` work in any context, and `value` is written through any access, as in Kotlin (`count.value += 1` on a `val` or a `&MutableState`). In composables nothing is checked at run time (state is read-only during a pass). Elsewhere a borrow of a non-`Copy` value, and any mutable borrow, holds the state's run-time borrow flag until its statement ends (until its binding's block when bound by `val`), so two aliases of one state that conflict still end the program with `state accessed while borrowed`. **BREAKING**: `read` and `edit` are removed and `E4033` is retired; `update` stays.
- **`remember` keeps owned values** (user direction). `remember { mutableStateOf(0) }` keeps the state object in its slot and gives a `&MutableState<Int>`, valid until the end of the body; `Copy` values are still returned by copy; the slot drops the value when its group leaves, its key changes or its composition is disposed. `E4021` no longer applies to remembered values; keys stay `Copy`.
- **No stale-state errors in UI code** (user direction, A2‴):
  - **R1.** When a group is removed, its nodes and the handlers they own are released before the values of its slots.
  - **R2.** Dropping a state retires it: its readers are invalidated, an explicit weak reference to it gives `null`, and the cell is freed at the end of the next composition pass (at once when no composition is live).
  - **R3.** A new parameter kind, `attached` (`onClick: attached () -> Unit`), is for function values that the composition keeps and releases with the group that passed them: UI handlers, `SideEffect`'s effect, and the root content of `composition`, `setContent` and `runApp`. A closure passed to an `attached` parameter captures each state it uses (a path such as `todo.done` or `*count`) as a generational weak handle, one `Copy` word, so `Checkbox(onCheckedChange = { todo.done.value = it })` works on a borrowed `todo`. An `attached` value may only be called or passed to another `attached` parameter (new `E5010`). Any other escaping closure that uses a borrowed state is `E5007`, whose help now suggests an explicit weak reference: `val ref = count.weak()` and `ref.get()?.update { it + 1 }`. The generational check stays as a safety net (`stale state handle`).
- **The library declares weak capture** (user direction, L5). `/prism/compose/runtime` declares `WeakReference<T> : Copy`, `get()`, and `public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>` (and the same for `State<T>`); the new `capture` modifier marks the function that attached closures use to capture a type. The compiler finds it from the type's declarations, never from its name. `E4029` covers its shape.
- **Borrowed state parameters are stable.** A `&State<T>` or `&MutableState<T>` parameter is kept as its weak handle and compared by identity, so a composable taking only states and `Copy` values stays skippable (as it was with `Copy` handles) and is a restart scope. This is the state-object part of `strong-skipping` (A1, next change), which generalizes identity to every borrowed parameter.
- **Ownership replaces cell owners.** A cell belongs to its state object, not to a slot, composition or program (revising `compose-architecture` D15 and `state-handles` D12); composition dumps no longer list cells. `E4032` and the run-time check for state created during a pass outside `remember` stay. Equal `value =` writes still invalidate nothing. Observation only: no snapshot isolation (snapshots and threads come later, user).
- **System invariant 6 is amended** (user direction): state objects are owned values, and generational state handles are an implementation detail of the runtime and of attached closures.
- **Examples.** The todo app gives `Todo` a `done: MutableState<Bool>`, keeps a plain `List<Todo>` in a remembered `MutableState`, remembers its states in the screen, and lets each row's checkbox write only its own state, in a restart scope of its own, so a toggle recomposes one checkbox rather than every row. The counter's source is unchanged. `examples/run` uses an explicit weak reference for its returned click handler.
- **Not in scope:** snapshots and threads; weak captures of holder structs whose fields are states (a follow-up: today a handler names the states it uses); `Clone` for states; assignment through `?.` (`ref.get()?.value = 1`); identity for borrowed parameters other than states (`strong-skipping`); delegated properties (`by`).

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `compose`: state objects, places, guards, retirement and deferred frees, release order, `remember` of owned values, weak references, stable borrowed states; removes the cell-owner, `read`, `edit`, `E4033` and dump requirements; scenarios use `&State<T>` parameters.
- `references`: weak captures for attached closures, attached values staying attached (`E5010`), remembered values holding no loan, state value borrows as normal loans, `E5007`'s weak-reference help.
- `type-system`: `attached` in function types and its conversions, the read-only conversion behind references, the rule type of `remember`.
- `syntax`: the `attached` parameter modifier and the `capture` function modifier.
- `standard-library`: the new state declarations, capture functions, `remember`'s signature.
- `typed-tree`: state values and operations over borrowed state objects, weak capture modes.
- `mir`: guards, weak captures, `remember` of owned values, state operations.
- `code-generation`: state object layout and drop glue, weak references, guard calls.
- `compilation-and-abi`: `attached` and `capture` in the ABI.
- `formatting`: `attached` and `capture` printed as written.
- `ui`: attached handler parameters, handlers and R1.
- `execution`: states owned by data run and free once; retired cells before the allocation check.
- `example-apps`: the todo app's per-item state.
- `system-invariants`: invariant 6.

## Impact

- `crates/prism-parser`, `crates/prism-ast`, `crates/prism-abi`, `crates/prism-formatter`: `attached` and `capture` modifiers.
- `crates/prism-typechecker`: `checker/state.rs` (state objects, places, removal of `E4033`, `read`, `edit`), `intrinsics.rs` (catalog), `captures.rs` (attached roots, weak captures), `remember` typing, function-type conversions, capture-function table.
- `crates/prism-typed-tree`: `CaptureMode::Weak`, the attached ownership reason.
- `crates/prism-mir`: `build/state.rs` (guards, operations over borrowed state objects), `build/closure.rs` (weak captures), remember lowering.
- `crates/prism-borrowck`: `escapes.rs` (`E5010`, `E5007` help), guard temporaries.
- `crates/prism-compose-transform`: `stability.rs` (borrowed states), `rewrite.rs` (remember of owned values, kept state parameters, restart views).
- `crates/prism-compose`: `state.rs` (retirement, deferred frees, guard flags, weak upgrade; owners removed), `composition.rs` (release order).
- `crates/prism-codegen`: layouts, drop glue, entries, contracts. `crates/prism-ui`: attached registration externals.
- `crates/prism-library`: `prism/compose/runtime.prism`, `foundation.prism`, `material3.prism`, `ui.prism`.
- Fixtures under `crates/prism-cli/tests/fixtures/`; existing state fixtures rewritten; compile-timing shapes.
- `examples/apps/todo/*`, `examples/run/compose/counter.prism`, `examples/run/main.prism`.
- `openspec/specs/diagnostics/spec.md` (`E4033` retired, `E5010`, rows of `E4021`, `E4029`, `E4031`, `E5007`), `openspec/specs/system-invariants/spec.md`, `AGENTS.md` §2 and §3, `openspec/config.yaml`'s context, `docs/architecture.md` §5 (D8, D12, D13, D15).
