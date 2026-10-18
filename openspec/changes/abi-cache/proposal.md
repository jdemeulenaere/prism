# Proposal: abi-cache

## Why

The deterministic hermetic builds invariant promises that editing a non-generic body causes zero downstream recompilation, and Phase 1 already computes the ABI hashes this needs. But `prism check` has no cache: every run re-checks every file, and nothing tests the promise. The ABI hash algorithm (FNV-1a or the original design's BLAKE3) was also left open until a cache exists. This is item 12 of "Then: the road to Milestone 2" in `ROADMAP.md`. It lands before code generation, so the backend's object code gets keys, a store and avoidance tests that already work.

## What Changes

- **BLAKE3 becomes the hash algorithm.** The ABI hash, the Monomorphization Hash of an exported generic body and every cache key are 256-bit BLAKE3 hashes, replacing the 128-bit FNV-1a hash. `prism check` and `prism abi` print 64 hex digits. **BREAKING** for anything that stored a 32-digit hash; nothing in the repository does.
- **An interface hash per file.** Each ABI also gets an interface hash: the ABI hash without the generic body hashes. Phase 2 of an importer reads signatures only, never a generic body, so its cache key uses the interface hashes of its direct imports. A generic body edit then re-checks no importer, while the ABI hash still covers the body for whatever instantiates it.
- **A build cache.** It is content-addressed, per user and outside the workspace: `$PRISM_CACHE`, or the platform cache directory (`~/Library/Caches/prism`, `$XDG_CACHE_HOME/prism` or `~/.cache/prism`, `%LOCALAPPDATA%\prism`). It never writes into the workspace, so it needs no `.gitignore` entry and adds nothing that could act as a marker file, and root discovery never reads it. Keys contain no absolute path, so two checkouts of one workspace share their entries.
- **Compilation avoidance in `prism check`.** For each file, `prism check` computes a key from the cache format version, the compiler's identity (its version and a hash of its executable), the file's module path, its content and its direct imports' interface hashes. A file whose key has an entry is not type-checked, lowered or borrow-checked again. A file that passes Phase 2 records an entry; a file with errors records none. Phase 1 (parsing and ABI extraction) still runs for every file, since the resolver parses every file to build the import graph.
- **Instantiation records.** When a file's MIR calls an imported generic with concrete type arguments, the check records an instance entry keyed by the generic's identity, its Monomorphization Hash and the type arguments. After a generic body changes, only the files that call it report the instances they must instantiate again. No importer is re-checked. Instantiating them is the later `monomorphization` change.
- **Entries are versioned and self-checking.** A cache problem never fails a check: a missing, truncated, corrupt, foreign-version or unreadable entry is a miss, and an unwritable cache is skipped. Entries are written to a temporary file and renamed into place, so concurrent `prism` processes never read a partial entry.
- **CLI.** `prism check --no-cache` neither reads nor writes the cache, and `PRISM_CACHE=off` disables it everywhere. `prism check --explain-cache` lists, after the usual report, the cache directory and, per file in check order, `checked` or `reused`, then the instances it would instantiate or reuses. The avoidance scenarios become CLI tests that edit files between runs in a temporary workspace with a temporary cache. `prism abi` also prints the interface hash.
- **Not in scope (deferred):** cached `.abi` entries and import lists that would let a warm run skip parsing unchanged files; caching typed trees or MIR (no consumer yet); object code and the target triple in keys (`codegen-core`); real instantiation (`monomorphization`); per-symbol dependency tracking, which would re-check an importer only when a symbol it uses changes; cache size limits and a `prism cache clean` command.

## Capabilities

### New Capabilities
- `build-cache`: the content-addressed build cache: where it lives, what its keys cover, how entries are written and validated, how failures degrade to misses, and the `--no-cache`, `PRISM_CACHE` and `--explain-cache` controls.

### Modified Capabilities
- `compilation-and-abi`: the ABI hash becomes a 256-bit BLAKE3 hash; adds the interface hash, compilation avoidance in `prism check` (implementing "Compilation Avoidance via ABI Hashes" of `docs/architecture.md` section 1), and instantiation records for generic body changes (the part of "Re-monomorphization on Generic Body Change" that applies before code generation).

## Impact

- `Cargo.toml`: the `blake3` dependency, built with `opt-level = 3` in the dev profile so hashing the compiler executable stays fast in debug builds.
- `crates/prism-abi`: `DeterministicHasher` wraps BLAKE3 and returns a 32-byte `ContentHash`; `FileAbi` gains `interface_hash`, `generic_body_hash` becomes a `ContentHash`; tests comparing hashes keep working, and a test pins the interface-hash scenarios.
- `crates/prism-resolver`: `SourceNode` keeps the source text it parsed, so the content key hashes exactly what was checked.
- `crates/prism-cli`: a `cache` module (location, compiler identity, entry format, atomic writes); `workspace.rs` gains `check_path_with` and `CheckOptions` and consults the cache per file, collecting instance requests from MIR; `CheckedFile` records the file's cache outcome. `main.rs` parses `--no-cache` and `--explain-cache`, prints the explanation and the interface hash. `check_path` stays uncached, so the fixture harness and library tests never touch a user's cache. A new `tests/cache_tests.rs` runs the scenarios against the binary.
- `crates/prism-typechecker`: a test constructing a `FileAbi` uses the new hash type.
- `docs/decisions-for-review.md`: the autonomous decisions; on archive, `ROADMAP.md` and `docs/architecture.md` section 1.
