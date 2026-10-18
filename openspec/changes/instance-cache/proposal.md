# Proposal: instance-cache

## Why

`monomorphization` makes generic code run, but nothing it produces survives the process. `prism run` type-checks, lowers, borrow-checks and compiles every file and every instance on every start. `prism check`'s instance records still use `abi-cache`'s keys, which miss what an instance really depends on. They are keyed by a Phase 1 hash of the body's AST, so they miss a private struct the body holds gaining a field, a private helper's parameter becoming `&T`, and `keep<Thing>` after `Thing` gains a field. They also see only direct calls from one file, not the instances a program needs transitively. Polymorphic recursion spread over an import cycle (`E6004`) is found only by `prism run`. This is item 21 of "Then: the road to Milestone 2" in `ROADMAP.md`, split out of `monomorphization` by its D1 and shaped by its D13. `hot-reload` (item 25) needs it: after an edit it must know cheaply which code units and instances changed, and compile only those. Compile speed is a primary criterion, and a warm `prism run` of a large unchanged program should spend no time in Cranelift.

## What Changes

- **The Monomorphization Hash moves onto the generic record.** It becomes the BLAKE3 hash of the canonical encoding of the generic's record (its elaborated MIR with source positions, and the definitions of the types it mentions), computed after Phase 2 for every generic, private ones included. The AST body hash leaves the ABI (D2).
- **BREAKING: one ABI hash.** Without body hashes, the ABI holds signatures and reachable type definitions only, so the interface hash and the ABI hash are the same hash. `prism abi` prints `ABI Hash:` alone, and check keys use the direct imports' ABI hashes, which is what the deterministic hermetic builds invariant says. A generic body edit no longer changes its file's ABI hash (D3).
- **Records and summaries in the build cache.** A file's check entry now also holds its **summary**: the Monomorphization Hash and the generic calls of each of its generics, the instances its non-generic bodies request, and whether it declares a valid `main`. Each record's MIR is stored in its own entry, addressed by its Monomorphization Hash and read only when an instance must be compiled (D4, D5).
- **Instances are planned per program, from summaries alone.** `prism check` and `prism run` list the instances the program needs, transitively and across files, without reading any MIR. `prism check` now reports `E6004` for polymorphic recursion through an import cycle (D7).
- **Instance keys cover what an instance's code depends on.** An instance's key is its symbol, its generic's Monomorphization Hash and a hash of the definitions, with markers, of every struct and enum its type arguments reach, plus the compiler and the target. The private helpers a body calls are covered by their signatures inside the record. Their bodies are not, since instances call them by symbol (D6).
- **Generated code is cached.** Each file's code unit and each instance's unit is stored as relocatable machine code with symbolic relocations and data objects. A file's unit is keyed by its check key and the target, and an instance's by its key. `prism run` reuses the build cache by default. A file whose code is cached is neither checked nor compiled again, an instance whose code is cached is not instantiated again, and fresh and cached code are linked through one path. `prism run --no-cache` and `--explain-cache` join `prism check`'s (D1, D8, D9).
- **The target identity in keys.** Code keys name the target triple and every Cranelift setting of the backend, the optimization level and the detected CPU features included, closing `abi-cache`'s D5 follow-up (D8).
- **BREAKING: `--explain-cache` lists the program's instances once.** Each file line says `checked`, `reused` or, under `prism run`, `compiled` (verdict reused, code generated). The instances follow once per program, in symbol order, as `instantiate <instance>` or `reused instance <instance>`. The old per-requester `for <path>` suffix is dropped. Under `prism check`, instance lines say whether the cache holds the instance's code (D10).
- **Cache format version 2.** Entries keep a text header and checksum but carry a binary body (D11).
- **For `hot-reload`.** A program's **build plan**, its units and instances with their code keys, is a value that two runs or two edits can compare (D12).
- Not in scope: cached `.abi` entries and import lists, so a warm run still parses every file and extracts its ABI; positions relative to the declaration in records; per-symbol fingerprints; cache eviction; sharing code between equal-layout instances; reusing cached code in `prism dump-clif` and `prism dump-mir`; Tier 2 object files.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `build-cache`: what the cache stores (check entries with summaries, record entries, code entries), the keys of each, `prism run` reading and writing it, `--no-cache` and `--explain-cache` on `prism run`, the program-wide instance lines, and format version 2.
- `compilation-and-abi`: the ABI hash covers no body and replaces the interface hash, the Monomorphization Hash is a hash of the generic record, records are kept with the file's verdict, instance keys cover the type arguments' definitions, and compilation avoidance and reachable-type re-checks are keyed by ABI hashes.
- `code-generation`: compiled units are relocatable artifacts defined through one path whether fresh or cached, and a warm run compiles nothing.
- `generics`: polymorphic recursion across files is reported by `prism check` as well as `prism run`.
- `execution`: `prism run` takes `--no-cache` and `--explain-cache`, and a run from cached code behaves as a cold run.

## Impact

- `crates/prism-abi`: `generic_body_hash` and `interface_hash` removed from `ExportedSymbol` and `FileAbi`, one hash per file.
- `crates/prism-mir`: a canonical binary codec for generic records and the typed-tree types they hold (`codec.rs`), and file and generic summaries (`summary.rs`) next to `monomorphization`'s `record.rs`.
- `crates/prism-typechecker`: the conversion of an ABI type definition into a type-table definition, made public for a program-wide type table.
- `crates/prism-codegen`: `instances.rs` split into planning from summaries (`plan_instances`, `E6004`) and building instance units from records, relocatable unit artifacts and their codec (`artifact.rs`), `Linker` defining artifacts (`link.rs`), and the target identity (`isa.rs`).
- `crates/prism-cli`: `cache.rs` (format 2, binary bodies, record, unit and instance-code entries, new keys), `workspace.rs` (summaries in check entries, program plans, `E6004` in `prism check`, program-wide instance outcomes), `run.rs` (`compile_program_with` reusing verdicts, records and code), a new `plan.rs` (keys and the build plan), `main.rs` (flags and explanation), tests in `cache_tests.rs`, the fixture runner and the warm-run timing shapes.
- Specs: the deltas above. The diagnostics catalog's `E6004` row says that `prism check` reports it too. It is edited directly.
- `docs/architecture.md` §1: on archive, "Re-monomorphization on Generic Body Change" is implemented and removed.
- **Prerequisites:** `monomorphization` must be archived first. Its records, instance units, symbols and `E6004` are the baseline, and several deltas here modify requirements it adds (see `design.md`, "Baseline").
