# Design: strong-skipping

## Context

See `proposal.md` for motivation. This change implements agreed item **A1** of the post–Milestone 2 review (`docs/milestone-2-review-notes.md`, "Agreed changes"): "No stability inference: every restartable composable can skip, as with Compose's strong skipping (user). `Copy` parameters compare by value. A borrowed parameter carries an identity the compose transform derives syntactically at the call site: address plus source version (a state cell's version; a top-level `val`; the parent parameter's identity passed down). A borrow of a local or temporary has none and always recomposes. Different identity means recompose, with no deep-comparison fallback and nothing cloned or kept (user: avoid expensive comparisons). A structural list change recomposes every row once; per-element stamps are a possible refinement (open). Replaces the kept-clone and kept-hash ideas." It is item 5 of "After Milestone 2: the review round" in `ROADMAP.md`, and it also takes option (b) of review finding F4 (paced benchmark inputs), as the review notes recommended "when `strong-skipping` lands". Decisions the user took are marked **(user direction 2026-10-19/20)**; every other decision the user might take differently is **(autonomous)** and logged in `docs/decisions-for-review.md`. Compile speed is a primary criterion (AGENTS.md §5): each decision states its compile-time cost.

**Baseline.** This packet builds on two packets that are proposed and not archived: `path-captures` (item 3, being implemented) and `owned-state` (item 4, proposed in `18fa18c`), which lands before this change. Their delta specs are taken as applied: captures are paths (`todo.id` copied by handlers); `MutableState<T>` is a move-only owning object whose cell never moves; `remember` keeps owned values and gives a shared borrow of the slot; `&State<T>` and `&MutableState<T>` parameters are stable, kept as their weak word and compared by identity (`owned-state` D13); `owned-state` D18 names this change's seam ("state contents by (handle, version), where every write, `update` and exclusive guard already goes through one runtime call"). The MODIFIED and RENAMED blocks of compose "Stable Parameters" and "Skipping Composables With Stable Parameters" are written on `owned-state`'s versions, which are written on `path-captures`'. The first task re-bases every block on the main specs once both are archived.

