# Proposal: drop-and-clone-hooks

## Why

The post–Milestone 2 review agreed that the standard and compose libraries become "as much as possible ... normal prism code that is imported like any other code" (item **A4** of `docs/milestone-2-review-notes.md`, "Agreed changes"; user, 2026-10-20), and named the language additions that needs. The second is **L2, custom drop and clone hooks**: today what dropping or cloning a value does is decided by the compiler alone (field-wise glue, plus hard-coded runtime calls for the intrinsic `Composition` and state types), so a type written in Prism that owns a runtime resource through an `Int` handle (a composition, a node, a state cell, a handle from an `external` function) cannot release it when it is dropped, nor copy it when it is cloned. The user decided that there is no `unsafe` layer for now (`List`, `String` and `Heap` stay Rust-backed), so hooks are for types wrapping runtime handles, not raw memory. This is item 8 of "After Milestone 2: the review round" in `ROADMAP.md`; `library-in-prism` (item 10) rewrites `Composition`, state and other owning library types over externals and relies on it.

## What Changes

- **Drop hooks.** `operator mut fun Session.drop() { disposeComposition(id) }`, declared in the file of a struct or enum, runs whenever a value of that type is dropped: at the end of its owner's scope, when it is overwritten, as a temporary, as a field, payload, list element, box content or closure capture, in a generic instance, and when the composition releases a remembered value or frees a state's cell. It runs **before** the value's parts are dropped (Rust's order). A moved-from value runs no hook: the existing drop elaboration and drop flags decide, so a conditional move runs the hook only on the paths where the value is still present.
- **Clone hooks.** `operator fun Ticket.clone(): Ticket`, declared in the file of a `: Clone` struct or enum, replaces its field-wise clone everywhere a value is cloned: `x.clone()`, a field of a cloned value, list and box clones, and `clone()` in generic instances. A `: Clone` declaration with a clone hook is not checked field by field, since the hook builds the copy.
- **Rules**, all checked by the type checker at constant cost per declaration or use:
  - a hook takes no visibility modifier, no parameters and no type parameters but its type's, in order; anything else is `E4035` (`InvalidOperatorDeclaration`, from `operator-conventions`);
  - a type with a drop hook cannot be `: Copy` (`E4035`), and a `: Clone` type with a drop hook must declare a clone hook (`E4035`), so a field-wise clone never duplicates a handle that a drop releases;
  - a drop hook is never called by name: `s.drop()` is the new **`E4036` `DropHookCalled`**, with help explaining that a value is dropped where its scope ends;
  - a value whose type has a drop hook is never taken apart by a pattern or a destructuring declaration that moves a part (`E4017`), since that would bypass its hook.
- **Generics.** Instances call the hook of the concrete type. Hooks of generic types (`operator mut fun <T> Owned<T>.drop()`) are instances of their own, planned from the files' summaries: a summary now also requests the hook instances that the types its bodies drop, clone or pass as type arguments reach.
- **Code generation.** Drop glue calls a type's drop hook first, then drops its parts; clone glue calls its clone hook instead of cloning field-wise. Glue calls hooks through the function table, so editing a hook's body recompiles no importer (deterministic hermetic builds) and hot reload swaps it like any function. Glue now runs user code, but only through hooks.
- **ABI and hot reload.** Whether a type has a drop hook and a clone hook is part of its definition: recorded in its exported record and in reachable definitions, covered by the ABI hash, the definitions' hash of instance keys, and the type's fingerprint, so a reload that adds or removes a hook gives every contract mentioning the type a new slot and values made before are dropped by glue that agrees with how they were made.
- **Top-level values** are still never dropped by the program. Under `--check-allocations`, the check's release of top-level values after `main` returns, which already runs their drop glue in reverse order of initialization, now runs their hooks; a hook that reads a top-level value the check has already dropped is a runtime error instead of a use after free.
- **Runtime errors in hooks** end the program like any other (there is no unwinding), and no further drop runs.
- **Not in scope (deferred):** rewriting any library type with hooks (`Composition`, state, nodes: `library-in-prism`); a way to skip a value's hook (`forget`), or to drop it early by name (a library `fun <T> drop(x: T)`); generic `equals` through the same planning (`operator-conventions` D6); hooks on intrinsic structs; a `close()`/`use { }` protocol.

## Capabilities

### New Capabilities

None: hooks are part of the ownership model and of how values are dropped and cloned.

### Modified Capabilities
- `ownership`: drop and clone hooks, their declaration rules (`E4035`), no `Copy` with a drop hook, clone hooks required beside drop hooks on `Clone` types, no calls by name (`E4036`), no taking apart (`E4017`), the structural `Clone` check waived by a clone hook, and their compile-time cost.
- `execution`: when hooks run (scope ends, assignments, temporaries, moves and drop flags, fields, payloads, list elements, box contents, closures, instances), clone hooks wherever values are cloned, a runtime resource released by a hook, runtime errors in hooks, a closure keeping only what it captures by value, and top-level values under the allocation check.
- `code-generation`: drop and clone glue calling hooks through the function table.
- `compilation-and-abi`: hook flags in the ABI and in definitions, and hook instances planned from summaries.
- `hot-reload`: type fingerprints covering hooks; hook bodies reloaded like functions.
- `compose`: hooks of values that a pass releases or whose cell is freed run after the pass; hooks run during a pass follow the pass's rules.

## Impact

- `crates/prism-ast` and `crates/prism-parser`: nothing new (the `operator` modifier is `operator-conventions`').
- `crates/prism-abi/src/signature.rs`: `drop_hook`/`clone_hook` flags on `ExportedSymbol::{Struct, Enum}` and `AbiTypeDefinition`, set in Phase 1 by a syntactic match of the file's own hook declarations.
- `crates/prism-typed-tree/src/types.rs`: hook flags in `TypeTable` beside markers; `crates/prism-typechecker` (`checker/operators.rs` hook shapes, `checker.rs` `check_marker_declarations`, `checker/patterns.rs` take-apart check, method-call resolution for `E4036`, `error.rs`).
- `crates/prism-mir/src/summary.rs`: hook instance requests in `FileSummary` and `GenericSummary`.
- `crates/prism-codegen/src/glue.rs` (`owns_storage` becomes "needs glue", drop and clone glue call hooks), `contract.rs` (fingerprints), `instances.rs` (planning hook instances), `symbols.rs`.
- `crates/prism-runtime/src/allocation.rs` and the top-level accessors: values dropped by the check are marked released.
- Fixtures under `crates/prism-cli/tests/fixtures/{ownership,execution,code-generation,compilation-and-abi,hot-reload,compose}/`, `run_cli_tests.rs`, `cache_tests.rs`, `watch_session_tests.rs`; `compile_timing.rs` shapes `drop-hooks` and `generic-hook-instances`.
- `openspec/specs/diagnostics/spec.md`: `E4036`, and the `E4017`, `E4035`, `E4016` and `E4010` rows.
