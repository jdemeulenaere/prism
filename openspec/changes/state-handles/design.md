# Design: state-handles

## Context

See `proposal.md` for motivation. This is item 29 of "Then: the road to Milestone 2" in `ROADMAP.md`, the state layer of `docs/architecture.md` §3 ("Reactive Tree Copy State Handles") and §5. It implements `compose-architecture` D12 (two handle types, one word each), D13 (borrowing state contents during composition, `read`/`update`/`edit` elsewhere), D14 (state is read-only during composition), D15 (cells belong to their creator) and the state half of D16 (no snapshots; reads register, writes invalidate, equal writes do not). It fills the seams `compose-runtime` D1 and `compose-transform` D1 and D23 left for it. Each decision is repeated below where this change refines it. The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Fast compilation is a primary criterion (user, 2026-10-19), so each decision states its compile-time cost next to its Kotlin-likeness, and its run-time cost where it has one.

**Baseline.** This change was proposed while `collections` was being implemented, and `standard-library`, `compose-runtime`, `compose-transform`, `instance-cache` and `hot-reload` were proposed but not implemented. It needs `compose-transform` archived, which brings the rest:

- `compose-runtime`: the `prism-compose` crate with its per-thread state (`thread.rs`), the group and slot arenas, `Handle` (a 32-bit index and a 32-bit generation in a `u64`, retired at the maximum generation), deferred releases at the end of a pass with "`state-handles` attaches cells to slots, and they are released after the slot's value" (its D7), the current scope of the active pass as a Rust API (its D1), `invalidate` in O(1) and doing nothing for removed scopes (its D8, D12), invalidations during a pass going to the next one (its D4), `COMPOSE_FUNCTIONS` and `UnitContext::entry_point`, the composition dump, `FunctionValue` in `prism-runtime/src/function_value.rs`, and the finish report hook `set_finish_report` (its D13).
- `compose-transform`: `intrinsic struct` and its catalog (its D16), `/prism/compose/runtime` in the prelude, `remember` lowered as `changed`/`Remembered`/calculation/`set_slot` (its D12), `Callee::Composer(ComposerOperation)` and `MirPhase::Composed` (its D19), `UnitContext::slot_type(ty)` emitting slot type records with fingerprints, drop and equality glue (its D20), stability by declared markers, so handles are stable parameters (its D7), restart scopes exactly at skippable composables and the root (its D10), `c.recompose()`, `c.setContent`, `c.dump()`, and the composable-context flag also off in default values (its D17).
- `standard-library`: the intrinsic catalog (`crates/prism-typechecker/src/intrinsics.rs`, `E4029`), `intrinsic val` extension properties, intrinsic extensions as members (its D7), `Callee::Builtin` calls of intrinsics, and `crates/prism-library` with `MODULES` and `PRELUDE`.
- `collections`: `List<T>`, its members, elements as places and `for` over a borrowed list, all used by the todo shapes.
- `escaping-closures` (archived): owned captures, one-word inline environments (a handler capturing one handle allocates nothing), and its D7 constraint: a state's value is written through a call taking the handle by value or by shared borrow, never as a `mut fun` receiver or a field write of the handle, since a handler's owned captures are `val`s.
- `hot-reload`: the 256-bit type fingerprints of slot type records, which cover a handle's type argument, so a remembered `MutableState<Int>` edited to `MutableState<String>` fails the slot's type check and is recreated (§5 "Changing a remembered type resets that state only").
- `instance-cache`: the exhaustive MIR codec, which must encode the new operations.

Every delta is ADDED, because the capabilities it extends (`compose`, `standard-library`) do not exist in the main specs until their changes are archived, and the requirements of the others are new. Compare each with the main specs as those changes leave them at apply time.

What exists today, beyond those packets:

- **Type checker** (`crates/prism-typechecker/src/checker.rs`): `is_in_compose()` and `push_compose_context` track composable contexts; `assignment_target`, `check_assignment` and `check_compound_assignment` build assignments over places; `build_member_access` and `member_type` resolve fields and the read-without-parentheses members (`length`, `size`); `infer_method_call`, `find_extension` and `pass_receiver` resolve and pass receivers; `captures.rs` decides owned captures from owning positions; `types.rs::is_assignable_to` holds subtyping.
- **MIR** (`crates/prism-mir`): `build/place.rs::place` lowers place nodes, a top-level value as `_p = const TopLevelValue(path)` and the place `*_p`; `build/place.rs::statement` lowers `Assign` and `CompoundAssign`; `build/call.rs` lowers calls and built-ins; calls carry `type_arguments` that `instantiate.rs` substitutes.
- **Borrow checker** (`crates/prism-borrowck`): a reference from `Constant::TopLevelValue` holds no loan, and `returns.rs` lets it be returned under any rule.
- **Backend** (`crates/prism-codegen`): runtime errors with locations go through `prism_rt_panic(kind, detail, file, line, column)` from a cold block; list bounds checks set the pattern of an inline test with a located slow path (`collections` D8, D20).