**Baseline (task 1.1, 2026-10-20).** `path-captures` is archived (`f6a6da7`, then `484d4c0`: closure slot contracts record each capture's path) and so is `owned-state` (`fd397f6`, with its review fixes `1721591` to `c4c6923`: guards hold a borrow of their own `state guard` origin; `remember` lends a non-`Copy` value as a `&T` without a loan; `&State`/`&MutableState` parameters are kept as weak words; `attached` parameters; `E5010`). Every MODIFIED block was compared with the current main text and keeps all its main requirement's scenarios; the RENAMED blocks still name the main requirements `Stable Parameters` and `Skipping Composables With Stable Parameters`; no ADDED requirement exists in a main spec. One scenario had drifted and was re-based: mir "A compound assignment through a handle" now reads "borrows `*s` into a temporary once", as the main text does. Every name `tasks.md` uses exists (`is_skippable`, `skippable_prologue`, `keep_parameter`, `parameter_bits`, `place_source`, `change_words`, `restart_body`, `ComposerOperation::{Keep, KeptParameter}`, `StateOperation`, `FileSummary::composable_bodies`, `ScriptCommand`, `DumpKind`), and the todo example's composables are those of `owned-state` D20 (`TodoScreen`, `TodoRow(todo, todos)`, which takes `todos` rather than `onRemove`, `DoneBox`, `LeftCount`, `FilterRow`). Two findings of the reconciliation change the plan, recorded as D19 and D20 below: the runtime overwrote a remembered value without a drop entry in place when its key changed, which would let a kept identity of the old value match the new one, so a lent value now always gets a new block (D19); and the `owned-state` reviewer's note that a `remember` borrow, which holds no loan, can reach an `attached` closure safely only because of release order is the same order D9 relies on, so scenarios now pin a replaced and a removed remembered value under `--check-allocations` (compose, "A Scope Restarted Alone Reads Its Kept Borrows"). `openspec validate strong-skipping --strict` is clean.

Current state (main specs and code, plus the two packets):

- **Skippability** (`crates/prism-compose-transform/src/stability.rs`, `compose-transform` D7): a parameter is stable when its type is `Copy` and borrow-free by declared markers, or an `escaping` (with `owned-state`, also `attached`) non-composable function type, or (with `owned-state`) a shared borrow of a state object. A composable returning `Unit` whose parameters are all stable is skippable and a restart scope (`is_skippable`). Every other borrow makes it unskippable (`compose-architecture` D9).
- **Calling convention and change bits** (`rewrite.rs`, D3, D4): after the declared parameters, the composer and `max(1, ⌈n / 31⌉)` change words, two bits per parameter (uncertain, same, different, static) and a force bit. A caller says static for constants and copies of top-level values, forwards its own bits for a copy of its parameter (`Source::Parameter`), and uncertain otherwise, from a block-local map (`Rewriter::sources`).
- **Kept arguments** (D8, D11): `skippable_prologue` tests each function parameter's keepability (else a replaceable group of the same key), starts the restart group, keeps each argument (`Copy` values by `prism_compose_keep`, function values by a switch over their bits and `Changed` through the slot type's equality glue), and skips when nothing forces, nothing changed and the scope is valid. `F#restart` (`restart.rs`) reads the kept slots and calls `F` with the force bit and every parameter same.
- **State reads in composable contexts** (`owned-state`'s mir "State Operations Are Calls"): `state read<T>` takes the state object by shared borrow and gives `&T`; a state place is its dereference. During a pass no state is written and no cell is freed; reads register the innermost restart scope.
- **Composable closures** (D14): `key(v) { }`, `Column { }` and every other content lambda is a separate MIR body, a borrowing closure built by `Rvalue::Closure { captures }` with one `LocalKind::Capture` local per captured path; it runs in a replaceable group and is never a restart scope.
- **Removals are deferred**: a forgotten slot, a removed group and a replaced restart function are released after the pass (compose, "Removals Are Deferred to the End of the Pass"), so every slot address given out during a pass stays valid until it ends. Replaced top-level storage stays valid for the whole run (hot-reload, "Replaced Top-Level Storage Stays Valid").

## Goals / Non-Goals

**Goals:**
- `TodoRow(todo: &Todo)` over `todos.value` skips when its item is unchanged: typing in the 1,000-item app's field reruns no row, a toggle reruns one row.
- Every check at run time is constant per parameter: one two-word comparison, no traversal, no clone, no hash, no allocation.
- Every decision at compile time is syntactic and local: one classification per assignment of a reference and per argument, linear in the body.
- Memory safety: a scope restarted alone never reads a borrow whose referent is gone or changed.

**Non-Goals:**
- Per-element versions inside a state's value (D3).
- Identity through calls that return borrows (`fun first(s: &State<List<T>>): &T`), which elision would allow (D5).
- Skipping composables with by-value non-`Copy` parameters, `&mut` parameters or composable content lambdas (D1).
- Memoizing composable lambdas, as Compose's strong skipping does with `remember` (lambdas here borrow their captures and are not values one can keep).

## Decisions

### D1. Every restartable composable can skip; no stability inference (user direction 2026-10-19/20; the parameter kinds autonomous)

A composable is **restartable** when it returns `Unit` and every parameter, the receiver and context parameters included, can be kept: a `Copy` borrow-free type by its declared markers (a type parameter by its `Copy` bound), an `escaping` or `attached` non-composable function type or a nullable of one, or a **shared borrow** `&T` or a nullable `(&T)?`, of any `T`. Every restartable composable is skippable and a restart scope; there is no other notion of stability, and nothing reads fields, bodies or other files. The word "stable" leaves the compose spec: its two requirements are renamed "Parameters That Can Be Kept" and "Every Restartable Composable Can Skip".

What stays unskippable, and why: a by-value non-`Copy` parameter (`s: String`, `items: List<T>`, an unbounded `T`) is moved into the callee, a new value on every call, which a restart could only pass again by cloning it from a slot; a `&mut T` lets the callee write the caller's data, so a skip would hide a write and a restart would alias it; a composable content lambda borrows its author's frame. These are the cases Compose's strong skipping compares by instance; Prism's idiom is to take `&T` instead, which now skips.

**Compile-time cost.** One test per parameter type (a marker lookup, a reference test, a capture-function lookup for state objects), as today.

Rejected:
- **Stability inference from fields** (whole-program, and the user asked for none).
- **Keeping a clone of a borrowed value and comparing by `==`** (review option C2 (b)): a clone per kept argument on every parent pass and a deep comparison (user: avoid expensive comparisons).
- **Keeping a hash** (C2 (c)): a traversal per pass, and a collision skips wrongly.
- **Compose's instance identity for by-value non-`Copy` parameters**: a moved value has no stable instance to compare, and restarting needs a clone.

### D2. A borrowed argument's identity is its address and its source's version, derived syntactically (user direction 2026-10-19/20; parameter parts autonomous)

A kept borrowed argument is identified by two words: its **address** (the reference itself) and a **version** that says whether the data at that address may have changed. The transform derives the version from where the caller's reference comes from:

| Source of the reference | Version | Why the data cannot change while the version holds |
|---|---|---|
| A state's contents: the result of `state read`, and any field, index or `Heap` dereference below it (`todos.value[i]`, a `for` variable over it, `todo.title` of one) | the state's current version (D3) | every write that can change the value gives it a new version |
| The value a `remember` keeps (its shared borrow, and parts of it) | permanent (1) | a remembered owned value is only borrowed shared and changes only through the states it holds; its slot never moves, and a new slot is allocated while the old one still exists (D9) |
| A top-level `val` (and parts of it) | permanent (1) | top-level values are immutable, and replaced storage stays valid with a new address |
| A borrowed parameter of the caller, the receiver, a context parameter or a composable lambda's parameter, and any field, index or `Heap` dereference of its referent | the parameter's own version, passed down | the referent's parts are unchanged while the referent is |
| A capture local of a composable closure standing for such a reference | the version captured with it (D6) | as for the captured reference |
| Anything else: a local or temporary of the caller (`val s = "a" + "b"`, `"Count: $n"`, a `remember`ed `Copy` value copied out), a call's result, a safe navigation | none (0) | the referent lives in the caller's frame, or comes from code the transform does not see |

A different address or version means recompose; equal address and version mean the referent is the same bytes, unchanged. There is no fallback comparison of contents, nothing is cloned and nothing hashed: the two words are kept and compared.

The user's direction names the parameter itself ("the parent's identity passed down unchanged"). Extending it to parts of the referent (`Title(todo.title)` in `TodoRow`) is autonomous: the part's address is computed at run time from the parameter, and its data is unchanged whenever the parameter's referent is, so the parameter's version holds for it.

**Compile-time cost.** One classification per assignment of a reference local and per argument (D5), constant each.

Rejected:
- **Address alone**: a list edited in place keeps its elements' addresses.
- **Version alone**: rows of one list share a version, and a row given another element must recompose.
- **Comparing the referent when the identity differs**: the deep comparison the user ruled out.

### D3. Versions: one per state, drawn from a counter per thread, renewed by every storing write (user direction 2026-10-19/20 for per-state versions bumped on every write; the counter and the exact events autonomous)

Each state holds a 64-bit version. The runtime's state layer keeps one counter per thread (states are per thread, `state-handles` D16), starting at 2 so that 0 means "no identity" and 1 "permanent"; a state takes the next value when it is created, and a new one on:

- a `value =` write or compound assignment that stores (an equal write keeps the old value and its version, `owned-state` D14);
- `update`, when it stores its result;
- taking an exclusive flag (a mutable borrow of the value, a `mut fun` call on it, a part write: `todos.value.add(x)`, `list[i].title = t`), when it is taken, since what the holder writes cannot be compared without a clone;
- retirement (`owned-state` D7).

A `state version<T>` operation reads it, taking the state object by shared borrow as every state operation does; it registers no reader. During a pass no state is written, so a version read anywhere in a pass stays valid until the pass ends.

A counter per thread rather than per state matters because a freed state's value can be freed, and a new state's value allocated at the same address: with per-state counters both could hold version 2, and a row kept from the old state would skip for the new one. With one counter, a version names one state in one period between writes.

**Per-element versions are deferred.** A structural change of a list (adding, removing, reordering) gives the list's state a new version, so every row over it recomposes once, and only once: the next pass sees equal identities again. Element versions (a version per slot of a `mutableStateListOf`-like list) would let an append rerun one row; they need a list type the runtime versions per element, so they wait for a decision of their own. The user may revisit this (review notes A1: "possible refinement (open)").

**Compile-time cost.** None. **Run-time cost.** One counter increment and one store per storing write or exclusive flag; 8 bytes per state.

Rejected: per-state counters from 0 (the reuse case above); versions bumped by reads; a version per write site; per-element versions now.

### D4. Versions are passed as one hidden word per borrowed parameter after the change words (autonomous)

The address is the argument itself, so only the version is added: every composable, function or composable lambda, takes after its composer and change words one `Int` **version word** per parameter that is a shared borrow (or a nullable of one), of any type (borrowed state objects included since the review, D21; they keep `owned-state` D13's identity, D11), in parameter order, receiver and context parameters first as they are among the parameters. Every composable call passes them, through a function value too, since a composable function type determines them from its parameter types alone: a caller in another file knows them from the callee's signature, and the ABI hash is unchanged (it already covers the parameter types). Composable lambdas take them although they never skip, so a lambda forwards its parameter's identity (`Each(items) { s -> Show(s) }`).

**Compile-time cost.** One test per parameter type of a composable function type, when composing its body and its calls.

Rejected:
- **A side table** keyed by the argument's address, filled by callers: a hash insertion per argument and a lookup per kept parameter, and nothing would let the callee tell a stale entry from a fresh one.
- **Fat references** (a `&T` that carries its version everywhere): doubles every reference in every function for a compose-only fact.
- **One version per call**: rows called with several borrows from different sources would lose all identities when one has none.
- **Passing versions only to functions**: lambdas could not forward them, and every row written inside `key(...) { }` would lose its identity (D6).

### D5. The classification: one walk per composable body, with version locals beside references (autonomous)

In a composable body (function or closure), the transform first collects, in one scan, the reference-typed locals whose value reaches a borrowed argument of a composable call or a reference capture of a composable closure, following each such local's assignments backwards through the reborrows, copies and projections they come from (a worklist over the assignments, each visited once). Each such local gets a **version local** (`Int`), assigned at every assignment of the local, in the same block, from the classification of the assigned value (D2): a constant 0 or 1, the version local of the reference it is derived from, a `$version` parameter or capture, or, right after a `state read` whose result is classified, the result of a `state version` call on the same state object operand. Because a version local is assigned wherever its reference is, they agree on every path, loops and branches included (`if (c) &a.value else &local` gives 0 on one path and the state's version on the other). At a composable call, each version argument is the version local of the argument's reference, or the constant its classification gives.

**Compile-time cost.** Linear in the body: one scan to index assignments of reference locals, a worklist visiting each assignment at most once, then constant work per classified assignment and per argument during the existing replay. A new compile-timing shape, `borrowed-arguments`, covers `n` composables each passing borrows of a state's contents, of a parameter's fields and of a local, through a loop and a `key` closure.

Rejected:
- **A data-flow analysis** over the body's control-flow graph: the same answer at a higher cost, since a version local assigned beside its reference already follows every path.
- **Reading the version at the call**: the state object's borrow may be dead there, and a reference does not know its state.
- **A version local for every reference**: work and words for references that never reach a composable.
- **Following elision through calls** (`fun first(s: &State<List<T>>): &T` keeps `s`'s identity by elision rule 1): the version would have to be that of the argument, which a plain function cannot return. A follow-up, if real code needs it.

### D6. A composable closure captures the version of each reference it borrows from its author (autonomous)

Content lambdas are separate bodies: in `Column { for (todo in todos.value) { key(todo.id) { TodoRow(todo) } } }`, `todo` lives in the `Column` lambda and `TodoRow` is called in the `key` lambda, through a capture. So when the transform composes a composable closure whose captured path is a dereference of a reference (the capture local is the reference, `path-captures` D1), it appends one hidden capture to the closure value, an owned copy of that reference's version local (or the constant its classification gives), and to the closure body a capture local of type `Int` that the body's classification uses for the captured reference. Owning closures (handlers) are not composable and get nothing. The parent and the closure are in the same file, composed in one call of `compose_file`, so the transform settles each composable closure's hidden captures from its capture list before it composes any body.

**Compile-time cost.** One test per capture of a composable closure; one more `Int` in its environment, which lives in the creating frame.

Rejected: losing identities inside lambdas (every row of every app is inside a `key` or content lambda, so no row would skip); inlining `key`'s content into its caller (changes groups, keys and hot-reload contracts).

### D7. A kept identity is one slot of type `&T`, two words, compared by its words (autonomous)

A skippable body keeps each borrowed argument in one slot of its restart group whose slot type is the parameter's reference type `&T`: 16 bytes, the address and the version, with no drop entry and an equality entry comparing both words; its record carries `&T`'s fingerprint, so a reload that changes `T` makes the slot mismatch and reset. It is kept by the existing `prism_compose_keep` with the bits its caller passed (uncertain compares, different stores, same and static read past it). `F#restart` reads it and calls `F` with the address and the version. Composition dumps print it as `slot &T`.

**Compile-time cost.** One slot type record per borrowed parameter type per unit, as for any kept type.

Rejected: two slots per parameter (two keeps); a new runtime entry (the generic keep already takes an address and a record); an untyped two-word slot (a reload changing `T` would keep a restart reading the old layout).

### D8. A borrowed argument without identity runs the call in a replaceable group (autonomous)

A restart function reads the kept address later, in another pass, so it must never keep the address of a local or temporary of the caller's frame. The callee's prologue therefore tests each version word, beside the existing keepability test of function values: when one is 0, the call runs in a replaceable group of the same key, exactly as a call with an unkeepable handler does today (`compose-transform` D7): the body runs, nothing is kept, it is not a restart scope, and the state it reads registers the nearest restart scope above it. A version of 0 passes down to the callee's own calls, so the whole subtree that borrows from the caller's frame behaves this way. `Text("Count: ${c.value}")` is such a call: it always ran, and it still does.

Since the decision depends on the argument, a call site whose argument alternates between having an identity and not (`Show(if (c) &a.value else &local)`) switches between a restart and a replaceable group and recreates its group each time, as unkeepable handlers already do; it is rare and costs state only of that call.

**Compile-time cost.** One test per borrowed parameter in the prologue.

Rejected:
- **A restart group without a restart function**: the runtime would need a new kind of scope that redirects invalidations to its parent.
- **Rejecting such calls**: `Text` with a template is the most common call in UI code.
- **Copying the referent into the slot**: the clone the user ruled out.

### D9. Restarting from a kept borrow is memory-safe (autonomous: the argument the implementation must preserve)

A scope restarted alone dereferences its kept address. That is safe because, by D2 and D8, a kept address always comes from one of three sources:

- **A state's contents.** The reference was derived from a `state read` during a pass, which registered the innermost restart scope `R` open at the read as a reader. Everything the reference flows to is called below that point, so every composable that keeps it is `R` or a descendant of `R`. Any write that can move or change the data gives the state a new version and invalidates `R`; a pass recomposes invalid scopes parents first and never recomposes a scope its parent's recomposition has composed or removed (compose, "Invalid Scopes Recompose in Tree Order"). So before any descendant restarts alone, `R` has rerun and either passed it fresh arguments (a new version, so the body runs with them) or stopped calling it (its group is removed). Retirement invalidates readers too, and the cell is freed only after the next pass.
- **A remembered slot.** It lives as long as the remembering group, whose descendants are the only keepers. A slot is replaced only while the remembering body runs (a changed key, a reset after a reload), which allocates the new value while the old one still exists (removals are deferred), so every keeper called with the new value sees a new address, and every keeper not called is removed.
- **A top-level value**: valid for the whole run, replaced storage included.

The transform's tests pin this with restarts after list reallocations (tasks, group 7).

### D10. Change bits stay as they are, with two additions (autonomous)

A borrow of a top-level `val`, or of a part of it named by fields, is static (its address and contents never change in the same code); an element chosen by an index is uncertain (found by the review: `Show(Items[i])` with `i` a parameter was static, and `Show` skipped with the old element when `i` changed). A reborrow or copy of a whole borrowed parameter forwards the caller's bits for it, as copies of parameters do today. Any other borrowed argument, parts of parameters included, is uncertain, so the callee compares the two words. In the pass after a reload, every kept identity is compared whatever its bits (`compose-hot-reload` D17, unchanged).

Rejected: forwarding the parameter's bits through projections, since an index that changed between passes (`items[sel.value]`) would be trusted as same.

### D11. Borrowed state objects keep `owned-state`'s identity (autonomous)

A `&State<T>` or `&MutableState<T>` parameter stays kept as its weak word and compared by cell (`owned-state` D13), not by a version (it takes a version word since D21, which it does not use): a state object moves with its container (a list of `Todo`s reallocated by an append), but its cell does not, so the cell is the better identity. Writes to the state's value do not change it: they invalidate the scopes that read it.

### D12. The equality glue of kept values: nothing becomes dead (autonomous; an audit the brief asked for)

After this change, kept values are compared in three ways: `Copy` values by their type's equality glue (bounded: a `Copy` type has a fixed size and no heap); function values by descriptor and environment equality, whose environments hold only `Copy` captures and weak words (bounded, `path-captures` and `owned-state`); borrowed states and borrowed values by their words. No kept value is a non-`Copy` value compared through its glue, so no deep comparison remains on the skipping path, but the function-value path through `Changed` and the slot type's equality glue is still used by handlers, and cells still use their records' glue for equal writes (`owned-state` D14). The implementation checks this and removes any glue that turns out unreferenced (task 6.2).

**Audit (task 6.2).** The composed MIR names four comparison sites: `remember`'s keys (`Changed` of a `Copy`, borrow-free key type, rewrite.rs `remember`), a function parameter's uncertain path (`Changed` of its function type, whose equality glue compares descriptor and environment, `keep_parameter`), a `Copy` or weak-handle parameter (`Keep` of its type or `WeakReference<S>`, compared by their equality glue: fixed-size values, words for the handle), and a borrowed parameter (`Keep` of `&T`, compared by the identity glue). No kept value is a non-`Copy` value compared through its glue, and no identity record names glue of its referent (test "The record of a kept borrow"). Cells still name their value type's equality glue for equal writes (`owned-state` D14), and `==` on references in program code still uses the referent glue of `equality.rs`, so no glue became unreferenced and nothing was removed.

### D13. Hot reload needs no new rule (autonomous)

Identities are data, not code, so a reload changes none. A reload that edits a call to pass another value gives another address (`Label(&a)` to `Label(&b)`), and the reload's pass compares every kept identity (D10). An edited top-level initializer gets new storage at a new address (`hot-reload`, "Top-Level Values Are Kept Unless Their Declaration Changed"). A reload that changes a referent's type changes the fingerprints of `&T` in the composable's contract, so its restart scopes recompose from their parent and reset their kept slots (compose, "A Scope Whose Composable's Signature Changed Recomposes From Its Parent"), and the identity slot's record no longer matches (D7).

### D14. `dump recompositions` counts how often each composable ran and skipped (autonomous)

Proving that a toggle reruns one of 1,000 rows needs a count, not 1,000 `println` lines. A headless event script's `dump recompositions` prints `recompositions`, then one line per composable function of the program's own files (not of the bundled library) that ran or skipped since the run started or the previous `dump recompositions`: two spaces, its declaration path as `prism dump-mir` names its body, ` ran <n> skipped <m>`, sorted by path. It is Compose's recomposition and skip counts (Layout Inspector), deterministic, and stable when the library changes (`library-in-prism` and `layout-in-prism` will rewrite the toolkit's composables).

The runtime counts per body key while counting is on: a body group's start counts a run, its `skip to group end` turns it into a skip. Counting is switched on before the app starts only when the script holds the command, so other runs pay one predictable branch per body group. The CLI names keys from the program's summaries: `FileSummary::composable_bodies` gains each composable function's declaration path beside its key (composable closures stay unnamed and uncounted).

**Compile-time cost.** A path per composable function in its file's summary.

Rejected: `println` in the example (1,000-line snapshots, and the example would print); counts in composition dumps (every dump fixture would change); counting library composables (snapshots churn with every toolkit change); counts in `--frame-stats` (its other numbers are timings, compared by benchmarks, not snapshots).

### D15. `pause <milliseconds>` paces scripts like a person; the benchmark uses it (F4 option (b), recommended by the review notes; the command autonomous)

A headless script's `pause <milliseconds>` sleeps the program thread. The time counts in no frame (frame time is busy time, ui "Frame Time Is Busy Time"), so a paced script measures what a frame costs when it starts on an idle, clocked-down core, as a person's input does (review finding F4: the same 2 ms of work takes 3.3 to 3.8 ms after a 20 to 100 ms sleep). `app_frame_timing.rs` paces the todo app's toggles and keystrokes 100 ms apart; it still reports back-to-back toggles, without a target, to show the clock's effect.

Rejected: sleeping in the benchmark harness (the app reads its whole script at start); a `--pace` option for every command (pacing is per script line in real sessions); keeping a busy core (wastes power and breaks the idle checks).

### D16. Frame budgets (autonomous)

On a release build, headless, with inputs 100 ms apart: toggling one item of `large.prism` SHOULD take under 1 ms of recomposition in the median (was 4 ms, back to back) and under 4 ms of frame time at the 99th percentile; typing one character into its field SHOULD take under 2 ms of recomposition in the median (the screen reruns its loop of 1,000 keyed calls, each a skip) and under 4 ms of frame time at the 99th percentile. The counter's targets are unchanged. The apply measures and records the numbers in this design; a target missed by the implementation is reported, not loosened silently.

**Measured (tasks 10.2 and 10.3, 2026-10-20, release build, M4 Pro, `app_frame_timing` with five runs of 100 inputs each).** Before this change (review findings F3 and F4, with every row rerunning): a toggle's recomposition took 1.9 ms median headless back to back and 6.2 to 7.3 ms in a window, input to present 12.9 ms median and 15.2 ms p99 in the window. After:

| `large.prism` | events | recomposition | display list | frame median / p99 |
|---|---|---|---|---|
| toggle, back to back (no target) | 0.063 ms | 0.010 ms | 0.091 ms | 0.171 / 0.305 ms |
| toggle, paced 100 ms | 1.630 ms | 0.239 ms | 0.750 ms | 3.031 / 5.997 ms |
| typing, back to back (no target) | 0.000 ms | 0.14 ms | — | 0.24 / 0.50 ms |
| typing, paced 100 ms | 0.017 ms | 2.245 ms | 1.171 ms | 3.457 / 5.865 ms |
| toggle in a window (`app_windows` `large`, 20 toggles) | 1.892 ms | 0.228 ms | 1.057 ms | 3.555 / 6.947 ms; raster 4.358 ms; input to present 7.897 / 9.331 ms |

Targets: toggle recomposition median under 1 ms is **met** (0.239 ms paced); toggle frame p99 under 4 ms is **missed** (6.0 ms); typing recomposition median under 2 ms is **missed** (2.245 ms); typing frame p99 under 4 ms is **missed** (5.9 ms). The counter's targets, the idle frames, constant live allocations and the first frames (31.0 ms cold, 3.2 ms warm) are met. The misses are the clocked-down core of F4, not work: the same toggle frame takes 0.17 ms back to back and 3.0 ms paced, and its `events` stage alone (hit-testing and running the handler, unchanged by this change) grows from 0.06 ms to 1.6 ms; typing's recomposition, the screen's loop over 1,000 keyed calls that each skip, takes 0.14 ms back to back and 2.2 ms paced. The targets are left as the design set them and reported, not loosened; making a paced frame fit 4 ms needs less work per input outside recomposition (the event dispatch and the display list) or the per-element versions of D3, which would let typing skip the loop's bodies, both follow-ups.

### D17. The todo example: rows read their own state (autonomous)

`owned-state` D20 gave each row a `DoneBox(done: &MutableState<Bool>)`, the only reader of an item's state, because a row taking `&Todo` was not a restart scope. Now it is, so `DoneBox` folds back into `TodoRow`, which reads `todo.done.value` itself, as Compose code would: a toggle reruns that row (and `LeftCount`, which counts the done items), typing reruns `TodoScreen` and skips every row, and adding an item reruns every row once. Three sessions on `large.prism` record this with `dump recompositions` (example-apps, "Rows Recompose Only When Their Item Changes").

Rejected: keeping `DoneBox` (no longer needed; one more composable per row).

### D18. Costs (autonomous summary)

- **Compile time** (measured: `borrowed-arguments` at 256, 1,024 and 4,096 composables takes 8.0, 33.2 and 153.9 ms in `compose`, x4.2 and x4.6 per x4, and every other shape stays within its bounds; its `inst` stage grew x5.4 and x6.3 per x4, 3.0 ms at 4,096, from scanning every block of the composed file, whose blocks outgrow the caches; the review made instance collection read only the blocks of bodies that request instances, 0.24 ms at 4,096, with a warm benchmark asserting x4.6 per x4): one test per parameter type; one scan and worklist per composable body over its reference assignments; constant work per classified assignment, per argument and per composable closure capture. No pass is added; the transform stays one walk per body plus a linear pre-scan. Shape `borrowed-arguments` in `compile_timing.rs`, near-linear.
- **Run time**: per borrowed argument, one more word passed and, when kept, one `keep` call comparing two words; per state read that feeds a borrowed argument, one `state version` call (a handle validation and a load); per storing write or exclusive flag, a counter increment. No allocation anywhere. A skipped row costs a group start and end and one keep per parameter, as a skipped `Copy`-only composable does today.
- **Memory**: 8 bytes per state; 16 bytes per kept borrowed parameter; 8 bytes per hidden capture, in the creating frame.

### D19. A remembered value that is not `Copy` gets a new block whenever its calculation runs (autonomous; found by the reconciliation)

D2 calls a remembered value permanent because a new value lives at a new address while the old one still exists. The runtime did not guarantee that: `store` overwrote a slot in place when the new value had the slot's type and no drop entry, which a move-only struct without owned storage (`struct Pick { val id: Int }`) has, so `remember(n) { Pick(n) }` given to `Show(p: &Pick)` would have kept an identity that the new value matched, and `Show` would have skipped with stale data. The value `remember` lends is now always stored through `prism_compose_remember_slot`, which puts it in a new block and forgets the old slot onto the pass's release list, as it did for values with a drop entry. Kept arguments and `Copy` remembered values (copied out, so never lent) still overwrite in place, so steady-state passes still allocate nothing. The scenario "A remembered value replaced by its key is passed again by its parent" pins it.

**Compile-time cost.** None. **Run-time cost.** One allocation when a lent value's calculation reruns (a changed key or a reset), as for values with a drop entry today.

Rejected: a version per `remember` slot passed beside the borrow (a word per lent value and a new operation, for the rare drop-less move-only value); deciding in place by a flag in the slot type record (a record layout change for one case).

### D21. Every borrow parameter takes a version word, borrowed state objects and mutable borrows included (autonomous; found by the review)

D4 first gave version words only to borrows of types without a capture function, so a borrowed state object took none. That made the calling convention depend on the referent, which substitution changes: `compose fun <T> Each2(items: &State<List<T>>, item: compose (&T, &String) -> Unit)` is composed once and passes two version words in its call of `item`, while the lambda `{ st, s -> Show(s, n) }` given for `T = MutableState<Int>` was composed with `&MutableState<Int>` and took one. The lambda read `x`'s version as `s`'s, so `Show` kept a borrow of `Each2`'s local with a version that was not 0 and its restart read a dead frame (a segmentation fault; fixture "A composable lambda forwards its parameter's identity", form `through-a-generic-function-type-instantiated-with-a-state`). The same mismatch came from the conversion of a function value taking `&T` to a function type taking `&mut T` in place (type-system, "Function Values Convert Only Without Changing Representation"): `Host(item: compose (&mut String) -> Unit)` passed no version word, and the lambda `{ s -> Show(s, n) }` written as `compose (&String) -> Unit` read one that was never passed (form `converted-to-take-a-mutable-borrow`). Whether a parameter takes a version word now depends only on its type's shape: every borrow, shared or mutable, and nullable of one. A mutable borrow has no identity (D1): callers pass `0` for it, and its body never uses the word. A body whose parameter is a borrowed state object takes the word and does not use it either: it keeps the cell's identity (D11), never takes the replaceable path for it, forwards the parameter as no identity (`0`, as before), and its restart function passes `0` for it. So behaviour is unchanged for programs that worked; only the signatures of composables with borrowed state or mutable borrow parameters gain a word.

**Compile-time cost.** None (one shape test per parameter type instead of a shape test and a capture-function lookup). **Run-time cost.** One more word passed per borrowed state or mutable borrow argument.

Rejected: deciding the words of a lambda from the function type it is given for (the lambda can reach a generic through variables and fields, so it is not local information); no version word for `&T` with `T` a type parameter (the reverse mismatch, with `T = String`); keeping a borrowed state's version for generic callees (a slot per state parameter for a rare case).

## Implementation Notes

Decisions refined while implementing, beside the decision each refines:

- **D5: fixed versions need no local** (autonomous). The analysis resolves each reference local that reaches a borrowed argument or a hidden capture: when every assignment gives the same constant (`0` or `1`) or the same version the body was given (a `$version` argument or a hidden capture), that operand is used directly, so `Show(local)`, `Show(Title)` and `Show(p)` pass `const 0`, `const 1` and `copy $version0` with no version local; only a local derived from a `state read`, or assigned versions that differ (two branches), gets a version local, assigned beside each assignment. Results of calls, `remember` and destructurings count as assignments (none, permanent, none), a cycle of assignments is not fixed, and a reference borrowed mutably as a whole, captured mutably, or an argument or capture assigned has no identity. Version locals start at `0` in the prologue, so a read on a path where their reference was never assigned is defined. Rejected: a version local for every reached reference (the design's first shape: more locals and copies for the common case of temporaries assigned once); aliasing a once-assigned local's version to its source's version local (wrong when the source is reassigned in a loop before the use).
- **D6: which captures get a hidden version** (autonomous). A composable closure built in a composable body gets one hidden `Int` capture, named `$version(<path>)`, per capture that holds a reference (not captured mutably, nor assigned in the closure), and per borrowed capture that stands for a place reached through a reference of its author, or for a part of such a hidden-place capture of an enclosing closure; owned captures that are not references and weak captures get none. A closure built in a body that is not composable (`composition { ... }` in `main`) gets none, so its borrows have no identity, which is safe and what a frame's data needs. Deciding it is one scan of the file's closure values and constant work per capture, before any body is composed.
- **D6: borrowing closures borrow their versions** (found by the benchmark). Hidden captures were first owned copies, which made every key lambda's environment an allocation (the backend allocates any environment with an owned capture beyond one word): typing into the 1,000-item field allocated one environment per row per pass (1,006 allocations a frame). A closure that borrows its captures, every content lambda, now borrows the local holding the version (its author's version local, `$version` argument or own hidden capture, or a temporary for a constant), so its environment stays in the creating frame and allocates nothing (4 allocations a typing frame, those of the text field), and a value built in an earlier iteration of a loop reads a reference and its version as they are now, as it already did the reference. A closure that owns its captures keeps owned copies.
- **D7: the operations of a kept identity** (autonomous). `Keep(&T)` of an identity takes `(composer, reference, version, passed bits)`, and the backend stores the two words in a 16-byte stack slot for `prism_compose_keep`; the restart function reads the reference with `KeptParameter` (one word, as before) and the version with a new `ComposerOperation::KeptVersion { index, ty }`, which reads the slot's second word. The record of an identity type (`&T` or `(&T)?`, the only slot types that are references) has size 16, no drop entry, and generated glue `<unit>#identity(<type>)` comparing both words. Rejected: a two-word MIR value (MIR has no pair type, and the identity exists only in the slot).

- **D14: where the names live** (autonomous). `FileSummary` gains `composable_functions`, each composable function's body key and declaration path (generic ones included, since their instances start their groups with the generic's key), beside the unchanged `composable_bodies`, so hot reload's use of keys is untouched. The CLI gathers the program's own files' entries (modules outside `prism/`) into `prism_ui::host::set_recomposition_names`; an app counts only when the configured headless script holds `dump recompositions`, enabling `prism_compose::counts` for those keys before its first composition and disabling it when it ends. A run counts on a restart or replaceable group start of a counted key, and `skip_to_group_end` turns it into a skip; a pause is `unmeasured`, so its time is in no frame.

