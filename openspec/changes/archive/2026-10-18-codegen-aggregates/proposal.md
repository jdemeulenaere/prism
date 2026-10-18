# Proposal: codegen-aggregates

## Why

The Tier 1 backend of `codegen-core` runs scalar programs only: every struct, enum or nullable value is rejected with `E6001`, so no program that models data can run, and nothing of the compose runtime (`State<T>`, `NodeId`, slot-table records) could be built on it. This change, step 15 of "Then: the road to Milestone 2" in `ROADMAP.md`, gives the backend memory layouts and teaches it every MIR construct over aggregates. It also settles the question `codegen-core` deferred (its D24, and the `mir-lowering` follow-up in `ROADMAP.md`): a file must lay out values of types defined in modules it does not import, such as the private field type of an imported struct, while its code still depends only on its direct imports' ABIs.

## What Changes

- **Layouts.** Every struct, enum and nullable type the backend supports gets a deterministic memory layout: structs inline by value, fields ordered by decreasing alignment; enums as the smallest unsigned tag that numbers their variants, followed by each variant's payload laid out like a struct, all variants sharing the storage after the tag; `T?` as a one-byte tag followed by its payload, except `(&T)?`, `(&mut T)?` and `Heap<T>?`, which are a single pointer whose null value is `null`. `Heap<T>` is laid out as one pointer, so recursive enums through `Heap<T>` have a finite layout, although constructing a `Heap<T>` still waits for `runtime-strings-and-drops`.
- **Infinitely sized types are a type error.** A struct or enum that contains itself by value, through fields, payloads and nullables but not through `Heap<T>`, is rejected with a new code, `E4023` (`InfinitelySizedType`), at its declaration, with help suggesting `Heap<...>`.
- **The ABI carries the definitions of reachable types.** Phase 1 records, in each file's ABI, the definition (kind, type parameters, marker, fields or variants) of every struct and enum that a value reached through its exports can hold, transitively, including private types and types of modules the importer does not import. The interface hash covers them, so a change to a type reachable through an import re-checks and recompiles exactly the importers whose imports' interfaces change, as the build cache already keys. The type checker records these definitions for layout only: using their members still needs their module imported (`E4014`, unchanged).
- **Code generation for aggregates.** Constructors (`Aggregate`), field reads and writes, writes through references (`*r = v`, `r.x = v`), borrows of aggregates, of fields and of nullable payloads (`&p.x`, the `(&T)?` view of a `&(T?)`), `Discriminant`, `WrapNullable`, `null`, the representation-preserving `Subtype` steps, `when` over enum variants and nullables, `?.` and `?:` (already branches in MIR), structural `==` and `!=` over structs, enums and nullables, the built-in `clone()` of an aggregate that owns no storage, and top-level `val`s of aggregate types. The built-in `Range` becomes an ordinary struct, usable as a parameter, result, borrow or top-level value. Concrete instances of generic structs and enums (`Pair<Int, Bool>`) are laid out by substitution; generic bodies still wait for `monomorphization`.
- **Calling convention.** A struct, enum or non-pointer nullable is passed as the address of caller-owned memory, and returned through an address the caller passes first, so a function's machine signature never depends on a layout. `Copy` and move are both bitwise copies at run time.
- **Drops of values that own nothing do nothing.** A struct or enum without `String`, `Heap<T>` or function values in it needs no drop glue, so its elaborated drops compile to nothing; `Copy` and non-`Copy` aggregates of scalars both run. Aggregates holding a `String` or `Heap<T>` stay rejected with `E6001`, naming the construct their contents need ("strings", "'Heap' values"), until `runtime-strings-and-drops`.
- **`prism dump-layout <file.prism>`** prints the layout of every struct, enum and nullable type the file declares or its compiled bodies hold, as deterministic text, and drives `layout.snap` fixtures.
- **Unchanged:** `print` and `println` accept scalars and `String` only; a struct, enum or nullable argument stays `E4004`, until a `toString` or `Display` story exists.
- Not in scope: `String`, drop glue, `clone()` of owning values and `Heap<T>` construction (`runtime-strings-and-drops`); generic bodies and calls (`monomorphization`); function values and closures (`closures`); scalar replacement of aggregates, niche filling beyond null pointers, passing small aggregates in registers, and a C-compatible layout for FFI (follow-ups); keeping values across a layout change while hot reloading (`hot-reload`).

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `code-generation` (created by `codegen-core`): layouts of structs, enums and nullables, layouts of types from transitively imported modules, the representation of aggregates and references to them, the calling convention for aggregates, drops that do nothing, `prism dump-layout`, and the updated list of constructs rejected with `E6001`.
- `execution` (created by `codegen-core`): the runtime meaning of constructing, copying, moving, reading and writing aggregates, borrows of places and fields, equality, `when` over enums and nullables, `?.` and `?:`, cloning values that own nothing, and aggregate top-level values.
- `compilation-and-abi`: the ABI records the definitions of the types reachable from its exports, the ABI and interface hashes cover them, and private declarations are recorded only that way.
- `type-system`: infinitely sized structs and enums are rejected (`E4023`).

## Impact

- `crates/prism-abi`: per-file type definitions read in Phase 1a, the reachable-type closure and its hashing in `FileAbi::extract` (`signature.rs`), a new `AbiTypeDefinition` record.
- `crates/prism-cli`: `extract_abis` in `workspace.rs` passes every file's definitions to the closure; `prism abi` prints the reachable types; the new `dump-layout` command (`run.rs`, `main.rs`); `layout.snap` snapshots in `tests/fixture_tests.rs`; fixtures under `tests/fixtures/code-generation/`, `execution/`, `compilation-and-abi/` and `type-system/`; `examples/run/` gains a struct, an enum and a nullable.
- `crates/prism-typechecker`: `ingest_file_abi` records reachable definitions in the type table; the `E4023` check over the file's own declarations.
- `crates/prism-codegen`: a new `layout.rs`; `representation.rs`, `supported.rs`, `lower.rs`, `unit.rs` and `dump.rs` extended; a layout dump.
- Specs: deltas above; the catalog in `openspec/specs/diagnostics/spec.md` gains `E4023` (a catalog row, not a requirement change).
- `docs/architecture.md`: on archive, §4 "Inline Value Struct Layout" and "Expression Evaluation Semantics" move into the specs, and the scenarios this change makes observable for `Copy` values ("Copy value clone is a copy", "A safe call on null evaluates no argument") leave their requirements, which stay for `runtime-strings-and-drops`.
- `ROADMAP.md`: on archive, item 15 and the follow-ups on layouts of transitively imported types, representation-changing subtyping and recursive layouts without `Heap<T>` are updated as `design.md` says.
