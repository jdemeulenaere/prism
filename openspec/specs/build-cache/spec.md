# Build Cache Specification

## Purpose
The build cache is the content-addressed store that lets `prism check`, and later code generation, skip work whose inputs have not changed: where it lives, what its keys cover, how its entries are written and validated, and how a developer disables or inspects it.

## Requirements

### Requirement: Per-User Cache Location Outside the Workspace
The cache MUST live in the directory named by the `PRISM_CACHE` environment variable when it is set to anything but `off`, and otherwise in a `prism` directory under the platform's per-user cache directory. The compiler MUST NOT create or write any file inside the workspace for caching, and MUST NOT read the cache to discover a workspace root or resolve an import.

#### Scenario: Default cache location
- **WHEN** `PRISM_CACHE` is unset and `prism check` runs on macOS, on Linux, or on Windows
- **THEN** the cache directory is `~/Library/Caches/prism`, `$XDG_CACHE_HOME/prism` (or `~/.cache/prism` when `XDG_CACHE_HOME` is unset), or `%LOCALAPPDATA%\prism` respectively

#### Scenario: Cache directory override
- **WHEN** `PRISM_CACHE=/tmp/c` is set and `prism check` succeeds
- **THEN** the cache's entries are written under `/tmp/c`

#### Scenario: Checking writes nothing in the workspace
- **WHEN** `prism check` runs on a workspace with the cache enabled
- **THEN** the workspace holds the same files and directories afterwards as before

#### Scenario: Cache shared across checkouts
- **WHEN** a workspace is checked, then copied to another directory and checked there with the same cache
- **THEN** the second check reuses every file, because no cache key contains an absolute path

### Requirement: Cache Keys Cover Exactly the Compilation Inputs
The key of a file's Phase 2 entry MUST be a BLAKE3 hash of the cache format version, the compiler's identity, the file's module path, its exact source text, and the interface hashes of its direct imports in import order. The compiler's identity MUST cover the compiler version and the content of the compiler executable. No other input MUST affect a key.

#### Scenario: Unchanged workspace reuses every file
- **WHEN** `prism check` runs twice on an unchanged workspace with the same compiler
- **THEN** the second run reuses every file

#### Scenario: Comment-only edit re-checks the file alone
- **WHEN** a comment is added to a file imported by others
- **THEN** that file is checked again, because its source text changed, and its importers are reused, because its interface hash did not change

#### Scenario: A different compiler re-checks everything
- **WHEN** a workspace checked by one compiler executable is checked by a different one, of the same version or not
- **THEN** no entry of the first is reused

### Requirement: Versioned Self-Checking Entries
Every entry MUST record the cache format version, its kind and its key, and end with a BLAKE3 checksum of its contents. The cache MUST keep entries of format version `N` under a `vN` directory. An entry that is missing, truncated, corrupt, of another version, kind or key, or unreadable MUST be treated as a miss and overwritten by the next successful check.

#### Scenario: Corrupt entry is a miss
- **WHEN** every entry of a warm cache is truncated or overwritten with garbage, and `prism check` runs again
- **THEN** the check succeeds as on a cold cache, every file is checked again, and the entries are rewritten

### Requirement: The Cache Never Fails a Check
A cache problem MUST NOT change the outcome of a check: when the cache directory cannot be created, read or written, `prism check` MUST behave as with the cache disabled, and report no error for it.

#### Scenario: Unwritable cache directory
- **WHEN** `PRISM_CACHE` names a path that is a regular file, and `prism check` runs on a valid workspace
- **THEN** the check succeeds and prints the same report as without a cache

### Requirement: Atomic Entry Writes
Each entry MUST be written to a temporary file in its final directory and then renamed into place, so that a reader sees either no entry or a complete one. Concurrent `prism` processes sharing a cache MUST NOT need a lock.

#### Scenario: Concurrent checks share a cache
- **WHEN** several `prism check` processes run at the same time on one workspace with one empty cache
- **THEN** every process succeeds with the same report, and a later check reuses every file

### Requirement: Disabling the Cache
`prism check --no-cache` MUST neither read nor write the cache, and `PRISM_CACHE=off` MUST disable the cache for every command.

#### Scenario: Checking without the cache
- **WHEN** a workspace is checked with `--no-cache`, or with `PRISM_CACHE=off`, after a check that filled the cache
- **THEN** every file is checked again, and no entry is written

### Requirement: Explaining Cache Use
`prism check --explain-cache` MUST print, after a successful check's report, a line `Cache: <directory>` (or `Cache: off`), then for each file in check order `  checked <path>` or `  reused <path>`, with the path relative to the workspace root and `/` separators, each followed by its instances as `  instantiate <instance> for <path>` or `  reused instance <instance> for <path>`.

#### Scenario: Explaining a warm check
- **WHEN** `app/main.prism` imports `/lib/box`, which exports `public fun <T> keep(x: T): T`, `app/main` calls `keep(1)`, and `prism check --explain-cache app/main.prism` runs twice with no edit
- **THEN** the first run prints `  checked lib/box.prism`, `  checked app/main.prism` and `  instantiate lib/box::keep<Int> for app/main.prism`, and the second prints `  reused lib/box.prism`, `  reused app/main.prism` and `  reused instance lib/box::keep<Int> for app/main.prism`