**Baseline update (`standard-library`, applied 2026-10-18).** Names and mechanisms that differ from what this design assumed:
- Intrinsic properties are members with `property: true` (`TypeChecker::has_intrinsic_property`). Their reads are typed by arms of `member_type` and `build_member_access` per receiver type (today `String` and `List`), gated on the member being visible, so `value` on a handle needs its own arm keyed by the member's `BuiltinFunction`; writes (`intrinsic var`) are new.
- The catalog (`intrinsics::CATALOG`) holds functions and properties only, with canonical declaration texts that write `: Unit` and leave defaults out (see `compose-transform`'s baseline update).
- Runtime entry points are looked up through `prism_codegen::externals::entry_point`, not `UnitContext::entry_point`.

**Baseline update (`ui-toolkit` revision, user direction 2026-10-19).** `compose-transform`'s prelude module is now `/prism/compose/runtime` (`crates/prism-library/prism/compose/runtime.prism`, `package prism.compose`), after Jetpack Compose's `androidx.compose.runtime` (`ui-toolkit` D21), so the state declarations are added there and the ABI writes a handle type as `prism/compose/runtime.MutableState<Int>`; every mention below uses that path. `ui-toolkit` keeps its modifiers plain data, whose handlers live in a table of `prism-ui` (its D23 and D25): no modifier creates state cells or remembers anything, so the ownership rules of D11 and D12 need no case for modifiers.

## Goals / Non-Goals

**Goals:**
- Jetpack Compose's state model and surface (`mutableStateOf`, `State<T>`, `MutableState<T>`, `.value`, `+=`) with Prism's single ownership, zero GC and zero hidden ARC.
- Memory safety whatever a program does: no borrow of a cell's value outlives it, no write invalidates a live borrow, no handle reaches another cell, and every misuse that no static rule catches is a runtime error at the access.
- Exactly the reading scopes recompose, and an equal write recomposes nothing.
- Constant compile-time work per state access, and amortized O(1) run-time work per read.

**Non-Goals:**
- Event dispatch, nodes and the frame loop (`ui-toolkit`), hot-reload policy (`compose-hot-reload`), counting live cells as leaks (`reactive-app-verification`).
- `derivedStateOf`, `snapshotFlow`, `mutableStateListOf`, mutation policies, `by` delegation, effects other than `SideEffect`, snapshots and threads (`compose-architecture` D16, D25).

## Decisions

### D1. The boundary with the other compose changes (autonomous)

| Concern | `state-handles` (this change) | Elsewhere |
|---|---|---|
| Handle types, `mutableStateOf`, `value`, `read`, `update`, `edit` | all of it | declared in `compose-transform`'s `/prism/compose/runtime` |
| Cells | the arena, flags, readers, owners, release | slots and deferred releases are `compose-runtime`'s |
| Read tracking | registering the active pass's current scope | the current scope is `compose-runtime`'s |
| `remember { mutableStateOf(...) }` | the window that attaches cells to the slot | `remember`'s lowering is `compose-transform`'s |
| Handlers | a Rust API that runs code as a handler of a composition, and `SideEffect` | `ui-toolkit` dispatches events through that API |
| Recomposing after writes | none: writes only invalidate | `c.recompose()` (`compose-transform`), `runApp`'s frame loop (`ui-toolkit`) |
| Leak counting | program cells released before the check | live cells counted by `reactive-app-verification` |

Rejected: a state-specific "recompose after each handler" call in this change (handlers do not exist yet as a runtime concept; the frame loop of `ui-toolkit` owns that rhythm).

### D2. The API: Kotlin's names, closures for what Kotlin does with aliasing (autonomous)

`/prism/compose/runtime` gains:

```
public intrinsic struct State<T> : Copy
public intrinsic struct MutableState<T> : Copy
public intrinsic fun <T> mutableStateOf(value: T): MutableState<T>
public intrinsic val <T> State<T>.value: T
public intrinsic var <T> MutableState<T>.value: T
public intrinsic fun <T, R> State<T>.read(reader: (&T) -> R): R
public intrinsic fun <T> MutableState<T>.update(transform: (T) -> T)
public intrinsic fun <T> MutableState<T>.edit(editor: (&mut T) -> Unit)
public intrinsic compose fun SideEffect(effect: escaping () -> Unit)
```

`mutableStateOf`, `State`, `MutableState` and `.value` are Jetpack Compose's. `update { v -> ... }` is `MutableStateFlow.update`'s shape. `read` and `edit` have no Kotlin counterpart, because Kotlin reads a list state by aliasing it (`todos.value.size` in a handler) and changes one by building a new list or with `mutableStateListOf`; with single ownership, a borrow of a cell's value outside composition needs a scope the runtime can guard (D6). The todo app's handler is `todos.edit { list -> list.add(Todo("x")) }`: `update` replaces the value and `edit` changes it in place, so the ROADMAP's `todos.update { it.add(Todo("x")) }` is written with `edit`, and with an explicit parameter until the implicit `it` (a ROADMAP follow-up) exists. The members take a shared receiver (`fun`), so a handler's captured `val` handle calls them (`escaping-closures` D7).

Left out: `derivedStateOf` (a memoized computed state, §5 D25), `snapshotFlow` (no coroutines), `mutableStateListOf` (per-element tracking; `edit` invalidates the whole list's readers, which is what a recomposition of a list reader needs anyway), `SnapshotMutationPolicy` (equal-write skipping is fixed, D10), and `var count by remember { mutableStateOf(0) }` (delegated properties are a language feature of their own). Each is a follow-up.

Compile-time cost: none beyond the library's own check, cached under the compiler version. Calls are signature-typed intrinsics, checked by the ordinary generic call path: `R` of `read` is solved from the closure's body, as `remember { 0 }` is solved (`generics`, "Inference is local and bidirectional").

Rejected: `getValue()`/`setValue(v)` functions (not Kotlin); `update` with a `(&mut T) -> Unit` editor (Kotlin's `update` returns the new value, and a by-value transform is what a `Copy` counter wants: `count.update { n -> n + 1 }`); only `edit` (no by-value form for `Copy` types); `mutableStateListOf` now.

### D3. Handles are intrinsic structs with unconditional markers; `MutableState<T>` converts to `State<T>` (autonomous; implements `compose-architecture` D12)

Both handles are intrinsic structs of `compose-transform` D16's form, laid out as one word, `Copy` for every `T`. A declared generic struct's `: Copy` is conditional on its type arguments (`generics`); an intrinsic struct's markers come from the catalog and hold for every argument, so `MutableState<List<Todo>>` is `Copy`. The catalog entry says so, and the declaration's `: Copy` must match it (`E4029`). Both are invariant, as every generic type is.

`MutableState<T>` is assignable to `State<T>` in `types.rs::is_assignable_to`, as a subtype step that keeps the word (MIR `Rvalue::Subtype`), in the positions where `&mut T` converts to `&T`. A member or extension lookup on a `MutableState` receiver that finds nothing for its type constructor is retried for `State`, with the receiver converted, so `m.read { }` and user extensions of `State<T>` work on both, as Kotlin's `MutableState<T> : State<T>` makes them. Both handles are stable parameters (`compose-transform` D7): `Copy` and borrow-free.

Compile-time cost: one more hash lookup only when a lookup on a `MutableState` misses, and one constructor comparison in assignability. Kotlin-likeness: the same two types and the same subtyping, without interfaces.

Rejected: one handle type (§5 D12: a `State<T>` parameter says the composable does not write); a general nominal subtyping or interface mechanism (traits are a later change; this is one built-in edge, as `&mut T` to `&T` is); declaring `read` twice, once per handle (the ABI and the docs would show duplicates); conditional markers for handles (a `State<String>` would not be `Copy`, against the state-handle invariant).

### D4. `intrinsic var` for the writable property (autonomous)

`standard-library` added `intrinsic val` because `length` and `size` are read without parentheses. `MutableState<T>.value` is also assigned, so the property form gains `intrinsic var`, which the parser, the ABI (as a modifier that changes the hash) and `prism fmt` accept as they accept `intrinsic val`. The catalog lists each property as read-only or writable, and a mismatch is `E4029`. Kotlin declares `override var value: T` on `MutableState<T>` and `val value: T` on `State<T>`, which is exactly what the two declarations say, and the writability of an assignment is read from the declaration found, not from a special case.

Compile-time cost: one token and one flag in the parser and the ABI.

Rejected: one `intrinsic val` on `State<T>` with writability decided by the receiver's type (the ABI would not say which handle is writable); ordinary extension properties (a follow-up of `standard-library`).

### D5. What `s.value` is (autonomous; refines `compose-architecture` D13)

| Use | `Copy` `T` | Non-`Copy` `T` |
|---|---|---|
| read (`val n = s.value`, operands, arguments) | a copy, anywhere: a call of `BuiltinFunction::StateValue`, so borrowing it borrows a temporary, as `s[i]` of a string does | in a composable context, a place (`ExpressionKind::StatePlace`) read in place, borrowed shared, projected and copied out of for `Copy` parts; moving it out is `E4017`; elsewhere `E4033` |
| `s.value = v` | anywhere outside composable contexts (`E4031` inside); `E4003` on a `State<T>` | the same |
| `s.value op= e` | the same as `=`: read, operate, write, the handle evaluated once | `E4033` (help: `update`) outside composable contexts, `E4031` inside |
| `&mut s.value`, `mut fun` receivers, part writes (`s.value.x = v`) | `E4031` inside composable contexts; `E4033` (help: `edit`) elsewhere | the same |

The rule is decided by the declared `Copy` marker of `T` (a type parameter only through a `Copy` bound), so it costs one marker lookup and no inference, and a generic `fun <T> f(s: State<T>)` reads `s.value` only in a composable, or with `T : Copy`.

Writes are whole-value: an assignment to `s.value` is a call of the runtime with the handle by value and the new value moved (D17), never a write of a place rooted at the handle, which `escaping-closures` D7 requires, so `{ count.value += 1 }` captures a `val` handle by copy and allocates nothing. A part write such as `s.value.x = v` would need the runtime to know the write ended in order to invalidate, which is `edit`. Borrowing a `Copy` value in place was rejected for one representation per marker: a copy is what Kotlin's getter returns, and the struct being copied is `Copy`, so small.

Why a place in composable contexts only (§5 D13, confirmed): during a pass no state can be written (D11) and no cell is freed before the pass ends (D12), and a borrow taken in a composable context can only reach frames of the same pass (escaping closures hold no borrows, struct fields hold none, and composables are called only in passes), so a shared borrow of the cell's value is valid until the body ends, with no run-time flag. Outside composition, any code may write any cell through any copy of a handle, which no static check can see, so access goes through closures whose borrow the runtime guards (D6).

Compile-time cost: a member lookup, a marker lookup and a test of the composable-context flag per access, all constant.

Rejected:
- A place for `Copy` values too (borrowing a `Copy` `s.value` outside composition would need a flag or an error, for no gain over a copy).
- Run-time-checked place borrows released at their last use (§5 D13: the borrow checker would have to emit releases).
- A closure-only API (no `for (todo in todos.value)` in composables, which Kotlin users write).
- Kotlin's semantics for `s.value.x = v` (a silent in-place change that Jetpack does not observe, a known source of bugs).

### D6. `read`, `update` and `edit` with run-time borrow flags (implements `compose-architecture` D13)

Each lowers, in MIR, to a begin call, a call of the closure, and an end call:

| Member | Begin | Closure | End | Cell while it runs |
|---|---|---|---|---|
| `read(reader)` | `state begin read` → `&T` | `reader(r)` | `state end read` | shared (a count) |
| `update(transform)` | `state take` → `T`, moved out | `transform(v)` | `state put`, the result moved in | empty |
| `edit(editor)` | `state begin edit` → `&mut T` | `editor(r)` | `state end edit` | borrowed mutably |

A cell's flag is free, shared with a count, empty or exclusive. `state read` (the copy or place read of D5) and `begin read` need it free or shared; writes, `take` and `begin edit` need it free; anything else is the runtime error `state accessed while borrowed` (D14). So re-entrant access, `s.update { n -> n + s.value }` or a nested `edit` of the same cell, fails at run time the way `RefCell` does, and a mutable borrow and a shared one never coexist.

The references the begin calls give are free references in MIR: they hold no loan (D18), and the closure's type keeps them from escaping: `(&T) -> R` returns a borrow-free `R` (type arguments are borrow-free, `E4022`), and `(&mut T) -> Unit` returns nothing. Runtime errors do not unwind, so the end call is reached on every path that continues.

`update` moves the value out rather than lending it: a `(T) -> T` transform takes and returns ownership, so `count.update { n -> n + 1 }` and `name.update { s -> s + "!" }` work without a `Copy` bound, and the cell holds no value while it runs. A read of an empty cell is the same borrow error. `update` and `edit` always invalidate (D10).

Run-time cost: two entry calls around one closure call. Compile-time cost: constant per call, in MIR lowering.

Rejected: a runtime that calls the closure itself (it cannot call a closure whose machine signature depends on `T`); static exclusivity for handles (every copy of a handle aliases the cell, which no static rule tracks without lifetimes).

### D7. Static rules and three new codes (autonomous; implements `compose-architecture` D13 and D14)

- **`E4031` (`StateWriteInComposable`)**: a write of a state in a composable context: `value =`, a compound assignment, `update`, `edit`, and the mutable uses of D5. Note: "state is read-only during composition"; help: "write it in an event handler or a `SideEffect`". A non-composable closure written inside a composable is not a composable context, so its writes are accepted (and checked at run time, D11).
- **`E4032` (`UnrememberedState`)**: `mutableStateOf` called in a composable context, with help `remember { mutableStateOf(<argument>) }`. A `remember` calculation and a default value are not composable contexts (`compose-transform` D12, D17).
- **`E4033` (`StateValueOutsideComposable`)**: outside composable contexts, a non-`Copy` `s.value` used other than as an assignment target, or any `s.value` borrowed mutably, used as a `mut fun` receiver or written in part. Help names `read` for a shared use (`todos.read { value -> ... }`) and `edit` for a mutable one, or `update` for a compound assignment.
- Existing codes: `E4003` for an assignment to the `value` of a `State<T>`, with help naming `MutableState<T>`; `E4017` for moving a non-`Copy` `s.value` out ("the value of a state"), with help suggesting a borrow or `.clone()`; `E5007` for a function value that may hold a borrow written into a state, since the value parameters of `mutableStateOf`, `value =` and `update` are type-parameter slots ("passed where the callee's declared parameter type has a type parameter outside a function type"), and `captures.rs` treats those positions as owning, so a closure written there owns its captures.

Three codes, one per fix, as the catalog's rationale asks: move the write to a handler (`E4031`), wrap the creation in `remember` (`E4032`), use a closure member (`E4033`). Kotlin reports none of these: Jetpack allows writes during composition through snapshots, and reports unremembered state only through a lint. Prism has no snapshots (§5 D16), so a write during composition is a bug it can name.

Compile-time cost: one flag test per write, per `mutableStateOf` call and per non-`Copy` `s.value`, constant.

Rejected: one code for all three (three different fixes); reusing `E4008` (that code is about calling composables); a warning for unremembered state (a state recreated every pass is never correct).

### D8. Cells live in an arena of `prism-compose`, their values in blocks that never move (autonomous)

`crates/prism-compose/src/state.rs` adds a `CellArena` to the per-thread state of `compose-runtime` D4. A cell record holds:

- its slot type record (`compose-runtime` D6: fingerprint, size, alignment, drop, equality, name), from which it drops and compares its value;
- its value, in a block of exactly the record's size and alignment, allocated when the cell is created and freed with it (none for a zero-sized type), so the value's address never changes while the cell lives, however the arena grows: a borrow of D5 stays valid while later cells are created in the same pass;
- its flag (D6), its readers (D9) and its owner (D12).

A handle is `compose-runtime`'s `Handle` over the cell arena: a 32-bit index and a 32-bit generation, the generation starting at 1, so the word 0 is never a handle and serves as the failure result of creation (D14). Every entry validates the handle in O(1), and also compares the record it is given with the cell's (by pointer, then by fingerprint, as slots match), so a handle can never reach a cell of another type even across a hot reload. Both failures are a stale handle (D15).

Run-time cost: one allocation per cell created, which happens on a `remember` miss, in a handler or in program code, never in a steady-state pass. Memory: one record of about 64 bytes per cell plus its value.

Rejected: values inline in the arena's vector (a vector that grows moves them, which would break D5's borrows); values in the owning slot (program and composition cells have no slot, and a slot's inline storage moves with its group's slot vector); one global arena behind a lock (`compose-runtime` D4); skipping the record comparison (a handle kept in a top-level value or a closure across a reload could reach a cell whose layout changed).

### D9. Reads register the innermost restart scope (autonomous; implements §5 "Restart Scopes and Invalidation")

`state read` and `begin read`, during a pass, register the active pass's current scope (`compose-runtime` "The Current Scope": the innermost restart group open, or the root) as a reader of the cell. The entries take no composer: plain functions called from composition have none, and the active pass is in the thread's state (`compose-runtime` D4), one thread-local access the entry makes anyway. Reads outside a pass register nothing, so a handler that reads a counter to log it does not subscribe anything.

A cell keeps its readers in a vector with a membership test, linear while it holds at most 8 scopes and through a hash set beyond, so a scope that reads a cell many times, in one pass or in many, is registered once. When the vector would grow, the scopes that have left their composition (a generation mismatch) are removed first, so it holds at most twice the live readers. A write that invalidates clears it (D10), and the scopes that still read the cell register again when they recompose.

A scope that stops reading a cell, without the cell being written, stays registered until the next write, which recomposes it once more for nothing. Jetpack clears a scope's old observations when it recomposes, which needs a per-scope list of the cells it read, one more structure per scope written on every read. That is a follow-up if the extra recompositions show up in the todo app's measurements.

Run-time cost: amortized O(1) per read, no allocation once a cell's readers have been registered (the steady state of "Steady-State Reads Allocate Nothing").

Rejected: registering the composer's group rather than its scope (only restart scopes can be recomposed alone, `compose-transform` D10); no deduplication (a list reader registers again on every pass, without bound); clearing per scope now (above).

### D10. Writes invalidate their readers; equal `value =` writes do not (autonomous; refines `compose-architecture` D16)

A write invalidates each reader (`compose-runtime` "Invalidating a Scope", O(1) each, nothing for removed scopes) and then clears the readers. The invalidated scopes recompose in the next pass, which the program runs: `c.recompose()` in fixtures, `runApp`'s frame loop after its queued events in `ui-toolkit`. Since writes happen only outside passes (D11), a pass never sees an invalidation made during it, and a pass's scopes are fixed when it starts (`compose-runtime` D4).

A `value =` write, and the write of a compound assignment, first compares the new value with the current one through the cell record's equality entry, the type's equality glue (`compose-transform` D20). When they are equal, the cell keeps its old value, the new one is dropped through the record's drop entry, and nothing is invalidated: Jetpack's `structuralEqualityPolicy`, so `count.value = count.value` and `name.value = "same"` recompose nothing. A type without an equality entry always invalidates, and so do `update` and `edit`, which change the value without the runtime seeing an old and a new one to compare.

§5 D16 places the comparison "at the write site, inline only for scalars". The runtime makes it instead, through the record the write passes anyway: one indirect call, which is what a glue call at the site costs, and no comparison code at any write site, so a unit's code does not grow with its writes. Records gain equality entries for every type `==` applies to, strings and lists included, which `compose-transform` did not need for kept parameters (strings are not stable); function types compare by closure and captures, as for skipping (`compose-transform` D9).

Run-time cost: one equality call per write plus O(readers) invalidations; an equal write allocates nothing and drops the new value. Compile-time cost: none per write; one equality glue function per cell type per unit, memoized with the other glue.

Rejected: comparing in generated code at each site (§5 D16's first version; code per site); invalidating on every write (an equal write is the most common no-op of event handlers); a configurable policy (follow-up); recomposing inside `recompose` until nothing is invalid (a side effect that writes on every pass would loop forever, and the frame loop already runs again when anything is invalid).

### D11. State is read-only during composition, also at run time (implements `compose-architecture` D14)

The static rules of D7 catch writes and creations written in composable contexts. Plain functions and non-composable closures called during a pass escape them, so the runtime checks too:

- **Writes.** `state write`, `take` and `begin edit` while the thread runs a pass fail with `state written during composition`. A pass is active from its first body to the end of its last, so releases and side effects (D13), which run after it, may write.
- **Creation.** `state create` during a pass needs a *creation owner* (D12). The runtime keeps a stack of them: a `remember` calculation pushes its slot, a handler or side effect pushes its composition, a top-level initializer pushes the program. During a pass with an empty stack, creation fails with `state created during composition outside remember`; outside a pass with an empty stack, the program owns the cell.
- **The window of `remember`.** `compose-transform`'s lowering of `remember` (its D12) runs the calculation only when the slot is missing or a key changed, and writes the result with `set_slot`. This change brackets that path with two new composer operations: `begin remember` before the calculation pushes a pending owner, and `end remember` after `set_slot` pops it and attaches the cells created since to the slot just written (`compose-transform` D23 left exactly this window). A hit runs neither.
- **Top-level initializers.** A top-level value is initialized on its first read (`execution`), which may happen during a pass. Its initializer is program code, so the backend brackets it with `prism_compose_begin_program`/`end_program`, and `val Theme = mutableStateOf(3)` works wherever it is first read. Writes in an initializer during a pass still fail.

Run-time cost: one test of the active pass per write; two entry calls per `remember` miss; two per top-level initialization, once per value per run.

Rejected: deferring writes made during a pass to its end (§5 D14); attaching cells by inspecting the remembered value for handles (a calculation may create cells it does not return, which would leak; and the runtime cannot read a value's layout); a static rule for plain functions (it would need to know which functions can run during composition, a whole-program analysis).

### D12. Each cell has one owner, which frees it (autonomous; implements `compose-architecture` D15)

| Creation | Owner | Freed |
|---|---|---|
| in a `remember` window | the slot `end remember` attaches it to | when the slot is released: a changed key forgets it, its group is removed, or its composition is disposed, at the end of the pass, after the slot's value (`compose-runtime` D7) |
| outside a pass, while a handler or side effect of a composition runs | the composition | when the composition is disposed |
| anywhere else outside a pass, or in a top-level initializer | the program | before `--check-allocations` reports; never without it |

Freeing a cell drops its value through its record, frees its block and gives its index a new generation, or retires the index at the maximum generation. Releases run after the pass, so a borrow of D5 taken earlier in the pass stays valid (§5 "Removed groups drop their slots after the pass").

A cell that is borrowed or emptied when its owner frees it (a handler's `edit` that recomposes or drops the composition owning the cell) becomes stale at once, so no new access reaches it, and is freed when the borrowing call ends; `put` into such a cell drops the transform's result. This is the only way a release can meet a live borrow, since only closures of D6 hold borrows outside passes.

Program cells are freed only under the check, as top-level values are released only by it (`runtime-strings-and-drops` D12): an exiting program has no reason to drop them. The check frees them first, because a cell may hold a `Composition`, whose drop disposes it and frees its cells, then reports live compositions (`compose-runtime` D13), then allocations. `compose-runtime`'s finish report hook is extended into that sequence in `prism_compose::finish`.

Handlers belong to `ui-toolkit`, which dispatches an event by calling `prism_compose::Composition::run_handler(handle, f)`, the Rust API that pushes the composition as the creation owner around `f`. In this change only side effects use it (D13), and the Rust tests.

Run-time cost: linear in what is released; no counting anywhere. Memory: a cell made by a handler lives until its composition does, as §5 D15 accepts.

Rejected: reference counting (zero hidden ARC); rejecting `mutableStateOf` outside `remember` (§5 D15); freeing program cells at every exit (wasted work); freeing a borrowed cell at once (a use after free in the borrowing closure).

### D13. `SideEffect`, brought forward from `compose-architecture` D25 (autonomous)

`SideEffect(effect: escaping () -> Unit)` is Jetpack Compose's: it queues `effect` each time a pass runs the call, and the effects run after the pass has ended and released what it removed, in the order they were queued, each as a handler of the composition (D12), and are then dropped. A skipped body queues nothing. `composition { }`, `setContent` and `recompose` run their pass's effects before they return, and an effect's writes are recomposed by the next pass.

D25 left effects out of Milestone 2. One is needed here: before `ui-toolkit`'s nodes hold handlers, nothing composed can act outside a pass, since writes during a pass are errors and no value leaves a composition. Without it, no Prism program could create a composition-owned cell or keep a handle past its cell's release, and the scenarios "A Handler's Cell Belongs to Its Composition" and "Stale Handles End the Program" could only be Rust tests. `SideEffect` is the smallest effect Jetpack has (no keys, no disposal, no coroutine) and stays useful afterwards, for instance to hand state to non-compose code. `LaunchedEffect` and `DisposableEffect` stay deferred.

It is an intrinsic composable lowered by the transform (`compose-transform` D19) to the composer operation `side effect`, which moves the function value to the composition's effect queue through `prism_compose_side_effect`. The queue is a reused vector, so a steady-state pass that queues effects allocates only what their closures do.

Compile-time cost: one catalog entry and one operation per call. Run-time cost: one entry call per queued effect, one call per effect after the pass.

Rejected: a test-only hook in `/prism/composer` (the composer module cannot reach the compiler's `remember` or state, and fixtures would test a protocol nobody writes); waiting for `ui-toolkit` (the run-time errors and owners of this change would ship without a Prism fixture); `LaunchedEffect` (needs coroutines or timers).

### D14. Errors at the access, through failure results (autonomous)

The state entries cannot see the source, and passing a location to every call would cost four arguments per access. So each entry signals failure in its result (a null address, a zero handle or a zero status), having recorded the error's kind in the thread's state, and the generated code tests the result and, in a cold block, calls `prism_compose_state_error(file, file_length, line, column)`, which ends the program with the recorded error at that location. This is the shape of list bounds checks (`collections` D8, D20): an inline test, a located cold path. The four errors are new `PanicKind`s of `prism-runtime`: `StaleStateHandle`, `StateBorrowed` ("state accessed while borrowed"), `StateWrittenDuringComposition` and `StateCreatedDuringComposition` ("state created during composition outside remember"), after `compose-runtime`'s `CompositionMisuse`.

Unlike the protocol misuse of `compose-runtime` D12, which only hand-written protocol code can reach, these errors are bugs of ordinary programs, and their location is what tells the programmer which handler or which plain function is wrong.

Run-time cost: one compare and a branch not taken per access. Compile-time cost: one cold block per access, as for list accesses.

Rejected: no location (as protocol misuse; unhelpful for these errors); four location arguments per call (more code per access than the cold block, and more argument moves on the hot path); a static location record per access (one data symbol per access).

### D15. Stale handles (implements `compose-architecture` D12)

A handle is stale when its index names no cell, its generation differs from the cell's, the cell has been released while borrowed (D12), or the record it is used with is not the cell's (D8). Every use of a stale handle, read, write or closure member, is the runtime error `stale state handle`. Indexes whose generation reaches the maximum are retired, as `compose-runtime`'s arenas retire them, so a handle can never come to name a newer cell. `==` of two handles compares their words and never fails: a stale handle is still a value.

Rejected: reading a stale handle as a default value (it would hide the bug); reusing retired indexes (a 32-bit generation wraps after four billion cells at one index, rare but unsafe).

### D16. One program thread (confirms `compose-architecture` D16)

Cells live in the per-thread state, like groups and compositions (`compose-runtime` D4). Prism has one program thread, on which compositions, handlers and side effects run. A handle used on another thread, which only Rust tests can do, finds no cell or another one and fails the generation or record check, as a stale handle: an error, never undefined behavior. Snapshots, which make Jetpack's state thread-safe, wait for threads.

### D17. Typed tree and MIR forms

- **Typed tree.** `ExpressionKind::StatePlace { handle }`, of the value's type, over a handle expression copied out. `BuiltinFunction::{MutableStateOf, StateValue, StateRead, StateUpdate, StateEdit, SideEffect}`, each with the value type as its first type argument (`StateRead` also `R`). `Assign` and `CompoundAssign` accept a `StatePlace` target (`typed-tree` "Assignments Are Statements Over Places" gains it). `TypedFile::validate` checks that a `StatePlace` is an assignment target or, in a composable body, is never moved out of.
- **MIR.** `Callee::State(StateOperation)` with `Create`, `Read`, `Write`, `BeginRead`, `EndRead`, `Take`, `Put`, `BeginEdit` and `EndEdit`, called with ordinary `Call` terminators whose type argument is the value type, so `instantiate.rs` substitutes them. A `Copy` read is `_r = state read<T>(copy h)` and a copy of `*_r`; a place is `*_r`; `s.value = v` is `state write<T>(copy h, move v)`; a compound assignment evaluates the handle into a temporary once, reads, operates (with the overflow checks of the operator) and writes; the closure members are the sequences of D6. `prism dump-mir` prints `state <operation><T>(...)`. Composed MIR adds `ComposerOperation::{BeginRemember, EndRemember, SideEffect}`.
- `instance-cache`'s codec encodes every new form, which its exhaustive matches enforce.

### D18. The borrow checker sees free references

A reference from `state read`, `begin read` or `begin edit` holds no loan: it is derived from a call with no borrowed input, like `Constant::TopLevelValue`. `returns.rs` lets a body return a borrow derived from `state read` under any rule, as it does for top-level values, which D5's argument makes sound, since such a body is composable. Nothing else changes: the begin/end pairs are ordinary calls, the closure call borrows its function value as any call does, and `take`/`put` move ordinary locals.

Compile-time cost: none beyond one more case where top-level constants are recognized.

### D19. The backend

- **Layout and glue.** Handles are one 8-byte word for every `T` (`layout.rs`), with no drop glue, a word copy as clone glue and a word compare as equality glue; `dump-layout` names them `prism/compose/runtime::State<...>` and `prism/compose/runtime::MutableState<...>`.
- **Operations.** Each `Callee::State` operation is a direct call of its entry point through `UnitContext::entry_point`, with the handle word, the record from `UnitContext::slot_type(T)`, and values moved in or out spilled to a stack slot and passed by address, followed by the failure test of D14. Composer operations of D17 call `prism_compose_begin_remember`, `prism_compose_end_remember` and `prism_compose_side_effect`.
- **Records.** `glue.rs` gives every cell type an equality glue function when `==` applies (`#equals(String)` over `prism_rt_string_equals`, `#equals(List<T>)` from `collections`, scalars as `compose-transform` D20 already does, function types as its D9).
- **Top-level initializers.** The accessor of a top-level value brackets its initializer with `prism_compose_begin_program` and `prism_compose_end_program` (D11).

Compile-time cost: one record per cell type per unit, shared with kept and remembered values; one call and one cold block per operation.

### D20. Cells in dumps

`compose-runtime`'s dump prints a slot as `slot <type name>`. A slot's cells follow it, one level deeper, as `cell <type name>`, in creation order, so fixtures see when a remembered state is created and freed. Program and composition cells are not in any group and are not dumped; the live count of `reactive-app-verification` covers them later.

### D21. Costs, and how they are measured

| Stage | Compile-time cost |
|---|---|
| type checker | per access: one member lookup (plus one on a `MutableState` miss), one marker lookup, one composable-flag test |
| MIR lowering | per access: one to three calls; per closure member: three |
| borrow checker | nothing new |
| backend | per operation: one call and one cold block; one record and one equality glue per cell type per unit |

Nothing is per instance beyond the substitution every generic call gets, and nothing is whole-program. `crates/prism-cli/tests/compile_timing.rs` gains `state-accesses`: one handler with `n` statements `s.value += 1`, one composable reading `n` states, and `n` `edit` calls, at its usual sizes.

| Run-time operation | Cost |
|---|---|
| `mutableStateOf` | one arena slot and one block allocation |
| read | one entry call, a generation and record compare, during a pass an amortized O(1) registration |
| write | one entry call, one equality call, O(readers) invalidations |
| `read`/`update`/`edit` | two entry calls around the closure call |
| release | one drop call and one free per cell |

`crates/prism-compose/tests/runtime_timing.rs` gains a shape reading one state from 1,000, 10,000 and 100,000 restart scopes ten times per pass, and `crates/prism-compose/tests/allocations.rs` checks that steady-state reads allocate nothing.

### D22. Capabilities and tests

State extends the `compose` capability, so one spec describes what a composable and its state do. The declarations extend `standard-library`, the property form `syntax` and `formatting`, the conversion `type-system`, the forms `typed-tree` and `mir`, the borrows `references`, the backend `code-generation`, and the errors and the release order `execution`. The diagnostics catalog is edited directly.

Behavior is pinned by executable fixtures under `crates/prism-cli/tests/fixtures/compose/`, `references/`, `type-system/`, `code-generation/` and `execution/`, all under `--check-allocations`, driving compositions from `main` with `composition`, `setContent`, `recompose` and `dump`, `main` and plain closures playing the event handlers. What Prism cannot observe (release while borrowed, release order, deduplication, another thread) is pinned by Rust tests of `prism-compose`.

## Risks / Trade-offs

- **[Two representations of `s.value`]** A copy for `Copy` types and a place for others: a `Copy` struct state is copied whole to read a field. → `Copy` structs are small by construction, and the rule is one marker lookup that users can predict.
- **[Non-`Copy` arguments from state]** `Text(name.value)` moves a `String` out of a state, `E4017`, where Kotlin passes a reference. → The help names `.clone()` and a borrow; `ui-toolkit` should take `&String` where a composable does not keep its text, which is its decision.
- **[Spurious recompositions]** A scope that stops reading a cell recomposes once more on its next write (D9). → Bounded to one per write; per-scope clearing is a follow-up if measurements ask for it.
- **[Cells made in handlers live until their composition ends]** (§5 D15). → An explicit release, or cells owned by data structures, is a later option.
- **[`SideEffect` before the other effects]** An effect API arrives before Milestone 2's plan wanted any. → It is Jetpack's simplest one, with its semantics, and the others stay deferred.
- **[Run-time errors for what Kotlin allows]** Writes in plain functions called during composition fail at run time. → They are the order-dependent bugs §5 D14 names; the error has a location.
- **[Prerequisites unimplemented]** Every name this design takes from `compose-runtime`, `compose-transform` and `standard-library` comes from their packets. → Tasks begin by checking them against the code those changes leave.

## Migration Plan

No existing program changes meaning: `/prism/compose/runtime` gains declarations, which a file's own declarations of the same names shadow (`standard-library` D4). `prism dump-mir --composed` shows the remember window around every `remember` miss, and composed snapshots are re-blessed. Top-level accessors gain two calls, which `dump-clif` snapshots of top-level values show. `examples/demo/ui` stops failing on `mutableStateOf` and fails only on names of the toolkit modules (`ui-toolkit` D21).

## Open Questions

None blocking. The follow-ups (`derivedStateOf`, `mutableStateListOf`, mutation policies, `by` delegation, implicit `it`, clearing a scope's reads when it recomposes, releasing handler-made cells, `LaunchedEffect`/`DisposableEffect`, a niche for `State<T>?` in the zero word) go to `ROADMAP.md` at archive.
