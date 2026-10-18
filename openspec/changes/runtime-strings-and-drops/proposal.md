# Proposal: runtime-strings-and-drops

## Why

After `codegen-aggregates`, the Tier 1 backend runs structs, enums and nullables, but every `String`, and every struct, enum or nullable holding one, is still rejected with `E6001`, and no drop does anything, because no value owns memory yet. Strings are what every real program prints and what Phase 4's UI shows (`Text("Count: $count")`), and owned memory freed deterministically where its owner's scope ends is the zero-cost memory safety invariant made real. This change, step 16 of "Then: the road to Milestone 2" in `ROADMAP.md`, gives the backend owned strings, an allocator, drop glue and clone glue, and a way to check, in every executable fixture, that no allocation outlives its owner or is freed twice. It splits off `Heap<T>` construction (`heap-boxes`, next) and string templates (`string-templates`), as `design.md` explains (D1).

## What Changes

- **Owned strings in the backend.** A `String` is UTF-8 text held in a 32-byte value: a pointer to its bytes, its length in bytes, its length in characters and its capacity, the capacity being 0 when the value owns no buffer. A string literal is static data of its code unit and costs no allocation, while still evaluating to a fresh owned value as the `ownership` spec says: dropping it frees nothing, and cloning it copies the value. Strings are passed and returned by address like aggregates, and can be fields, payloads, nullable payloads, top-level values and the referents of borrows.
- **String operations.** `a + b` builds a new string, `s += t` is `s = s + t` (an in-place append stays a follow-up, D15), `==` and `!=` compare the text, `when` matches string literals, and `print` and `println` write the text.
- **String members.** `s.length` is the number of characters (Unicode scalar values), and `s[i]` the character at character index `i`, raising the runtime error "string index 5 out of range for length 3" outside `0..<s.length`. A string's character is a value, not a place: `&s[i]` borrows a temporary, and `s[i] = c` stays `E4003`, now with help saying strings are immutable. **BREAKING** for `&s[i]`, which borrowed `s` itself.
- **Converting scalars to text.** `Int`, `Double`, `Bool` and `Char` gain a built-in `toString()`, which writes what `println` writes; an extension named `toString` on those types, or `length` on `String`, can never be called and is `E4010`. Other types may declare their own `toString()` extension.
- **An allocator.** Generated code and the runtime allocate through `prism-runtime` functions over Rust's global allocator, with sized frees; running out of memory is the runtime error "out of memory".
- **Drop glue.** Dropping a value runs code generated per type: a `String` frees its buffer when it owns one; a struct drops its owning fields in declaration order, an enum its variant's owning payloads, a nullable its payload when present; a type that owns nothing has no glue. Every drop the elaborated MIR keeps, under a drop flag or not, runs it, including the old value of an assignment and the old referent of `*r = v`. Top-level values are never dropped. A runtime error still runs no drop: there is no unwinding.
- **Clone glue.** `x.clone()` of an owning `Clone` value builds a value that shares no buffer with `x`: field by field for structs, by variant for enums, by presence for nullables, a new buffer for a string that owns one.
- **Allocation checking.** `prism run --check-allocations` tracks every allocation and, when `main` returns, reports allocations still live and any free of memory not allocated, exiting with status 102. Every executable fixture runs with it, so each one asserts that its program leaks nothing and frees nothing twice.
- **Fewer `E6001`s.** Strings and the aggregates holding them stop being rejected. `Heap<T>` values stay rejected, now pointing at `heap-boxes`; function values, generic calls and composable functions are unchanged.
- **`ROADMAP.md`.** `heap-boxes` (construction and unboxing of `Heap<T>`, its drop and clone glue, and its covariance) and `string-templates` become items 17 and 18, before `monomorphization`; the later items are renumbered.
- Not in scope: `Heap<T>` construction and moving out of it (`heap-boxes`); string templates (`string-templates`); in-place append for `+=`, a small-string representation and a null-pointer niche for `String?` (follow-ups); printing structs, enums and nullables; further string members (`substring`, `isEmpty`, conversions from text).

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `code-generation`: the representation of `String`, strings in the calling convention, drop glue and clone glue, the allocator, glue functions in `prism dump-clif`, the updated `E6001` list and layout dump.
- `execution`: the runtime meaning of string literals, concatenation, equality, `length`, indexing, `toString()` and printing strings; when drops run and what they free; cloning owning values; the runtime errors "string index out of range" and "out of memory"; `--check-allocations`.
- `type-system`: the built-in `length` member and character indexing of `String`, and the built-in `toString()` of the scalar types.
- `typed-tree`: the new built-in operations (`length`, string indexing, `toString()`).
- `mir`: calls of the new built-ins, with a receiver, and string indexing as a call rather than a place.
- `references`: a string's character is not a place, so it is borrowed as a temporary.
- `extension-functions`: built-in members take precedence over extensions, and extensions named like them are `E4010`.

## Impact

- `crates/prism-runtime`: new `string.rs` (string operations and `toString()`), `allocation.rs` (allocator and allocation tracking), new panic kinds, `start` and `finish` taking the check option.
- `crates/prism-codegen`: `representation.rs` and the layout table give `String` a memory representation; `supported.rs` stops rejecting strings and names `heap-boxes` for `Heap<T>`; `lower.rs` lowers literals, string operators, the new built-ins, drops and clones; a new `glue.rs` builds drop and clone glue per code unit; `unit.rs` registers top-level values with the allocation check; `dump.rs` prints glue; `program.rs` passes the check option.
- `crates/prism-typed-tree`, `crates/prism-typechecker`, `crates/prism-mir`: the `StringLength`, `StringCharacter` and `ToString` built-ins, their typing and precedence, `E4010` for uncallable extensions, string indexing as a built-in call.
- `crates/prism-cli`: `prism run --check-allocations`; the fixture runner runs every executable case with it; fixtures under `code-generation/`, `execution/`, `type-system/`, `typed-tree/`, `mir/`, `references/` and `extension-functions/`; `examples/run/` prints a string.
- Specs: deltas above; the `E4010` and `E6001` rows of the catalog in `openspec/specs/diagnostics/spec.md`.
- `docs/architecture.md`: on archive, §4's "Clone Evaluation Semantics" (but `Heap<T>`), "Reference Evaluation Semantics", "Scope Exit Cleanup" and "Method Call Evaluation Semantics" move into the specs; the `Heap<T>` requirements stay for `heap-boxes`.
- Builds on `codegen-aggregates` (layouts, the calling convention, drops that do nothing), whose delta specs this change's deltas modify; it must be archived first.