### Requirement: Instances Shown by Their Identity
`--explain-cache` MUST show an instance as the generic's module, `::`, for an extension the name of its receiver type without type arguments and `.`, the generic's name, and its type arguments in angle brackets. What it shows for a reused file MUST be what checking that file again would show.

#### Scenario: Explaining an instance of a generic extension
- **WHEN** `lib/box.prism` exports `public struct Box<T>(val value: T)` and `public fun <T> Box<T>.size(): Int`, `app/main.prism` calls `Box(1).size()`, and the extension's type parameter is renamed to `U` between two runs of `prism check --explain-cache app/main.prism`
- **THEN** the first run prints `  instantiate lib/box::Box.size<Int> for app/main.prism`, and the second reuses `app/main.prism`, whose import's interface hash ignores type parameter names, and prints `  reused instance lib/box::Box.size<Int> for app/main.prism`

## Design Rationale

Decisions from `abi-cache` (planned autonomously on 2026-10-17, reviewed on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **A per-user, content-addressed store outside the workspace** (D3). The cache lives in `$PRISM_CACHE` or the platform's per-user cache directory (Go's `os.UserCacheDir` rules), with a `CACHEDIR.TAG` so backup tools skip it. Writing nothing into the workspace means no `.gitignore` entry, read-only checkouts keep their cache, and nothing at the root can act as a marker file (the zero-marker invariant). No key holds an absolute path, so every checkout, worktree and copy of a workspace shares its entries. Rejected: a `<root>/.prism/` directory like Gradle's `.gradle/` (writes into the source tree, looks like a marker, and the root depends on the entry file); a per-user directory keyed by workspace path (identical checkouts would miss each other's entries).
- **What is cached before code generation** (D4). Check entries ("this file passes Phase 2 against these import interfaces", with its instance requests) and instance records. Failures are never cached: their diagnostics hold absolute paths and source excerpts, and a failing file is the one being edited. `.abi` entries are not cached yet, because the resolver parses every file to build the import graph and extracting an ABI from an AST in memory costs little next to parsing; they pay off together with cached import lists, once a warm run can skip parsing (a ROADMAP follow-up). Typed trees and MIR have no consumer across runs yet.
- **The compiler's identity is its version and a hash of its executable** (D5). During development the version never changes while the checker does, so a version-only key would reuse verdicts of an older compiler and hide regressions. Hashing the executable is how Go's build cache identifies development toolchains; `blake3` is built with `opt-level = 3` in the dev profile so this stays fast in debug builds. If the executable cannot be read, caching is off. Rejected: the version alone; a build-time hash of the sources (misses dependency and toolchain changes); the git commit (misses uncommitted edits).
- **Small, versioned, self-checking text entries** (D6). A header names the format version, kind and key, body lines hold instance requests, and a final line is a BLAKE3 checksum of everything before it. Text rather than a binary encoding: entries are a few hundred bytes, need no dependency and can be read with `cat`. A field containing a tab or newline cannot be written, so such a file is simply not cached. A format change that old readers would misread takes a new `vN` directory, so two compilers never fight over one entry.
- **Every cache problem is a miss** (D7). Reading never fails a check, and neither does writing: an unwritable cache is skipped silently, since it is not the user's error. An entry is written only after its file passes, so an interrupted check leaves only valid entries.
- **Atomic writes without locks** (D8). An entry is written to `<name>.tmp-<pid>-<counter>` in its final directory and renamed into place, which is atomic on POSIX and replaces the target on Windows, and the checksum catches a torn file after a crash. Entries are functions of their keys, so racing writers write identical bytes and need no lock. Rejected: lock files (stale after crashes, no benefit when writers agree); `fsync` per entry (a lost entry is only a miss).
- **The store is a module of `prism-cli`, and caching is opt-in** (D10). `check_path` stays uncached, so the fixture harness, library tests and embedders never touch a user's cache; `check_path_with` takes a cache explicitly and the `prism` binary opens one from the environment unless `--no-cache`. The store moves to its own crate when a second driver (the LSP, `prism run`) needs it. Rejected: caching in `check_path` by default, which would make every test depend on the state of the user's cache directory.
- **`--explain-cache` says what happened, not why** (D11). It appends to the unchanged default report, with root-relative `/`-separated paths so tests compare them on every OS. A content-addressed store keeps no history, so it cannot tell which input of a missed key changed; a per-module index of the last key's inputs would, and is a debugging aid to add if it is ever missed. The avoidance scenarios are integration tests in `crates/prism-cli/tests/cache_tests.rs` that edit files between runs of the binary, which the fixture harness cannot express.
- **Instances are shown by their identity** (D12, from the review). An extension's instance names its receiver by the type's name alone (`lib/box::Box.size<Int>`): the receiver's type arguments are the extension's own type parameters, whose names no interface hash covers, so showing them let a reused importer show a stale name.
