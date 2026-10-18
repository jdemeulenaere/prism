# Design: abi-cache

## Context

See `proposal.md` for motivation. This change is item 12 of "Then: the road to Milestone 2" in `ROADMAP.md`, planned autonomously on 2026-10-17, and also covers the Phase 2 items "Decide the authoritative ABI hash algorithm", "`.abi` cache and compilation avoidance" and "Incremental compilation verification". Every decision the user might make differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`.

Current state:

- **Hashing** (`crates/prism-abi/src/hasher.rs`): `DeterministicHasher` is a 128-bit FNV-1a over a byte stream of tags, lengths and `0xFF`-terminated strings. `FileAbi::calculate_hash` walks the sorted symbols; an exported generic function's `generic_body_hash: Option<u128>` (its Monomorphization Hash, a span-free hash of its AST body) is folded into the walk. No test pins a hash value, only equalities and inequalities. `prism check` prints `ABI: 0x<32 hex>` per file and `prism abi` prints `ABI Hash: 0x<32 hex>`.
- **Driver** (`crates/prism-cli/src/workspace.rs`): `check_path` builds a `DependencyGraph` (the resolver reads and parses every reachable file and keeps its AST, not its text), extracts every ABI (Phase 1), then runs Phase 2 per file in topological SCC order (type checking against the direct imports' `FileAbi`s, MIR lowering, borrow checking) and stops at the first file with errors. Nothing is cached, and the fixture harness and many tests call `check_path` directly.
- **Phase 2 inputs**: by the deterministic hermetic builds invariant and the `compilation-and-abi` spec, a file's typed tree, MIR and borrow-check verdict depend only on its AST, its module path (from which its package follows) and the ABIs of its direct imports. Phase 2 never reads a generic body of an import: the Monomorphization Hash in an import's ABI is unused by it.
- **MIR calls** (`TerminatorKind::Call`) name the callee by `DeclarationPath` (module, receiver type, name) and carry its type arguments, which inside a generic body may mention the body's own type parameters.
- **Workspace roots** come from each entry's path and `package` declaration, so one directory can be a package for one entry and a root for another (zero-marker invariant).
- The workspace has no external dependency yet.

## Goals / Non-Goals

**Goals:**
- Settle the hash algorithm before any hash is persisted.
- Make the avoidance promises of the hermetic builds invariant observable and tested: a body edit re-checks no importer, a signature edit re-checks direct importers only, an unchanged workspace re-checks nothing.
- A cache store that code generation can extend with object-code entries without changing its location, format or guarantees.

**Non-Goals:**
- Skipping Phase 1 or parsing on a warm run (D4).
- Serializing typed trees, MIR or diagnostics.
- Code generation and real instantiation; the target triple joins keys with the first target-dependent entry.
- Eviction and size limits.

## Decisions

### D1. BLAKE3, 256 bits, everywhere (autonomous)

The ABI hash, the interface hash (D2), the Monomorphization Hash and every cache key and checksum are BLAKE3 hashes, kept at the full 256 bits in a `ContentHash([u8; 32])` and printed as 64 hex digits. `DeterministicHasher` keeps its API (tags, lengths, `0xFF`-terminated strings) over a `blake3::Hasher`, so the hashed byte streams, and therefore every equality the specs state, are unchanged.

The ABI hash is now a cache key: a collision would make the cache silently reuse a verdict, and later object code, computed against another interface. FNV-1a is not collision resistant: collisions can be built on purpose, and a 128-bit state gives no margin against them. BLAKE3 is cryptographic, is faster than FNV-1a on anything but tiny inputs (FNV-1a does a 128-bit multiply per byte), and is what the original design named. Rejected: FNV-1a (no collision resistance for a content-addressed store); SHA-256 (slower, nothing gained); BLAKE3 truncated to 128 bits (64-bit collision resistance, to save 16 bytes per hash).

The `blake3` crate is the first external dependency. It is built with `opt-level = 3` in the dev profile, because D5 hashes the compiler executable on every cached run and an unoptimized BLAKE3 would take a noticeable fraction of a second in debug builds.

### D2. An interface hash keys Phase 2 (autonomous)

`FileAbi` gains `interface_hash`: the same walk as today with each function's Monomorphization Hash left out. The ABI hash becomes `H("abi", interface_hash, then, per symbol in order, a presence byte and the Monomorphization Hash)`. It still changes exactly when the old one did, and is now a function of the interface hash and the body hashes.

A file's Phase 2 entry is keyed by its direct imports' interface hashes, not their ABI hashes. Phase 2 reads signatures only, so a verdict computed against one generic body holds for every other body with the same interface. The deterministic hermetic builds invariant still holds: the verdict depends on less than the import ABI hashes, never on more. Whatever does depend on a generic body, the future instances, is keyed by the Monomorphization Hash itself (D9).

Rejected: keying by ABI hashes, which re-checks every importer of a file whenever a generic body in it changes, though none of them reads that body. Rejected for now: per-symbol fingerprints (each file records the imported symbols it uses and is keyed by their hashes, as Kotlin's and Zig's incremental compilers do). That would re-check an importer only when a symbol it uses changes, but it needs a use set per file and a two-step lookup. It refines this design rather than replacing it, and is a ROADMAP follow-up.

### D3. A per-user, content-addressed cache outside the workspace (autonomous)

The cache directory is `$PRISM_CACHE` when set (and not `off`), else `<user cache dir>/prism`, where the user cache directory is Go's `os.UserCacheDir`: `~/Library/Caches` on macOS, `$XDG_CACHE_HOME` or `~/.cache` on other Unix systems, `%LOCALAPPDATA%` on Windows. With no such directory (no `HOME`), caching is off. Entries live under `<dir>/v1/<first two hex digits>/<key hex>`, and the directory gets a `CACHEDIR.TAG` so backup tools skip it, as Cargo's does. Nothing is written in the workspace.

Keys hold no absolute path (D5), so the store is shared by every checkout, worktree and copy of a workspace and by every workspace using the same library files: this repository's own worktrees reuse each other's verdicts.

Rejected: a hidden directory at the workspace root (`<root>/.prism/`, like Gradle's `.gradle/`). It writes into the source tree, so every workspace needs a `.gitignore` entry and read-only checkouts lose the cache. A directory at the root also looks like a marker, which the zero-marker invariant exists to avoid. And since the root depends on the entry file, one tree could hold caches at several levels. Rejected: a per-user directory keyed by workspace path. A path in the key makes two identical checkouts miss each other's entries, and a moved workspace lose its cache, for an input that cannot change any result.

### D4. What is cached before code generation (autonomous)

- **Check entries**: "this file passes Phase 2 against these import interfaces", plus the file's instance requests (D9). They are written only after type checking, MIR lowering and borrow checking all succeed.
- **Instance records** (D9).

Not cached:
- **Failures.** A file with errors records nothing, so its diagnostics are always computed fresh. They contain absolute paths and source excerpts, which are not key inputs, and a failing file is the one being edited.
- **`.abi` entries.** The resolver parses every reachable file to find its imports, and extraction from an AST already in memory costs little next to parsing. A cached ABI would save nothing today. It pays off once a warm run can skip parsing an unchanged file, which also needs its import list cached by content hash and a resolver that builds the graph from cached entries. That is a follow-up, and adding an entry kind is backward compatible (D6).
- **Typed trees and MIR.** Nothing consumes them across runs yet (existing ROADMAP items).

### D5. The check key, and the compiler's identity (autonomous)

The check key is `H("prism-cache/check", format version, compiler identity, module path, H(source text), number of direct imports, each import's interface hash in import order)`. The source text is the exact text the resolver parsed, which `SourceNode` now keeps, so an edit made during a check cannot pair a new key with an old verdict. Import order is part of the source, so it adds nothing a user could not see. The package follows from the module path. Check entries need no target triple, since nothing in Phase 2 depends on the target. Object-code entries will add it.

The compiler identity is `H(compiler version, BLAKE3 of the running executable)`, computed once per process. During development the version string never changes while the type checker does, so a version-only key would reuse verdicts from an older compiler, and `prism check examples/` after a checker change would hide regressions. Hashing the executable is how Go's build cache identifies development toolchains. If the executable cannot be read, caching is off. Rejected: version only (stale verdicts); a build script hashing the compiler's sources (misses dependency and toolchain changes); the git commit (misses uncommitted edits).

### D6. Entry format: small, text, self-checking

```
prism-cache 1
kind check
key <64 hex>
instance\t<module>\t<receiver constructor or empty>\t<name>\t<type-arguments hash>\t<display>
checksum <64 hex>
```

The checksum line is `BLAKE3` of every byte before it. Reading checks the magic and version, kind and key against what was asked for, the checksum, and each body line's shape. Any mismatch is a miss (D7). A check entry has one `instance` line per instance request. An instance record has no body. A field containing a tab or newline cannot be written, so such a file is not cached; module paths and names never contain them in practice. New entry kinds and body lines need a new version only when old readers would misread them, and a new version uses a new `vN` directory, so two compilers never fight over one entry.

Text rather than a binary encoding: entries are a few hundred bytes, the format needs no dependency, and an entry can be read with `cat` when debugging the cache.

### D7. Failures degrade to misses

Reading never fails a check: a missing, unreadable, malformed or foreign entry is a miss. Writing never fails a check either: if the directory cannot be created or a write or rename fails, the entry is simply not recorded. Nothing is reported, since an unwritable cache is not the user's error. `--explain-cache` still shows the directory and that every file was checked. A check entry is written only after its file passes, so a check interrupted half way leaves only valid entries.

### D8. Atomic writes without locks

An entry is written to `<final name>.tmp-<pid>-<counter>` in its final directory, then renamed over the final name. Renaming within one directory is atomic on POSIX and replaces the target on Windows (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`), so a reader sees no entry or a complete one, and the checksum also catches a torn or corrupted file after a crash. Two processes writing one key write identical bytes, since an entry is a function of its key, so whichever rename lands last changes nothing, and no lock is needed. A crash can leave a stray `.tmp-` file, which nothing reads. Rejected: lock files (stale locks after crashes, and no benefit when racing writers agree); `fsync` per entry (an entry lost to a crash is only a miss).

### D9. Instance records stand in for re-monomorphization (autonomous)

After a file passes, its MIR is scanned for calls to a generic function or extension of another module that is one of its direct imports, with type arguments that mention no type parameter. Each such call becomes an instance request: the callee's module, receiver constructor and name, a structural hash of its type arguments with every nominal type qualified by its module, and a display text (`lib/box::keep<Int>`, or `lib/box::Box.get<Int>` for an extension: the receiver is shown by its type's name alone, since its type arguments name the extension's type parameters, which no key covers). Requests are deduplicated and sorted, and stored in the check entry, so a reused file still has them.

For each request, the instance key is `H("prism-cache/instance", format version, compiler identity, callee identity, the callee's signature hash and Monomorphization Hash from the import's ABI, type-arguments hash)`. The signature hash is the callee's own part of the interface hash stream (`ExportedSymbol::interface_hash`): an instance compiled for `fun <T> drop(x: T)` is not one for `fun <T> drop(x: &T)`, even when the body is the same. (The first implementation keyed by the body alone and reused the instance after such a signature change; review caught it.) If the record exists, the instance is reused; otherwise the record is written and the instance is reported as one to instantiate. After a generic body changes, the importers are reused (D2), and exactly the requests naming that generic miss. That is "re-monomorphization only where a generic body changed", at the granularity code generation will need, before any instance exists.

This follows the direction `docs/architecture.md` recommends for `monomorphization`: instances as separate artifacts keyed by the generic, the type arguments and the generic's body hash, so a file's own artifact stays a function of its content and its imports' interfaces. That change will fill these records with code, and may change their key (for example to an MIR hash, if the Monomorphization Hash stops covering what an instance depends on). It may also discard them, at no cost, since they are a cache.

Not recorded:
- Polymorphic requests, whose type arguments mention the caller's own type parameters (`fun <T> wrap(x: T) = keep(x)`). They become concrete only when `wrap` is instantiated, which is monomorphization's transitive work.
- Calls to the file's own generics, which are re-checked with the file anyway.
- Default value generators of imported generics, whose text is already part of the interface hash.

Rejected: recording nothing and leaving the scenario to `monomorphization`. The scenario would stay untested, and the avoidance design could not show that it leaves room for instances. Rejected: storing the Monomorphization Hash seen at check time in the check entry and comparing on the next run. That entry would then have to be rewritten under the same key, so its value would depend on history and not on its key alone.

### D10. Where the code lives, and who uses the cache (autonomous)

The store (location, compiler identity, entry format, atomic writes) is a `cache` module of `prism-cli`, the crate that already drives workspaces. It moves to its own crate when a second driver (the LSP, `prism run`) needs it. Adding a crate now would only add a manifest.

`check_path` stays uncached, so the fixture harness, library tests and future embedders never touch a user's cache. `check_path_with(path, &CheckOptions { cache })` takes the cache explicitly, and the `prism` binary opens it from the environment unless `--no-cache` is given. `CheckedFile` gains its outcome (`Checked` or `Reused`) and its instance outcomes, which `--explain-cache` prints.

Rejected: caching inside `check_path` by default, which would make every test depend on the state of `~/Library/Caches`.

### D11. Observing the cache: `--explain-cache`

`prism check --explain-cache` appends the `Cache:` line and one line per file and instance (the `build-cache` spec). Paths are relative to the workspace root with `/` separators, so tests compare them on every OS. The default report is unchanged, so scripts and tests reading it are not affected. The avoidance scenarios are integration tests in `crates/prism-cli/tests/cache_tests.rs`: each builds a workspace and a cache in temporary directories, runs the `prism` binary with `PRISM_CACHE` set, edits files, runs it again and compares the explanation lines. Each test is named after the scenario it protects. The fixture harness cannot express edits between runs, and extending it for these few multi-step scenarios would not pay off.

The explanation says what happened, not why: a content-addressed store keeps no history, so it cannot tell which input of a missed key changed. A "why" would need a per-module index of the last key's inputs, which is a debugging aid to add if it is ever missed.

## Risks / Trade-offs

- [A warm run still parses every file and extracts every ABI, so the system-invariants scenario "Identical inputs produce cache hit" holds for Phase 2 only] → Phase 2 is most of the work today. Skipping parsing needs the `.abi` and import-list entries of D4, listed as a follow-up.
- [The cache grows without bound] → Entries are a few hundred bytes and keyed by content, so growth follows the number of distinct file versions checked. Eviction by last use, as Go trims its cache, is a follow-up; meanwhile the directory can be deleted at any time.
- [Hashing the executable costs a few milliseconds per run (about 9 MB in a debug build)] → It is computed once per process, and with `opt-level = 3` BLAKE3 hashes gigabytes per second.
- [A Phase 2 bug that makes a verdict depend on something outside the key would make cached and uncached runs disagree] → The invariant forbids such inputs, the `--no-cache` scenario compares both runs, and the compiler identity changes with every rebuilt compiler, so a fixed bug never leaves stale verdicts behind.
- [The Monomorphization Hash covers only the generic's own AST body, not the private helpers it calls, so an instance record can be reused although its instance would change] → No instance exists yet. `monomorphization` decides what the hash covers (already an open question there); D9's key changes with it.
- [`blake3` builds C or assembly code through `cc` on some targets] → It is the reference implementation and builds on all tier-1 targets. Its `pure` feature is available if a target ever lacks a C compiler.

## Migration Plan

No persisted hash or cache exists, so nothing migrates. The printed ABI hashes change length and value.

## Open Questions

- The deterministic hermetic builds invariant lists "file content, direct import ABI hashes, compiler version, and target triple". The keys also cover the file's module path, which the file's own ABI already records, and use the interface hash, which is coarser than the ABI hash. Should the invariant's wording name the module path explicitly? The answer changes no code.
