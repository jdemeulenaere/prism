# Proposal: borrowck-moves

## Why

Since `value-semantics`, every non-`Copy` value has a single owner and a move takes a whole value, but nothing stops a program from using a value after it moved: `take(s); take(s)` typechecks, and the MIR of `mir-lowering` still drops `s` after moving it. The type checker cannot reject this, because whether `s` still holds a value depends on the path taken through branches and loops. This change, step 7 of "the road to the borrow checker" in `ROADMAP.md`, adds the borrow checker's first stage: a `prism-borrowck` crate that checks the MIR of each file for uses of moved and uninitialized values, reporting codes in a new `E5xxx` range, and then elaborates the MIR's drops so that a moved value is never dropped and a conditionally moved one is dropped under a runtime flag. It implements "Single-Owner Value Semantics and RAII" of `docs/architecture.md` §3 (except its runtime half) and "Borrow checking on MIR" of §1, and settles the follow-ups `ROADMAP.md` assigns to it from `mir-lowering` and `extension-methods`.

## What Changes

- **A new `prism-borrowck` crate** checking a file's MIR alone: no other file's bodies, MIR or ABI. It builds move paths over places (a local plus field projections; a nullable's payload counts as the nullable), runs initialization dataflows over each body's control-flow graph, and checks every use of a place against them. `Copy` values never move.
- **Use after move (`E5001`, `UseAfterMove`)**: reading, copying, moving, borrowing, calling, capturing or assigning a field of a value on a path where it was moved, until it is assigned a new value. This covers moves in one branch of an `if`, `when`, `&&`, `||` or `?.`, moves inside a loop body that reach the next iteration, and the receiver of a `consuming fun` call. The diagnostic points at the use, labels each move that reaches it ("value moved here", or "value moved here, in previous iteration of loop"), says "possibly moved" when the value is moved on some paths only, and suggests `.clone()` for a `Clone` type or `: Copy` for a struct or enum whose members are all `Copy`.
- **Uninitialized reads (`E5002`, `UseOfUninitialized`)**: today only one program shape reaches one, a `do`/`while` condition reading a body local whose declaration a `continue` skipped (`mir-lowering`, N5); it is rejected, pointing at the `continue`.
- **Drop elaboration.** After a file passes, each scope-exit and assignment drop of the MIR becomes nothing (the value is moved on every path), stays (it holds a value on every path), or is guarded by a drop flag, a `Bool` local set where the value is initialized and cleared where it moves. `prism check` elaborates every file that passes; `prism dump-mir --elaborated <file>` prints the result. This takes "drop elaboration on MIR" from `runtime-strings-and-drops`, which keeps drop glue (what a drop does).
- **By-value receivers are evaluated first** (orchestrator decision). The receiver of a `consuming fun` call, and a `Copy` receiver copied out, is moved or copied before the arguments are evaluated, strict left-to-right as in Kotlin and Rust. Borrowed receivers keep the two-phase shape (the place first, the borrow after the arguments). **BREAKING:** `s.consume(f(&mut s))` is now a use after move instead of moving the mutated `s`.
- **A default value that consumes an earlier parameter** (`fun f(a: String, b: String = a)`) is rejected by the type checker as `E4017`, since its generator only borrows the earlier arguments; the rule depends only on the expression's shape. **BREAKING** for such defaults, which typechecked before.
- **Diagnostics with related locations.** The CLI renders a diagnostic's related locations (where a value moved) and notes after its primary location.
- Not in scope: borrow conflicts, non-lexical borrows, closure capture modes and escaping closures (`borrowck-exclusivity`); returned borrows and elision (`borrowck-elision`); partial moves and destructuring; drop glue and running destructors (`runtime-strings-and-drops`); moves of variables captured by closures (`borrowck-exclusivity`); deferred `val` initialization.

## Capabilities

### New Capabilities
None. Use after move and uninitialized reads are ownership rules, which the `ownership` spec's purpose already reserves for the borrow checker (design D2).

### Modified Capabilities
- `ownership`: adds "Use After Move Is Rejected", "Assignment Reinitializes a Moved Variable", "Use-After-Move Diagnostics Show the Moves" and "Reading an Uninitialized Value Is Rejected", and widens "Moves Take Whole Values Only" to an earlier parameter moved in a default value.
- `mir`: drops are elaborated after borrow checking (drop flags), by-value receivers are evaluated before the arguments, the `do`/`while` `continue` scenario names its `E5002`, the validator forbids moves through a dereference and knows drop flags, and `prism dump-mir --elaborated`.
- `compilation-and-abi`: the third stage of the Phase 2 pipeline, borrow checking each file's MIR from the MIR alone.
- `diagnostics`: the `E5xxx` range for the borrow checker, and related locations in rendered diagnostics.

## Impact

- `crates/prism-borrowck` (new): move paths, a bit-set dataflow framework (forward and backward, so the later borrow analyses reuse it), dominators, the initialization analyses, the use check, the diagnostics model and drop elaboration. Added to the workspace `Cargo.toml`.
- `crates/prism-mir`: `call.rs` evaluates by-value receivers first; `LocalKind::DropFlag` and `MirFile::phase`; the validator learns drop flags and rejects moves through a dereference; `dump.rs` prints drop flags and the phase; block merging becomes reusable after elaboration.
- `crates/prism-typechecker`: `check_move_source` rejects moving an earlier parameter or `this` inside a default value.
- `crates/prism-cli`: `workspace.rs` borrow-checks and elaborates each lowered file, with a `WorkspaceError::Borrow`; `diagnostics.rs` renders borrow errors with related locations; `main.rs` adds `dump-mir --elaborated`; `fixture_tests.rs` compares `elaborated-mir.snap` snapshots and validates the elaborated MIR of every passing fixture and example. New fixtures under `tests/fixtures/ownership/`, `mir/`, `compilation-and-abi/` and `diagnostics/`; snapshots of fixtures with consuming receivers are re-blessed; fixtures that only passed by using a moved value are rewritten (design D14).
- `openspec/specs/diagnostics/spec.md`: a new `E5xxx` catalog section with `E5001` and `E5002`, and the `E4017` row widened. The catalog is not a requirement, so it is edited directly.
- `docs/architecture.md`: §4's "Method Call Evaluation Semantics" moves by-value receivers before the arguments. On archive, §1's "Borrow checking on MIR" and §3's "Single-Owner Value Semantics and RAII" move into the specs, except the runtime half of "Deterministic scope exit cleanup", which goes to §4. `ROADMAP.md`: step 7 and its four follow-ups are settled, and step 16 no longer includes drop elaboration.
