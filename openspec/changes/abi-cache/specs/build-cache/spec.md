# Build Cache Specification Delta

## Purpose

The build cache is the content-addressed store that lets `prism check`, and later code generation, skip work whose inputs have not changed: where it lives, what its keys cover, how its entries are written and validated, and how a developer disables or inspects it.

## ADDED Requirements

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
