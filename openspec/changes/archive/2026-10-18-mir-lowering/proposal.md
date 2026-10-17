# Proposal: mir-lowering

## Why

The typed tree (`typed-tree`, step 5) records every type-checking decision, but it is still a tree: control flow is nested, temporaries only say "until the end of the enclosing statement", nothing says when a value is dropped, and a default value is an expression of the callee's body. The borrow checker (`borrowck-moves`, `borrowck-exclusivity`, `borrowck-elision`, steps 7 to 9) needs a control-flow graph over places, with explicit storage and drops, to compute maybe-moved values, liveness and borrow regions; the Cranelift backend (steps 14 and later) needs the same graph with every local typed. This change, step 6 of "the road to the borrow checker" in `ROADMAP.md`, adds that graph. It implements the scenario "Lowering to MIR" of `docs/architecture.md` §1, and settles the questions `typed-tree` left to it: where temporaries live and die, how a default value is evaluated at run time, how a default that borrows a temporary is passed, and how an optional mutable borrow lent as an optional shared borrow is reborrowed. No program changes meaning and no diagnostic is added.

## What Changes

- **A new `prism-mir` crate** lowering the typed tree of an error-free file to MIR: one body per function, top-level `val`, closure and default value, each a list of locals (all typed) and basic blocks. Statements assign rvalues to places (a local plus deref, field, payload and index projections) and mark storage; terminators branch, call, drop and return. It is small on purpose: no unwinding edges, no assertions, no SSA.
- **Explicit storage and drops.** Every local and temporary is marked live where its scope starts and dead where it ends. Every value that may own resources gets a drop where its scope ends, on every exit path (fallthrough, `return`, `break`, `continue`, a `null` leaving a `?.` chain), and before an assignment overwrites it. Drops are not yet refined by moves: `borrowck-moves` computes which values are maybe moved, and drop elaboration (`runtime-strings-and-drops`) turns these drops into static or flag-guarded ones.
- **Temporary scopes.** A temporary lives until the end of its statement, unless it is created in a part that is evaluated conditionally or repeatedly (a branch, a condition, a `when` pattern, the right side of `&&`/`||`, the fallback of `?:`, the rest of a `?.` chain after its first test, a block's tail), which then ends it. A temporary borrowed by `&` as a whole `val`/`var` initializer lives as long as the binding (`val r = &make()`).
- **Control flow becomes branches**: `&&`, `||`, `if`, `?:`, `?.`, `when` (a switch on the discriminant for variant patterns), `while`, `do`/`while`, `for` over a `Range`, `break`, `continue` and `return`. Code after a diverging expression is not lowered, so every block is reachable.
- **Calls** name their callee (a declaration path, the built-in `clone`, a function-typed place, or a default value's generator), pass their arguments in parameter order, and evaluate the receiver place first, then the written arguments in source order, then the receiver borrow, then the defaults. Composable calls stay marked for `compose-transform`.
- **Default values are generator bodies** that receive shared borrows of the receiver and earlier arguments; a reference-typed parameter's generator also receives a caller-owned slot, so a default that borrows a temporary (`fun greet(n: &String = "anon")`) keeps it alive until the end of the caller's statement. Both shapes follow from the callee's signature, so callers in other files need nothing more than the ABI.
- **Closures are separate bodies** whose captured bindings are capture locals; the enclosing body builds the closure value from the captured places. How each capture is taken is left to `borrowck-exclusivity`, and closure conversion to `closures`.
- **The typed tree records its file's type table** (every known struct and enum, its marker and, when known, its fields or variants), moved from the checker, so MIR, the borrow checker and the backend need neither the checker nor the ABIs to know whether a type is `Copy` or what its fields are.
- **`prism dump-mir <file.prism>`** prints the MIR as stable text; fixtures may hold `mir.snap`, re-blessed with `PRISM_BLESS=1`, and a validator checks the MIR of every passing fixture and example. `prism check` lowers every file that typechecks, so the borrow checker only has to be added to the pipeline.
- Not in scope: borrow checking, drop elaboration and drop flags, capture modes and closure conversion, overflow and division-by-zero semantics, layouts, optimizations, and caching MIR.

## Capabilities

### New Capabilities
- `mir`: what the MIR of a file holds (bodies, places, operands, storage, drops, temporary scopes, branches, calls, default generators, closures, reborrows) and the `prism dump-mir` command.

### Modified Capabilities
- `compilation-and-abi`: adds the Phase 2 requirement that each file that typechecks is lowered to MIR from its typed tree alone, the second stage of "Semantic Pipeline After Type Checking".
- `typed-tree`: the tree records the type table of its file, and its dump lists the imported types.

## Impact

- `crates/prism-mir` (new): the MIR data model, the lowering (scopes and drops, places and operands, control flow, calls and defaults, closures), the text dump and the validator. Added to the workspace `Cargo.toml`.
- `crates/prism-typed-tree`: `types.rs` gains `TypeTable` (definitions and markers, `is_copy`, `is_clone`), `TypedFile` gains `types`, and `dump.rs` lists imported types.
- `crates/prism-typechecker`: `checker.rs` keeps its definitions and markers in a `TypeTable` and hands it to the tree; `is_copy` and `is_clone` delegate to it.
- `crates/prism-cli`: `workspace.rs` lowers each file that typechecks and adds `dump_mir`; `main.rs` adds `dump-mir`; `tests/fixture_tests.rs` compares `mir.snap` snapshots and validates the MIR of passing fixtures and examples; the typed-tree snapshots of fixtures with imports are re-blessed. New fixtures go under `tests/fixtures/mir/` and `tests/fixtures/compilation-and-abi/`.
- No new error codes; `openspec/specs/diagnostics/spec.md` is unchanged.
- `docs/architecture.md`: on archive, the scenario "Lowering to MIR" moves into the specs. `ROADMAP.md`: step 6's questions and the item on `val r = &make()` are settled here; new follow-ups are listed in `design.md`.
