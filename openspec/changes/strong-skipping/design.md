# Design: strong-skipping

## Context

See `proposal.md` for motivation. This change implements agreed item **A1** of the post–Milestone 2 review (`docs/milestone-2-review-notes.md`, "Agreed changes"): "No stability inference: every restartable composable can skip, as with Compose's strong skipping (user). `Copy` parameters compare by value. A borrowed parameter carries an identity the compose transform derives syntactically at the call site: address plus source version (a state cell's version; a top-level `val`; the parent parameter's identity passed down). A borrow of a local or temporary has none and always recomposes. Different identity means recompose, with no deep-comparison fallback and nothing cloned or kept (user: avoid expensive comparisons). A structural list change recomposes every row once; per-element stamps are a possible refinement (open). Replaces the kept-clone and kept-hash ideas." It is item 5 of "After Milestone 2: the review round" in `ROADMAP.md`, and it also takes option (b) of review finding F4 (paced benchmark inputs), as the review notes recommended "when `strong-skipping` lands". Decisions the user took are marked **(user direction 2026-10-19/20)**; every other decision the user might take differently is **(autonomous)** and logged in `docs/decisions-for-review.md`. Compile speed is a primary criterion (AGENTS.md §5): each decision states its compile-time cost.

**Baseline.** This packet builds on two packets that are proposed and not archived: `path-captures` (item 3, being implemented) and `owned-state` (item 4, proposed in `18fa18c`), which lands before this change. Their delta specs are taken as applied: captures are paths (`todo.id` copied by handlers); `MutableState<T>` is a move-only owning object whose cell never moves; `remember` keeps owned values and gives a shared borrow of the slot; `&State<T>` and `&MutableState<T>` parameters are stable, kept as their weak word and compared by identity (`owned-state` D13); `owned-state` D18 names this change's seam ("state contents by (handle, version), where every write, `update` and exclusive guard already goes through one runtime call"). The MODIFIED and RENAMED blocks of compose "Stable Parameters" and "Skipping Composables With Stable Parameters" are written on `owned-state`'s versions, which are written on `path-captures`'. The first task re-bases every block on the main specs once both are archived.

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

The address is the argument itself, so only the version is added: every composable, function or composable lambda, takes after its composer and change words one `Int` **version word** per parameter that is a shared borrow (or a nullable of one) of a type without a capture function (state objects keep `owned-state` D13's identity, D11), in parameter order, receiver and context parameters first as they are among the parameters. Every composable call passes them, through a function value too, since a composable function type determines them from its parameter types alone: a caller in another file knows them from the callee's signature, and the ABI hash is unchanged (it already covers the parameter types). Composable lambdas take them although they never skip, so a lambda forwards its parameter's identity (`Each(items) { s -> Show(s) }`).

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

A borrow of a top-level `val` is static (its address and contents never change in the same code). A reborrow or copy of a whole borrowed parameter forwards the caller's bits for it, as copies of parameters do today. Any other borrowed argument, parts of parameters included, is uncertain, so the callee compares the two words. In the pass after a reload, every kept identity is compared whatever its bits (`compose-hot-reload` D17, unchanged).

Rejected: forwarding the parameter's bits through projections, since an index that changed between passes (`items[sel.value]`) would be trusted as same.

### D11. Borrowed state objects keep `owned-state`'s identity (autonomous)

A `&State<T>` or `&MutableState<T>` parameter stays kept as its weak word and compared by cell (`owned-state` D13), with no version word: a state object moves with its container (a list of `Todo`s reallocated by an append), but its cell does not, so the cell is the better identity. Writes to the state's value do not change it: they invalidate the scopes that read it.

### D12. The equality glue of kept values: nothing becomes dead (autonomous; an audit the brief asked for)

After this change, kept values are compared in three ways: `Copy` values by their type's equality glue (bounded: a `Copy` type has a fixed size and no heap); function values by descriptor and environment equality, whose environments hold only `Copy` captures and weak words (bounded, `path-captures` and `owned-state`); borrowed states and borrowed values by their words. No kept value is a non-`Copy` value compared through its glue, so no deep comparison remains on the skipping path, but the function-value path through `Changed` and the slot type's equality glue is still used by handlers, and cells still use their records' glue for equal writes (`owned-state` D14). The implementation checks this and removes any glue that turns out unreferenced (task 6.2).

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

### D17. The todo example: rows read their own state (autonomous)

`owned-state` D20 gave each row a `DoneBox(done: &MutableState<Bool>)`, the only reader of an item's state, because a row taking `&Todo` was not a restart scope. Now it is, so `DoneBox` folds back into `TodoRow`, which reads `todo.done.value` itself, as Compose code would: a toggle reruns that row (and `LeftCount`, which counts the done items), typing reruns `TodoScreen` and skips every row, and adding an item reruns every row once. Three sessions on `large.prism` record this with `dump recompositions` (example-apps, "Rows Recompose Only When Their Item Changes").

Rejected: keeping `DoneBox` (no longer needed; one more composable per row).

### D18. Costs (autonomous summary)

- **Compile time**: one test per parameter type; one scan and worklist per composable body over its reference assignments; constant work per classified assignment, per argument and per composable closure capture. No pass is added; the transform stays one walk per body plus a linear pre-scan. Shape `borrowed-arguments` in `compile_timing.rs`, near-linear.
- **Run time**: per borrowed argument, one more word passed and, when kept, one `keep` call comparing two words; per state read that feeds a borrowed argument, one `state version` call (a handle validation and a load); per storing write or exclusive flag, a counter increment. No allocation anywhere. A skipped row costs a group start and end and one keep per parameter, as a skipped `Copy`-only composable does today.
- **Memory**: 8 bytes per state; 16 bytes per kept borrowed parameter; 8 bytes per hidden capture, in the creating frame.

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
