# Proposal: strong-skipping

## Why

A composable that takes a borrowed parameter never skips today (`compose-transform` D7, `compose-architecture` D9): `TodoRow(todo: &Todo)` reruns for every item whenever its list's screen recomposes, so typing one character into the todo app's field recomposes all 1,000 rows of `large.prism` (review finding F3). Borrows are how Prism passes data without cloning, so this hits most real composables, `Text(todo.title)` included. The post–Milestone 2 review agreed item **A1** (`docs/milestone-2-review-notes.md`, "Agreed changes"; user, 2026-10-19 and 2026-10-20): no stability inference at all, as with Compose's strong skipping, so every restartable composable can skip; `Copy` parameters compare by value; a borrowed parameter carries an identity that the compose transform derives syntactically at the call site, and a changed identity recomposes, with no deep comparison, nothing cloned and nothing hashed. It is item 5 of "After Milestone 2: the review round" in `ROADMAP.md`, and builds on `path-captures` (handlers copy `todo.id`) and `owned-state` (state objects in data, remembered owned values, borrowed states compared by identity), both proposed and not yet archived.

## What Changes

- **Every restartable composable can skip** (user direction). A composable returning `Unit` whose parameters can all be kept is skippable and a restart scope: `Copy` values (compared by `==`, as today), `escaping` and `attached` function values (by closure and captures, as today), borrowed state objects (by their cell, `owned-state`), and now **any shared borrow** `&T` or `(&T)?`, kept as its identity. `&mut` parameters, by-value non-`Copy` parameters and composable content lambdas still make a composable unskippable. **BREAKING** (behaviour only): composables such as `TodoRow(todo: &Todo)` and the toolkit's `Text(text: &String)` become restart scopes and skip.
- **A borrowed argument's identity is its address and its source's version.** The compose transform classifies each borrowed argument by where its reference comes from, in one syntactic walk of the caller's MIR: a state's contents (`todos.value[i]`, a `for` variable over them, a field of either) carry the state's current **version**; a remembered value and a top-level `val` are *permanent* (their storage never moves or changes while a kept identity can name it); a borrowed parameter of the caller, or a part of its referent, passes the caller's own version down; anything else (a local, a temporary such as `"Count: $n"`, a call's result) has **no identity**.
- **Versions.** Every state gets a version when it is created and a new one on every write that stores (`value =`, compound assignments, `update`) and on every exclusive flag (`todos.value.add(x)`, part writes): a 64-bit number from one counter per thread, so a version is never repeated, even by another state whose value lands at a freed address. Equal writes keep the version. Per-element versions are deferred: a structural change of a list (adding, removing) recomposes every row once (open for the user to revisit).
- **The calling convention.** A composable, function or lambda, takes one hidden version word per borrowed parameter after its change words; a call passes the version its classification gives (`0` for none). A composable closure that borrows a reference from its author gets that reference's version as a hidden `Int` capture, so `key(todo.id) { TodoRow(todo) }` and `Column { ... }` keep identities. A restart function passes the kept version.
- **Skipping and restarting.** A skippable body keeps a borrowed argument as its identity, two words in a slot of type `&T`, and compares them; a changed address or version runs the body. A call with a borrowed argument that has no identity runs in a replaceable group, never skipped and not a restart scope, so no restart ever reads a borrow of a frame that is gone. The change bits are unchanged: a borrow of a top-level `val` is static, a reborrow of a parameter forwards its bits, everything else is uncertain and compared.
- **Hot reload.** The pass after a reload compares every kept identity, as it compares every kept argument; replaced top-level storage has a new address; a changed referent type changes the composable's contract, whose scopes reset (unchanged rules).
- **Tooling.** Headless event scripts gain `dump recompositions` (per composable of the program: how many times it ran and was skipped since the previous dump, like Compose's recomposition counts) and `pause <milliseconds>` (to pace inputs like a person, review finding F4 option (b)).
- **Examples and targets.** The todo app's rows read their own `done` state (`owned-state`'s `DoneBox` folds back into `TodoRow`, which is now a restart scope). New sessions on `large.prism` prove that a toggle recomposes one row, typing skips every row, and adding an item recomposes each row once. `app_frame_timing.rs` paces the todo inputs 100 ms apart and tightens the toggle target to under 1 ms of recomposition, with a typing target beside it.
- **Not in scope:** per-element versions of lists (`mutableStateListOf`-like stamps); identity through calls returning borrows (elision could carry the argument's version; a follow-up); skipping with by-value non-`Copy` parameters or content lambdas (they cannot be restarted without a clone); memoizing composable lambdas.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `compose`: restartable composables and the parameters they keep (renaming the two "stable" requirements), borrowed identities and versions, replaceable calls for borrows without identity, what callers say of borrowed arguments, the reload pass comparing identities, the transform's linear cost.
- `mir`: composed bodies taking version words, calls passing them, `state version`, hidden version captures, kept identities in skippable bodies and restart closures.
- `code-generation`: composable signatures with version words, slot type records of kept identities.
- `ui`: `dump recompositions` and `pause` in event scripts; pauses are not frame time.
- `example-apps`: the large todo sessions proving skipping; paced frame budgets.

## Impact

- `crates/prism-compose-transform`: `stability.rs` (restartable parameters, borrowed ones), a new `identity.rs` (the classification and version locals), `rewrite.rs` (version arguments at calls, hidden captures, the prologue's identity test and keep), `restart.rs` (kept identities), `lib.rs` (file-level pass over closure captures).
- `crates/prism-mir`: `StateOperation::Version`, the identity slot type, codec, validator and dump.
- `crates/prism-compose`: per-cell versions and the thread's counter (`state.rs`), the recomposition counts (`composer.rs`), entry points (`entry.rs`, `functions.rs`).
- `crates/prism-codegen`: identity slot records and their equality glue (`glue.rs`, `equality.rs`, `lower.rs`), the version entry.
- `crates/prism-ui`: `script.rs` (`dump recompositions`, `pause`), `headless.rs`, `statistics.rs`; `crates/prism-cli`: naming body keys for the counts (`FileSummary::composable_bodies` gains names).
- `crates/prism-cli/tests`: fixtures under `compose/`, `mir/`, `code-generation/`, `ui/`; `compile_timing.rs` shape `borrowed-arguments`; `app_frame_timing.rs`.
- `examples/apps/todo/screen.prism` and three new sessions; `docs/architecture.md` §5 (`compose-architecture` D9 revised).
