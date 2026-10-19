# Design: owned-state

## Context

See `proposal.md` for motivation. This change implements three items the user agreed in the post–Milestone 2 review (`docs/milestone-2-review-notes.md`, "Agreed changes"): **A2′** ("`MutableState<T>` becomes a move-only owning object (contents in a stable runtime block, freed on drop), usable directly as a struct field ... `remember` may keep owned values ... Escaping closures that capture a state place capture an automatic generational `Copy` handle ... This amends system invariant 6 ... and dissolves most of state-handles D5 (`read`/`update`/`edit`, E4033), since owned state borrows normally. Snapshots: observation only for now"), **A2‴** (R1, R2 and R3: no stale-state errors in UI code; reference counting rejected as hidden ARC) and **L5** of A4 (the weak-capture behaviour is declared by the library, not hard-coded on a type name). A2, a separate `OwnedState<T>` type, is superseded by A2′. It is item 4 of "After Milestone 2: the review round" in `ROADMAP.md`. Decisions the user took are marked **(user direction 2026-10-20)**; every other decision the user might take differently is **(autonomous)** and logged in `docs/decisions-for-review.md`. Compile speed is a primary criterion (AGENTS.md §5): each decision states its compile-time cost.

**Baseline.** This packet builds on `path-captures` (item 3), which another agent is implementing and which is not archived yet. Its delta specs are taken as applied: closures capture paths (`todo.done`, `*count`), owning closures copy `Copy` paths and widen the others to their binding, and its D11 leaves "a fourth `CaptureMode`" for state objects to this change. The MODIFIED blocks of compose "Skipping Composables With Stable Parameters" and references "Values Written Into a State Escape" are based on `path-captures`' versions. The first task re-bases everything on the main specs once `path-captures` is archived.

Current state (main specs and code):

- **State is a `Copy` handle** (`state-handles` D3; compose, "State Handles Are Copy Words"): `State<T>` and `MutableState<T>` are one word naming a cell of the per-thread arena in `crates/prism-compose/src/state.rs` (32-bit index, 32-bit generation, the value in a block that never moves, a flag, readers and a release mark). The catalog (`IntrinsicStruct::{State, MutableState}` in `crates/prism-typed-tree/src/types.rs`) makes them `Copy` for every `T`, with no drop glue.
- **Cells have owners** (D11, D12, D15 of `state-handles`/`compose-architecture`): a cell created in a `remember` window belongs to its slot, one created by a handler or side effect to its composition, any other to the program; a dump lists slot-owned cells.
- **`s.value`** (D5): a copy for a `Copy` `T` anywhere; a place for other `T` only in composable contexts, where borrows hold no loan until the body ends (references, "Borrows of a State's Value Hold No Loan"); outside them `E4033` and the closures `read`, `update`, `edit`, which hold the cell's run-time flag (`begin_read`, `take`/`put`, `begin_edit`).
- **`remember<T : Copy>`** (`compose-architecture` D8) returns a copy, so non-`Copy` data lives in a state.
- **Handlers** are `escaping` parameters (`Button(onClick: escaping () -> Unit)`), moved into `prism-ui`'s handler table (`crates/prism-ui/src/handlers.rs`), owned by the node that claims them; one whose environment is one word owning nothing is interned.
- **Release order** at the end of a pass (compose-runtime D7): children before parents; within a group its slots last to first, then its restart function, then its node.

## Goals / Non-Goals

**Goals:**
- State inside data: `struct Todo { val id: Int, val title: String, val done: MutableState<Bool> }` in a plain `List<Todo>` held by a remembered `MutableState<List<Todo>>`; rows writing only their own state.
- Kotlin-like code: `count.value += 1` on a `val`; `todos.value.add(x)`, `for (t in todos.value)` in handlers and plain functions; `Checkbox(onCheckedChange = { todo.done.value = it })` on a borrowed `todo`.
- No stale-state errors in UI code (R1, R2, R3), with the generational check as a safety net, and memory safety everywhere.
- No hidden ARC, no garbage collector, no snapshot system.
- A capture rule decided syntactically and declared by the library (L5), at constant cost per captured path.
- A clear seam for `strong-skipping` (A1).

**Non-Goals:**
- Snapshot isolation, `Snapshot.withMutableSnapshot`, multithreaded state (user: later).
- Weak captures of holder structs (D21), `Clone` for states, assignment through `?.`, `mutableStateListOf`, `derivedStateOf`, `by` delegation (C3).
- Identity for borrowed parameters other than states (`strong-skipping`).
- A version counter per cell (D18, `strong-skipping` adds it).