## Risks / Trade-offs

- [A structural list change reruns every row once] → accepted for now (user, A1); per-element versions are the recorded follow-up.
- [A borrow through a plain function's result never skips (D5)] → the idiom is to index or iterate the state's value in the composable; elision-based propagation is a follow-up if needed.
- [Library composables called with temporaries (`Text("Count: $n")`) still always run] → unchanged from today, and cheap (a text node update).
- [The memory-safety argument (D9) rests on tree-order recomposition and deferred removals, which a future change could weaken] → D9 is promoted to the compose Design Rationale at archive time, and the tests of group 7 restart scopes after reallocating and after removing the data they borrowed.
- [Hidden captures make every composable closure that borrows a reference one word larger] → in the creating frame, no allocation.
- [The version words change every composable's machine signature that has a borrowed parameter] → build-cache entries are keyed by the compiler's hash, so nothing stale is loaded; hot-reload contracts derive from parameter types, which do not change.
- [`dump recompositions` names depend on declaration paths, which a refactor renames] → the sessions are snapshots, re-recorded like any other.

## Migration Plan

Behaviour only: no source changes for programs (composables with borrowed parameters start skipping, which is observable through `println` in bodies, as Compose's skipping is). The todo example drops `DoneBox`. Fixtures whose composition dumps show a borrowed composable's group now print `restart` and a `slot &T` where they printed `replaceable`; each is re-recorded and read. Rollback is a revert of the change's commits.

## Open Questions

- Whether `state version` should be fused into `state read` when both are emitted (one call instead of two); measure in `app_frame_timing` first.
- The exact dump name of identity slots (`slot &String`) may be tuned at implementation if it collides with another slot name.
