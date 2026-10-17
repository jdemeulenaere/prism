# Proposal: typed-tree

## Why

The type checker decides everything later stages need: what each name refers to, the type of every expression, which argument fills which parameter, and every implicit operation (`reference-access` classifies copies, copy-outs, borrows, reborrows and nullable views as an `Adjustment`; `extension-methods` classifies receiver passing as a `ReceiverAdjustment`). Today it throws all of that away: it returns only a list of errors, and both classifications are computed and dropped (`#[allow(dead_code)]`). This change, step 5 of "the road to the borrow checker" in `ROADMAP.md`, makes the checker produce a typed tree that records those decisions, so that `mir-lowering` (step 6), the borrow checker, generics, the Cranelift backend, closure conversion and the `compose fun` transform read them instead of re-deriving them. It implements the scenario "Typed tree from the type checker" of `docs/architecture.md` §1. No program changes meaning.

## What Changes

- **A new `prism-typed-tree` crate** holding the tree and the semantic `Type`, which moves out of `prism-typechecker` (the checker keeps the rules: lowering from annotations and ABIs, assignability, joins).
- **The checker builds the tree while it checks**, one node per decision, rather than a second pass re-deriving decisions. `TypeChecker::check_module` returns a `TypedFile`; errors are reported as today.
- **Per-body arenas with typed ids.** Each function and top-level `val` has a body holding its locals, expressions and statements; nodes are addressed by `ExpressionId`, `StatementId` and `LocalId`.
- **Resolved names.** Locals, parameters, `this` and loop variables are bindings of their body, distinct even when names shadow. Top-level functions, extensions and values are module-qualified declaration paths (module, receiver type for an extension, name), the same identity the ABI uses. Fields and variants are indices. Implicit `this` appears as an explicit access.
- **A type and a span on every node, and explicit conversions.** Wherever a value fits its position only by subtyping (`T` into `T?`, a `Nothing` branch, `&mut T` as `&T`), a coercion node of the expected type wraps it, so a node's type always equals the type its position expects.
- **Explicit value uses.** Places (bindings, top-level values, field paths, dereferences, indices, payloads of nullables, temporaries) are distinct nodes. A consumed place is wrapped in an explicit move or copy; a place used anywhere else is read in place. Every borrow is an explicit node recording shared or mutable and where it comes from (written, argument, receiver, conversion); a borrowed non-place is an explicit temporary; a reborrow is a borrow of a dereference; a copy-out is a copy of a dereference; nullable views are safe-navigation chains.
- **Normalized calls.** A call names its callee (a declaration path, a built-in function such as `clone`, or a function-typed value), lists its arguments in parameter order with explicit default slots, records the source order in which written arguments are evaluated, passes an extension's receiver as the first argument with its receiver borrow, move or copy, and records whether the callee is composable. Struct and variant constructions are construction nodes in field order.
- **Structured control flow.** Blocks keep their tail (a discarded tail becomes a statement); `if`, `when` (resolved patterns, an exhaustiveness flag), `&&`, `||`, `?:` and `?.` chains stay structured for MIR lowering to turn into branches. Loops are statements and every `break`/`continue` names its loop. Assignments are statements over a place target (a binding, a field path, or a dereference for `*r = v`).
- **Closures record their captures** (the enclosing bindings they use, in first-use order) and their compose flag; capture modes are left to `borrowck-exclusivity`.
- **Error recovery.** The tree is produced even when the file has type errors: where checking fails, an error node of the error type keeps the subexpressions that were checked. A file without type errors has no error node and no error type.
- **`prism dump-typed-tree <file.prism>`** prints the tree as stable indented text, checked against its imports' ABIs only. Snapshot fixtures (`typed-tree.snap` next to a fixture's `main.prism`, re-blessed with `PRISM_BLESS=1`) test it, and a validator checks the structural invariants of the tree of every passing fixture.
- **Plain fix found while proposing:** an `if` without `else` checked against a non-`Unit` type is checked twice today, so an error inside its branch is reported twice. It is checked once now. Implementing found three more observable changes, recorded in `design.md` (D17, D21): error nodes have the error type, which suppresses some follow-on mismatches; `(&T)?` renders as written in diagnostics; and a range pattern in a `when` without subject, accepted unchecked until now, is `E4002`.
- Not in scope: MIR (step 6) and anything it decides (drop elaboration, temporary scopes beyond "end of the enclosing statement", how defaults are evaluated at run time); capture modes; generics (step 10 adds type arguments to calls); caching or serializing trees; parser error recovery.

## Capabilities

### New Capabilities
- `typed-tree`: what the typed tree records for every file (resolved names, types, coercions, moves, copies, borrows, normalized calls, receivers, control flow, captures, built-ins, error nodes) and the `prism dump-typed-tree` command. `mir-lowering` adds a sibling capability for the MIR.

### Modified Capabilities
- `compilation-and-abi`: adds the Phase 2 requirement that the type checker turns each file into a typed tree using only the file and its direct imports' ABIs, the first stage of the planned "Semantic Pipeline After Type Checking".

## Impact

- `crates/prism-typed-tree` (new): `types.rs` (moved from the checker), the tree data model, the text dump and the validator. Added to the workspace `Cargo.toml`.
- `crates/prism-typechecker`: `types.rs` keeps the lowering functions and the assignability rules as a trait over `Type`; `scope.rs` symbols record their binding id or defining module; `checker.rs` builds nodes in every `infer_*`/`check_*` function, maps `Adjustment` and `ReceiverAdjustment` to nodes, and checks an `if` without `else` once; a new `builder.rs` holds the body arenas, the open closures (for captures) and the open loops (for `break` targets).
- `crates/prism-cli`: `workspace.rs` splits Phase 1 and the per-file check so that one file can be checked against its imports' ABIs alone, and adds `dump_typed_tree`; `main.rs` adds the `dump-typed-tree` command; `tests/fixture_tests.rs` compares `typed-tree.snap` snapshots and validates the trees of passing fixtures. New fixtures go under `tests/fixtures/typed-tree/` and `tests/fixtures/compilation-and-abi/`.
- `crates/prism-typechecker/tests/regressions.rs`: a regression test for the duplicated diagnostic.
- No new error codes; `openspec/specs/diagnostics/spec.md` is unchanged.
- `docs/architecture.md`: §1's MIR scenario names its dump `prism dump-mir <file>`, matching `dump-typed-tree`. On archive, the scenario "Typed tree from the type checker" moves into the specs.