## Decisions

### D1. `MutableState<T>` is an owned state object (user direction 2026-10-20)

`MutableState<T>` and `State<T>` become move-only intrinsic structs: neither `Copy` nor `Clone`, invariant in `T`, one word naming a cell whose value lives in a block that never moves (the existing arena, `state-handles` D8). Moving a state object moves the word only, so borrows of its value and weak handles of it are unaffected by moves. Its drop glue retires the state (D7). `mutableStateOf(v)` creates a cell and returns its owner. `==` of state objects compares identity (the words), as Kotlin's `SnapshotMutableStateImpl` does not override `equals`, so a struct holding a state compares items by identity of their states.

A state object can be a local, a parameter, a struct or enum field, a list element, a top-level value, a remembered value or the value of another state, like any value.

**Compile-time cost.** None new: the catalog's markers change; the rest is ordinary ownership checking.

Rejected:
- **A separate `OwnedState<T>` beside `Copy` handles** (A2): superseded by the user's A2′; two state types for one concept.
- **`Clone` for states** (a new cell with a clone of the value): catalog markers hold for every type argument, so it would need conditional intrinsic markers; and Kotlin copies references, so neither meaning is obviously right. A follow-up.
- **Value equality for `==`**: comparing contents of states is a deep comparison that no Compose code relies on.

### D2. `value` is written through any access to the state object (autonomous)

`s.value = v`, `s.value += 1` and `update` are accepted on a `MutableState<T>` reached by value, through a `val`, or through any reference, shared ones included, as Kotlin writes `val count = remember { mutableStateOf(0) }; count.value++` and passes `MutableState<Int>` to functions that write it. A state object is a cell with interior mutability; the type `MutableState` in a signature says the callee may write, and `&State<T>` says it will not (D5). Writes stay rejected in composable contexts (`E4031`) and during passes (run time), as today.

**Compile-time cost.** None: the assignment check reads the declaration found (`intrinsic var`).

Rejected:
- **Inherited mutability** (writes need a `var` binding or `&mut`, as for fields): `val count = mutableStateOf(0); count.value += 1` and the counter's handler would be errors, `remember` would have to give `&mut`, and every function writing a state would take `&mut MutableState<T>` and be called with `&mut`. Kotlin-likeness first (user); the cost is that a write through a shared borrow is not visible at the call, a deliberate exception to the reference-access principle for state, as `Cell` is in Rust.

### D3. A state's value is a place; outside composable contexts its borrows hold the state's flag until their scope ends (autonomous)

For a non-`Copy` `T`, `s.value` is a place of the state object in every context, borrowed normally: the borrow checker sees a borrow of `s.value` as a borrow through `s` (a loan of `s`'s place, or tied by elision to the reference `s`), so `val items = &s.value; take(s)` is `E5005` and `compose fun items(s: &State<List<Int>>): &List<Int> = &s.value` is valid by rule 1. Moving it out or taking it apart stays `E4017`.

Static checking cannot see two references to one state (`f(s, s)` with two `&MutableState` parameters), two weak handles of one cell in a closure, or a write through one alias while another borrows (D2). So:

- **In composable contexts nothing is checked at run time**, as today: no state is written and no cell is freed while a pass runs (D7), so a borrow is valid until the body ends. A read still checks that no exclusive flag is held (by program code around a `recompose()`).
- **Outside composable contexts**, a shared borrow of a non-`Copy` value takes the state's flag shared, and a mutable borrow, a `mut fun` call on the value or a part write takes it exclusively, through the existing runtime calls (`begin_read`/`end_read`, `begin_edit`/`end_edit`), which become guards. A guard ends where the temporary scope of the borrow's statement ends, or where the block of the `val`/`var` it initializes ends, exactly as MIR already ends borrowed temporaries ("Borrowed Initializer Temporaries Live as Long as Their Binding"), on every path. A conflicting access through another alias is the existing `state accessed while borrowed`. `Copy` reads and whole writes take nothing past their call.
- **The reference cannot outlive its guard**: MIR gives the begin call two input borrows, the state object and the guard temporary, and ties its result to both, so a reference stored past the guard's scope is `E5006`, and the state object stays borrowed (`E5005` on a move).

This dissolves `E4033` and the closures `read` and `edit` (D4). It is RefCell's discipline with lexical guards, which needs no help from the borrow checker to place releases.

**Compile-time cost.** Constant per access: a marker lookup and the composable flag in the type checker (as today), two calls and one temporary in MIR for a guarded borrow. No new analysis.

**Run-time cost.** Two runtime calls per guarded borrow (begin, end), each a handle validation and a flag update (about the 13–15 ns of a read today); nothing in composables.

Rejected:
- **No run-time flags outside composition**: unsound through aliases (`for (x in a.value) { b.value.add(x) }` with `a` and `b` one state reallocates the list being iterated).
- **Flags released at the borrow's last use**: needs the borrow checker to emit releases into MIR (rejected by `state-handles` D5 for the same reason); lexical guards cost a slightly longer hold, never unsoundness.
- **Keeping `read`/`edit` closures as the only access outside composables** (today): not Kotlin, and the user asked for normal borrows.
- **One guard per function body**: a borrow in one statement would block a write in a later one.

### D4. `read` and `edit` are removed, `update` stays, `E4033` is retired (autonomous)

`s.read { v -> f(v) }` is `f(s.value)` and `s.edit { v -> v.add(x) }` is `s.value.add(x)` (D3), so both go, as Prism is unreleased (AGENTS.md §5: breaking changes are fine). `update { }` stays: it moves the value out, calls the transform and moves the result in, which a place cannot do (`todos.update { it.filter { !it.done.value } }`, since `filter` consumes), and it is `MutableStateFlow.update`'s shape. `E4033` (`StateValueOutsideComposable`) is retired, never reused (diagnostics, "Stable Diagnostic Codes"). `E4031` keeps covering mutable borrows, `mut fun` calls and part writes of a state's value in composable contexts.

Rejected: deprecating `read`/`edit` (no users to migrate); removing `update` (no other way to transform a non-`Copy` value by value).

### D5. `State<T>` stays a read-only face of the same object (autonomous)

`State<T>` remains a separate intrinsic type with the same word and drop glue. `MutableState<T>` converts to `State<T>` by value (an owned read-only state, which `derivedStateOf` will return later) and behind a shared reference (`&MutableState<T>` as `&State<T>`), as a subtype step that keeps the representation, and an argument of type `MutableState<T>` is borrowed implicitly for a `&State<T>` parameter. Composables that only read take `&State<T>` (Compose's `State<T>` parameter), so `Label(count)` reads as in Kotlin. Lookups on `MutableState` still fall back to `State`.

**Compile-time cost.** The existing constructor comparison in assignability, now also under `&`.

Rejected: **`&MutableState<T>` as the read-only view** (D2 makes it writable); **dropping `State<T>`** (a parameter type that says "reads only" is the point of `state-handles` D3); a wrapper struct (a second word for nothing).

### D6. `remember` keeps any borrow-free value: a copy when `Copy`, a borrow otherwise (user direction 2026-10-20 for keeping owned values; autonomous for the result type)

`remember<T>` loses its `Copy` bound (revising `compose-architecture` D8). Its result is typed by the catalog's rule, as `value` is: a copy for a `Copy` `T` (unchanged: `val id = remember { 7 }` is an `Int`), and a shared borrow of the slot's value otherwise (`val count = remember { mutableStateOf(0) }` is a `&MutableState<Int>`, used exactly as before through auto-dereference). The borrow holds no loan and is valid until the body ends, as borrows of top-level `val`s are, because slot blocks never move and are released only after the pass (compose-runtime D6, D7); so a later composable call does not conflict with it, and a composable may return it. The slot drops the value when it is released (a removed group, a changed key, a disposed composition, a reset after a reload), which retires the states it owns (D7). `T` must still hold no borrow (`E4022`), and keys stay `Copy` and borrow-free.

**Compile-time cost.** One marker lookup per `remember` call to pick the rule.

Rejected: **`&T` always** (`val id = remember { 7 }` would be a reference, unlike Kotlin); **`&mut T`** (a write during composition); **moving the value out once** (one owner: the slot); a separate `rememberOwned`.

### D7. Dropping a state retires it; its cell is freed after the next pass (user direction 2026-10-20: R2; details autonomous)

Retiring a state (its owner's drop glue calls `prism_compose_state_retire`) marks the cell retired, invalidates and forgets its readers (a reader showed data that is gone), and makes `WeakReference.get()` return `null`. The value is **not** dropped yet: borrows taken before the retirement (a guard in an outer frame, a borrow during the pass that dropped it) and weak handles in nodes stay valid. Reads and writes through weak handles keep working until the cell is freed, and a write to a retired cell invalidates nothing; its value is lost with the cell, as a write to a Kotlin state object no one shows any more.

The cell is freed, its value dropped, **at the end of the first pass on the thread that starts after its retirement**, after that pass's releases and before its side effects. By then every composable that received the removed data has recomposed or been removed (the data's container was written, invalidating its readers; with `strong-skipping` every changed borrowed argument recomposes too), so each node's handler was replaced or released (R1, D8). A retirement during a pass (a slot released at its end, a composable dropping a moved-in state) waits for the next pass. When no composition is live on the thread, a retired cell is freed at once, and disposing the last composition frees every retired cell. A cell that a guard or `update` holds when its free is due is freed when the last of them ends (as today for released cells). Under `--check-allocations`, every retired cell is freed after the top-level values are released and before live compositions are counted.

The runtime keeps a list of retired cells with the thread's pass count at retirement; the end of a pass frees the entries recorded before it started.

**Compile-time cost.** None. **Run-time cost.** Retiring is O(readers) plus a push; a pass frees its due cells in time proportional to their number; nothing for passes with none.

Rejected:
- **Freeing at once**: a node's handler that captured the state would hit a freed cell on the next event before the pass that replaces it.
- **Freeing at the end of the current pass**: a state dropped during a pass may still be named by handlers of nodes that pass did not replace.
- **Tracking per composition**: matters only when compositions share state, which apps (one `runApp`) do not; the generational check covers the rest.
- **Reference counting handlers or cells**: hidden ARC (user, A2‴).

### D8. A removed group releases its node before its restart function and slots (user direction 2026-10-20: R1; order autonomous)

Within a group, the end-of-pass release now drops its node first (with the handlers the node owns, removed from the handler table), then its restart function, then its slots last to first; children are still released before parents. So a removed group's nodes and handlers are gone before the values its slots own are dropped, and with D7 no node ever holds a handler whose state is freed. Nothing runs program handlers during the release, so the order is a guarantee for the runtime's own invariants (no live node or handler entry naming a freed cell, at any point) rather than an observable race, and it is what "Release Order at the End of a Pass" now states.

**Compile-time cost.** None. Rejected: the old order, which is safe only through D7 and leaves R1 implicit.

### D9. `attached`: a parameter kind for function values the composition keeps (user direction 2026-10-20: R3, "a parameter kind distinct from plain `escaping`"; syntax autonomous)

A parameter of function type may be marked `attached` where `escaping` may be (`onClick: attached () -> Unit`, `content: attached compose () -> Unit`, `f: attached (() -> Unit)?`), never both. It means: the callee keeps the value only where the composition keeps it, and releases it no later than the group that passed it: the handler table (owned by a node), a kept-argument slot, the side-effect queue, a composition's root content. It is part of function types, rendered and hashed like `escaping` (Phase 1 records it).

- **Values.** An `escaping` value (an owning closure holding no weak handle, an `escaping` parameter) may be passed to an `attached` parameter. An `attached` value may be called, borrowed, captured by a closure that does not escape or is itself given to an `attached` parameter, and passed to an `attached` parameter; any other escape is `E5010` (`AttachedFunctionEscapes`, new, borrow checker). A borrowing value is `E5007`, as for `escaping`.
- **Types.** A function type converts to one where a parameter unmarked in it is `attached`, or one `attached` in it is `escaping` (contravariance: a callee that keeps nothing accepts anything; one that may keep a value anywhere accepts what an attached callee accepts). Nothing else converts.
- **Stability.** An `attached` function parameter of a composable is stable as an `escaping` one is, kept as a keepable copy.
- **The library**: `onClick` of `Button` and `Modifier.clickable`, `onCheckedChange` of `Checkbox`, `onValueChange` of `Modifier.toggleable`, `TextField` and `BasicTextField`, `SideEffect`'s `effect`, `content` of `composition`, `Composition.setContent` and `runApp`, and the handler parameters of `prism_ui_register_click`, `_toggle` and `_text`. Composable content lambdas stay unmarked (they borrow).

The name says what the kind is: a function value attached to the composition, released with it. Compose has no such concept, since its closures hold GC references.

**Compile-time cost.** A flag per parameter, one more root kind in the capture pass's flow graph (D10), and a test per escape position in the borrow checker's existing escape walk.

Rejected names: `handler` (also content and effects), `bound` (the specs use "bound" for type-parameter bounds), `composed` (`Modifier.composed`, which the user banned), `retained` (reads as reference counting), `kept` (the specs' "kept arguments"), `escaping(composition)` (ambiguous with a parenthesized function type). Rejected designs: **implicit weak captures for every escaping closure** (stale errors anywhere, A2‴); **inferring which parameters the composition keeps** (whole-program).

### D10. Closures given to `attached` parameters capture states through weak handles (user direction 2026-10-20: R3; details autonomous)

In `captures.rs`'s existing flow graph (`escaping-closures` D1, D2), positions passing a value to an `attached` parameter become roots of their own kind. A closure that owns its captures and whose owning roots are all `attached` is **attached-owned** (recorded as the reason "passed to an attached parameter here"). For such a closure, each captured path (`path-captures` D1) whose type has a capture function (D12) is captured through a weak handle, a fourth `CaptureMode::Weak`, when the same conditions as `path-captures` D3's copies hold: its binding is a `val`, it dereferences no `&mut` reference, and every field it reads after a reference is a `val` field (nothing later can re-point it: `Todo.done` must be a `val`). This includes a whole binding (`composition { Screen(count) }` in `main`, where `count` stays `main`'s and is still written by `main`). The weak handle is made where the closure is built by the capture function's operation (one word copied from the state object), holds no borrow and owns nothing, so the closure stays keepable, its environment compares by words, and a one-handle environment is inline and interned by the handler table. Every other path follows `path-captures` D3 (copied or widened: a state through a `var` field or a `&mut` widens to its binding, `E5007` for a reference).

In the closure's body, a use of the path is a place that views the capture's word as the state object (a projection in MIR, below), never moved (a closure never moves its captures, `ownership`) or dropped. Each state operation validates the handle, so a use after the cell was freed is `stale state handle` at the use: the safety net. R1 and D7 keep UI code from reaching it; a closure given to an `attached` parameter in a plain function, capturing a local state that the function drops while the composition lives, can still reach it, which is a bug of that program, reported, never memory-unsafe.

In MIR, a weak capture is a capture local of type `WeakReference<S>` built by the `state weak` operation from a borrow of the path's place, and a use of the path in the closure's body is the place `view(local)`, a projection that reads the word as the state object `S` and is never moved or dropped (the borrow checker treats it as a shared dereference). `get()`'s result is the same view behind a reference.

An owning closure that is not attached-owned keeps `path-captures`' rules: an owned state binding is moved into it (`store { s.value += 1 }` owns `s`), and a state used through a reference widens to the reference, `E5007`, whose help now suggests an explicit weak reference: "make a weak reference before the closure: 'val countRef = count.weak()', and use 'countRef.get()' inside it" (D11).

**Compile-time cost.** In the existing linear pass: one more root kind in the graph search, and for each captured path of an attached-owned closure one capture-function lookup by type constructor (a hash lookup) on top of `path-captures` D3's constant checks. A compile-timing shape, `weak-captures`, covers `n` handlers each capturing several state paths through references.

Rejected: **moving owned state bindings into attached closures** (the `main` + `composition` pattern of every state test would lose its state); **weak captures for every owning closure** (A2‴'s errors anywhere); **reference-counted handlers** (hidden ARC, user).

### D11. Explicit weak references (user direction 2026-10-20: `count.weak()`, `ref.get()?.value`; details autonomous)

`/prism/compose/runtime` declares `WeakReference<T> : Copy`, one word, and `fun <T> WeakReference<T>.get(): (&T)?`. `s.weak()` copies the state's word into a `WeakReference<MutableState<T>>` (or `<State<T>>`) without keeping the state. `ref.get()` returns `null` once the state is retired, and otherwise a shared borrow of a state object that reaches the same state: the reference's own word viewed as the state object, which is why the result is tied to `ref` by elision rule 2 and not to the owner, whose address a handle does not know. Every access through it validates the handle, so a cell freed while the borrow lives (only possible if code holding it runs a pass) is `stale state handle`, not a dangling read. Kotlin's name and shape: `WeakReference(x).get()`, nullable after the referent is gone, here deterministically.

`ref.get()?.value` must read through `?.` to an intrinsic property, and `ref.get()?.update { it + 1 }` call through it; the first apply task checks both and makes them work if the safe-navigation lowering only reaches fields and calls. Assignment through `?.` (`ref.get()?.value = 1`) is not in Prism and stays out of scope.

**Compile-time cost.** Two catalog entries. **Run-time cost.** `weak()` copies a word; `get()` is one call.

Rejected: a closure API (`ref.with { s -> }`, less Kotlin-like); returning the owner's address (unknown to a handle); two weak types `WeakState`/`WeakMutableState` (one generic type, as Kotlin).

### D12. The library declares how a type is captured: `capture fun` (user direction 2026-10-20: L5; syntax autonomous)

`/prism/compose/runtime` declares

```
public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>
public intrinsic capture fun <T> State<T>.weak(): WeakReference<State<T>>
```

`capture` is a new contextual modifier directly before `fun`, after `intrinsic`, on an extension with a receiver. It marks the function attached closures use to capture a value of the receiver's type. The type checker builds a table from each module's declarations (the ABI records the flag), keyed by the receiver's type constructor; the capture pass asks "does this path's type have a capture function?" through that table, never by the type's name, so nothing in `captures.rs` mentions `MutableState`. The catalog lists capture functions like other intrinsics: one per type, an intrinsic extension declared in the type's module, with a shared receiver, no parameters, no type parameters but the receiver's, returning `WeakReference` of the receiver type; anything else is `E4029`. Only the bundled library may declare one (`intrinsic` is library-only, `E3002`), because the weak view relies on the runtime's generational cells. When `library-in-prism` writes state in Prism over externals, the declaration stays; the view stays a compiler contract until the language can express it.

**Compile-time cost.** A table entry per capture function when a module is loaded; an O(1) lookup per captured path of an attached-owned closure and per composable parameter type (D13).

Rejected: **testing `IntrinsicStruct::MutableState` in the capture pass** (what L5 forbids); **a marker on the struct** (`struct MutableState<T> : Weak`): markers take no arguments and would not name the weak type or the function; **an annotation**: Prism has no attribute syntax.

### D13. Borrowed states are stable parameters, compared by identity (autonomous)

A parameter of type `&State<T>` or `&MutableState<T>` (a shared borrow of a type with a capture function) is stable. The composable keeps its weak handle in the slot and compares it with the kept one by word: the same state object skips, another runs the body. This is Compose's semantics for `State` parameters: a change of the value invalidates the scopes that read it, not the callers. The restart function passes the composable a borrow of the kept slot's word viewed as the state object (slot blocks are stable during the pass). Without this, every composable taking a state, skippable while states were `Copy` handles, would stop being skippable and a restart scope until `strong-skipping`, regressing `Label(count)`-style scopes, the hot-reload restart scenarios and `examples/run`'s counter. `strong-skipping` (A1) generalizes identity to every borrowed parameter.

**Compile-time cost.** One capture-function lookup per parameter type in `stability.rs` (`is_stable`), constant. **Run-time cost.** One word compare per kept state parameter.

Rejected: **waiting for A1** (a regression of skipping meanwhile); **comparing values** (deep, and wrong: reads track the contents).

### D14. Equal writes still invalidate nothing; mutable borrows and `update` always invalidate (autonomous)

`value =` and compound assignments keep `state-handles` D10's equality check through the cell record's equality glue (Jetpack's `structuralEqualityPolicy`). An exclusive guard (a mutable borrow, a `mut fun` call or a part write of the value) invalidates the readers when it is taken, like `edit` did, since its effect cannot be compared without a clone; `update` always invalidates. Retirement invalidates too (D7).

**Compile-time cost.** None.

### D15. `E4032` and the run-time creation check stay; remember windows only allow creation (autonomous)

A state created on every pass is always a bug (Compose's lint reports it), so `mutableStateOf` in a composable context stays `E4032`, and creating a state during a pass outside a `remember` calculation or a top-level initializer stays the run-time error `state created during composition outside remember`. The `begin_remember`/`end_remember` windows and the program markers of top-level initializers remain, but they only allow creation: they no longer attach cells to an owner (D16).

Rejected: **dropping both checks** (sound now, since an unremembered state is just dropped, but the bug is silent); a static rule for plain functions (whole-program).

### D16. A cell belongs to its state object; cell owners and cells in dumps go (autonomous)

The slot, composition and program owners of `state-handles` D12 (`compose-architecture` D15) are replaced by ownership: the state object owns its cell, and whatever owns the object (a slot, a list, a local, a top-level value) decides when it is retired. The runtime no longer knows which slot holds which cell, so composition dumps list slots (`slot MutableState<Int>`) and no `cell` lines (`state-handles` D20). The live-cell count of the allocation check stays.

Rejected: keeping the remember windows' attachment just for dumps.

### D17. Hot reload needs no new rule (autonomous)

A slot holding a state object keeps it across a reload when its type fingerprint is unchanged (`MutableState<Int>` covers `Int`; a `List<Todo>` covers `Todo`'s fields, `done: MutableState<Bool>` included) and is reset otherwise, which drops the value and retires its states (D7). A closure's slot contract records each capture's mode and type fingerprint, so a reload that changes a capture from weak to copied or moved gives the closure a new slot, as any change of captures does. Every cell access passes the value type's record, so a handle never reaches a cell of another type across a reload. Top-level states keep their storage by symbol, fingerprint and initializer hash, as before.

### D18. The seam for `strong-skipping` (autonomous)

A1 needs a stable identity for borrowed arguments. This change provides it for the three kinds of borrow it creates: a **state object** is identified by its cell handle, which moves with it and survives being passed by reference (D13 already compares it); a **remembered value** by its slot block's address, stable while the slot lives, and unchanged while it lives, since a remembered value is borrowed shared and changes only through the states it holds; a **state's contents** by (handle, version), where every write, `update` and exclusive guard already goes through one runtime call (D3, D14) at which A1 can bump a per-cell version. This change adds no version counter and no identity for other borrows.

### D19. System invariant 6 is amended (user direction 2026-10-20)

"First-Class Compose Primitives and Native Hot Reload" now reads: native support for declarative UI via the `compose` keyword and state objects (`MutableState<T>`) owned like any value, whose contents live in runtime cells that never move; generational state handles are an implementation detail of the runtime and of the closures the composition keeps. A scenario pins state owned by data with no GC or reference count. AGENTS.md §2's one-line summary and `openspec/config.yaml`'s context line change with it (tasks 13.2).

### D20. The examples (autonomous)

- **Counter**: its source does not change. `val count = remember { mutableStateOf(0) }` is a `&MutableState<Int>`, and `Button(onClick = { count.value += 1 })` captures `*count` through a weak handle, one interned word as before.
- **Todo** (`examples/apps/todo/`): `model.prism` declares `package struct Todo { val id: Int, val title: String, val done: MutableState<Bool> }`; `Filter.shows` reads `todo.done.value`; operations are functions over the states they use, `addTodo(todos: &MutableState<List<Todo>>, draft: &MutableState<String>, nextId: &MutableState<Int>)`, `removeTodo(todos, id)`, `activeCount(todos: &State<List<Todo>>)`, and `sampleTodos(count)` builds items with their own states. The `TodoState` holder goes (D21). `screen.prism`: `TodoApp` remembers `mutableStateOf<List<Todo>>(listOf())` and calls `TodoScreen(todos)`, which remembers `draft`, `filter` and `nextId`, keys each row by `todo.id`, and shows `LeftCount(todos)`; `TodoRow(todo: &Todo, onRemove: attached () -> Unit)` shows `DoneBox(todo.done)`, the title and a `Remove` button calling `onRemove`; `compose fun DoneBox(done: &MutableState<Bool>) { Checkbox(checked = done.value, onCheckedChange = { done.value = it }) }` is skippable (D13), so it is a restart scope that alone reads `done`: a toggle writes only the item's state and recomposes one checkbox, not the list (F3), while `LeftCount`, a restart scope of its own, recounts. `large.prism` remembers `mutableStateOf(sampleTodos(1000))`. The session file `editing-the-row-layout-keeps-the-list.screen.prism.v2` follows.
- **`examples/run/compose/counter.prism`**: `CounterLabel` and `Counter` take `&State<Int>` (still skippable by D13, so "a click recomposes the label alone" holds), and `clicker(count: &MutableState<Int>): () -> Unit` returns `{ ref.get()?.update { it + 1 } }` over `val ref = count.weak()`, the explicit form for a handler that is not attached.

Rejected: keeping a `TodoState` holder of states (its handlers could not capture it, D21); `DoneBox` left out until A1 (F3's toggle would still recompose every row).

### D21. Holder structs are not captured weakly (autonomous; non-goal)

A handler calling a method on a borrowed holder of states (`{ state.add() }` with `state: &TodoState`) uses the path `*state`, whose type has no capture function, so it widens to the reference: `E5007`. Handlers name the states they use (`{ addTodo(todos, draft, nextId) }`), as Compose samples do with hoisted states. Possible follow-ups, for the user to choose: a declared marker for structs whose fields are all states or `val` `Copy` values, captured as a weak image of the struct; or weak references to remembered slots. Both need a design of their own (a marker derived from fields is the kind of inference the compile-speed principle avoids).

### D22. Diagnostics (autonomous)

- **`E5010` `AttachedFunctionEscapes`** (new, next free `E5xxx`): an `attached` value escaping elsewhere (D9). Its own fix ("declare the receiving parameter `attached`, or call it"), different from `E5007`'s.
- **`E4033`** retired, never reused (D4).
- **Rows updated**: `E4021` (no longer `remember` values), `E4029` (capture functions), `E4031` (no `edit`; mutable borrows of a state's value), `E5007` (attached as well as escaping positions; the weak-reference help, D10), `E2001` (`attached` and `capture` misuse).

### D23. Costs (autonomous summary)

- **Compile time**: constant per state access, per captured path and per parameter type; one root kind more in the capture pass's linear graph search; a table built per loaded module. No pass is added. Shapes: `guarded-state-accesses` (n statements borrowing and writing state values in a plain function) and `weak-captures` (n attached handlers each capturing three state paths through references) beside `state-accesses`, all near-linear.
- **Run time**: a guarded borrow costs two runtime calls; a retirement O(readers); a pass frees its due cells linearly; a weak capture copies a word; a kept state parameter compares a word. No pass allocates more in steady state: the retired list is reused.
- **Memory**: a retired cell lives until the end of the next pass.

## Risks / Trade-offs

- [The change is large and breaking: every state fixture and scenario with a by-value `State` parameter, `read`, `edit` or `E4033` changes] → the delta specs rewrite them one by one; the tasks list the fixture directories; scenarios removed with their requirements are named in REMOVED blocks.
- [Writes through shared borrows hide mutation at call sites (D2)] → limited to state objects, whose type says so; `&State<T>` documents read-only callers.
- [Guards hold a flag until the end of a statement or block (D3), slightly longer than the last use] → conflicts can only come from aliases, which are rare; the error names the access.
- [Tying a guarded reference to its guard in the borrow checker may need a built-in signature with two input borrows] → if it proves hard, the fallback is to reject a guarded borrow that initializes a binding (statement scope only) with `E5006`, recorded as a Baseline update.
- [A closure given to an `attached` parameter outside a composable can outlive a local state it captured weakly (D10)] → `stale state handle` at the use, never memory-unsafe; documented in the `attached` rationale.
- [Several compositions sharing state: a cell retired by one composition's data may be freed by another's pass (D7)] → the generational check; apps run one composition.
- [Composables taking borrowed holders or items (`TodoRow(todo: &Todo)`) are still not skippable until `strong-skipping`] → `DoneBox` shows the state-identity path now; A1 next.
- [`?.` to an intrinsic property may not lower today (D11)] → checked in the first group, fixed in this change if needed.
- [Memory held one pass longer by retired cells] → bounded by what the previous pass dropped.

## Migration Plan

Unreleased language: no compatibility layer. Programs change as follows: by-value `State<T>`/`MutableState<T>` parameters become `&State<T>`/`&MutableState<T>` (call sites unchanged by implicit borrows); `s.read { v -> f(v) }` becomes `f(s.value)`; `s.edit { v -> g(v) }` becomes `g(&mut s.value)` or a method call on `s.value`; copies of handles become borrows or `s.weak()`; user composables taking handlers that capture states mark them `attached`; `remember` of a non-`Copy` value now returns a borrow. Build-cache entries are keyed by the compiler's hash, so nothing cached is reused. Rollback is a revert of the change's commits.

## Open Questions

- The exact wording of `E5010`, the `E5007` weak-reference help and the note for a state behind a `var` field may be tuned during implementation.
- Whether the run-time cost of guards in handlers that only read a `List`'s size is worth an optimization (a guard-free read of `Copy` projections such as `todos.value.size`): measure in `app_frame_timing` first.
