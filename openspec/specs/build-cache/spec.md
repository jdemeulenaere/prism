# Build Cache Specification

## Purpose
The build cache is the content-addressed store that lets `prism check` skip Phase 2 and `prism run` skip Phase 2, instantiation and code generation for work whose inputs have not changed: where it lives, what it stores (check entries with file summaries, generic records by their Monomorphization Hash, and relocatable code of file units and instances), what its keys cover, the target identity code keys name, how its entries are written and validated, and how a developer disables or inspects it.

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

#### Scenario: Running writes nothing in the workspace
- **WHEN** `prism run app/main.prism` runs twice on a workspace with the cache enabled, the second run loading every unit from the cache
- **THEN** the workspace holds the same files and directories afterwards as before

#### Scenario: Cache shared across checkouts
- **WHEN** a workspace is checked and run, then copied to another directory and checked and run there with the same cache
- **THEN** the second check reuses every file and the second run loads every unit and instance from the cache, because no cache key contains an absolute path

### Requirement: Cache Keys Cover Exactly the Compilation Inputs
The key of a file's check entry MUST be a BLAKE3 hash of the cache format version, the compiler's identity, the file's module path, its exact source text, and the ABI hashes of its direct imports in import order, then, for a workspace file, those of the prelude modules in prelude order. A library module MUST be keyed by the same inputs. The compiler's identity MUST cover the compiler version and the content of the compiler executable. No other input MUST affect a check key.

#### Scenario: Unchanged workspace reuses every file
- **WHEN** `prism check` runs twice on an unchanged workspace with the same compiler
- **THEN** the second run reuses every file

#### Scenario: Comment-only edit re-checks the file alone
- **WHEN** a comment is added to a file imported by others
- **THEN** that file is checked again, because its source text changed, and its importers are reused, because its ABI hash did not change

#### Scenario: A different compiler re-checks everything
- **WHEN** a workspace checked by one compiler executable is checked by a different one, of the same version or not
- **THEN** no entry of the first is reused

#### Scenario: Library entries are shared between workspaces
- **WHEN** two workspaces that both use the prelude are checked one after the other with the same compiler and cache
- **THEN** the second check reuses the entries of `prism/core.prism` and `prism/collections.prism` that the first wrote

### Requirement: The Executable's Hash Is Recorded by Its Metadata
The compiler MAY read the hash of its executable's content from an entry under the cache's `executables` directory, keyed by the executable's path, size, modification and status-change times and file identity. It MUST write such an entry only for an executable whose metadata did not change while it was hashed and that is at least two seconds old, and MUST NOT count these entries among the entries a check or run writes.

#### Scenario: A rebuilt compiler is identified by its content
- **WHEN** the hash of a compiler executable has been recorded, and the executable is then replaced by one with other content at the same path
- **THEN** the next `prism check` hashes the new executable and reuses no entry of the first

### Requirement: Versioned Self-Checking Entries
Every entry MUST record the cache format version, its kind, its key and the length of its body, and end with a BLAKE3 checksum of its contents. The cache MUST keep entries of format version `N` under a `vN` directory. An entry that is missing, truncated, corrupt, undecodable, of another version, kind or key, or unreadable MUST be treated as a miss and overwritten by the next successful check or run.

#### Scenario: Corrupt entry is a miss
- **WHEN** every entry of a warm cache is truncated or overwritten with garbage, and `prism check` runs again
- **THEN** the check succeeds as on a cold cache, every file is checked again, and the entries are rewritten

#### Scenario: Corrupt code entry is a miss
- **WHEN** a program has been run with the cache, every entry is then truncated or overwritten with garbage, and `prism run --explain-cache app/main.prism` runs again
- **THEN** it prints what the cold run printed, lists every file as `checked` and every instance as `instantiate`, and the entries are rewritten

### Requirement: The Cache Never Fails a Check
A cache problem MUST NOT change the outcome of a check or a run: when the cache directory cannot be created, read or written, `prism check` and `prism run` MUST behave as with the cache disabled, and report no error for it.

#### Scenario: Unwritable cache directory
- **WHEN** `PRISM_CACHE` names a path that is a regular file, and `prism check` runs on a valid workspace
- **THEN** the check succeeds and prints the same report as without a cache

#### Scenario: Unwritable cache directory for a run
- **WHEN** `PRISM_CACHE` names a path that is a regular file, and `prism run app/main.prism` runs a valid program
- **THEN** the program prints the same output and exits with the same status as with `--no-cache`, and nothing is reported about the cache

### Requirement: Atomic Entry Writes
Each entry MUST be written to a temporary file in its final directory and then renamed into place, so that a reader sees either no entry or a complete one. Concurrent `prism` processes sharing a cache MUST NOT need a lock.

#### Scenario: Concurrent checks share a cache
- **WHEN** several `prism check` processes run at the same time on one workspace with one empty cache
- **THEN** every process succeeds with the same report, and a later check reuses every file

#### Scenario: Concurrent runs share a cache
- **WHEN** several `prism run` processes run the same program at the same time with one empty cache
- **THEN** every process prints the same output and exits with status 0, and a later run loads every unit and instance from the cache

### Requirement: Disabling the Cache
`prism check --no-cache` and `prism run --no-cache` MUST neither read nor write the cache, and `PRISM_CACHE=off` MUST disable the cache for every command.

#### Scenario: Checking without the cache
- **WHEN** a workspace is checked with `--no-cache`, or with `PRISM_CACHE=off`, after a check that filled the cache
- **THEN** every file is checked again, and no entry is written

#### Scenario: Running without the cache
- **WHEN** a program is run with `prism run --no-cache`, or with `PRISM_CACHE=off`, after a run that filled the cache
- **THEN** every file is checked and compiled again, every instance is instantiated again, and no entry is written

### Requirement: Explaining Cache Use
`prism check --explain-cache` MUST print, after a successful check's report, a line `Cache: <directory>` (or `Cache: off`), then for each file in check order, library modules first, `  checked <path>` or `  reused <path>`, with the path relative to the workspace root and `/` separators, or for a library module its module path and `.prism` (`prism/core.prism`), then the program's instances as "Instances Explained Once per Program" states.

#### Scenario: Explaining a warm check
- **WHEN** `app/main.prism` imports `/lib/box`, which exports `public fun <T> keep(x: T): T`, `app/main` calls `keep(1)`, and `prism check --explain-cache app/main.prism` runs, then `prism run app/main.prism`, then `prism check --explain-cache app/main.prism` again with no edit
- **THEN** the first check prints `  checked prism/core.prism` and `  checked prism/collections.prism` (or `reused`, when another workspace already checked them), `  checked lib/box.prism`, `  checked app/main.prism` and `  instantiate lib/box::keep<Int>`, and the second prints `  reused lib/box.prism`, `  reused app/main.prism` and `  reused instance lib/box::keep<Int>`

### Requirement: Instances Shown by Their Identity
`--explain-cache` MUST show an instance as the generic's module, `::`, for an extension the name of its receiver type without type arguments and `.`, the generic's name, and its type arguments in angle brackets. What it shows for an instance requested by a reused file MUST be what checking that file again would show.

#### Scenario: Explaining an instance of a generic extension
- **WHEN** `lib/box.prism` exports `public struct Box<T>(val value: T)` and `public fun <T> Box<T>.size(): Int`, `app/main.prism` calls `Box(1).size()`, and the extension's type parameter is renamed to `U` between two runs of `prism check --explain-cache app/main.prism`
- **THEN** the first run prints `  instantiate lib/box::Box.size<Int>`, and the second reuses `app/main.prism`, whose import's ABI hash ignores type parameter names, and shows the instance as `lib/box::Box.size<Int>` again

### Requirement: Code Entries Are Keyed by the Code's Inputs
A file's code entry MUST be keyed by its check key and the target identity. An instance's code entry MUST be keyed by the format version, the compiler's identity, the target identity, the instance's symbol, its generic's Monomorphization Hash and a hash of the definitions of the structs and enums its type arguments reach. The target identity MUST cover the target triple and every code generator setting, detected CPU features included.

#### Scenario: Another target generates code again
- **WHEN** a program run with one target identity is run with the same compiler under a different target identity, such as other CPU features or another optimization level
- **THEN** every check entry is reused and no code entry of the first target is

#### Scenario: An unchanged program compiles nothing
- **WHEN** `prism run --explain-cache app/main.prism` runs twice on an unchanged program that calls imported generics
- **THEN** the second run lists every file as `reused` and every instance as `reused instance`, compiles no function, and writes no entry

### Requirement: Generic Records Are Stored by Their Hash
When a file passes Phase 2, its check entry MUST hold its summary: each generic's Monomorphization Hash and generic calls, the instances its other bodies request, and whether it declares a valid `main`. Each generic's record MUST be stored in an entry keyed by the format version, the compiler's identity and its Monomorphization Hash. An uncached instance MUST be built from its record entry, or, when that is missing or invalid, from its generic's file checked again.

#### Scenario: A missing record is rebuilt from its file
- **WHEN** a program calling `keep(1)` of `lib/box` has been run with the cache, every record entry is then deleted, and `app/main.prism` is edited to also print `keep("a")`
- **THEN** the next `prism run --explain-cache app/main.prism` prints `1` and `a`, lists `lib/box.prism` and `app/main.prism` as `checked`, `lib/box::keep<Int>` as `reused instance` and `lib/box::keep<String>` as `instantiate`

### Requirement: Explaining a Run's Cache Use
`prism run --explain-cache` MUST print the lines of "Explaining Cache Use" to stderr before `main` runs, where `  checked <path>` means that the file's Phase 2 ran, `  compiled <path>` that its verdict was reused and its code generated, and `  reused <path>` that its code was loaded from the cache without checking or compiling it.

#### Scenario: Explaining a warm run
- **WHEN** `prism run --explain-cache app/main.prism` runs twice on the program of "Explaining a warm check", with no edit
- **THEN** the first run prints `  checked lib/box.prism`, `  checked app/main.prism` and `  instantiate lib/box::keep<Int>` to stderr, the second `  reused lib/box.prism`, `  reused app/main.prism` and `  reused instance lib/box::keep<Int>`, and both print `1` to stdout

#### Scenario: A checked program compiles its units
- **WHEN** `prism check app/main.prism` runs on that program with an empty cache, and then `prism run --explain-cache app/main.prism`
- **THEN** the run prints `  compiled lib/box.prism`, `  compiled app/main.prism` and `  instantiate lib/box::keep<Int>`

### Requirement: Instances Explained Once per Program
After the file lines, `--explain-cache` MUST list each instance the program needs once, in symbol order: `  instantiate <instance>` when the cache holds no code for it, which `prism run` then generates, or `  reused instance <instance>` when it does.

#### Scenario: Two callers of one instance
- **WHEN** `app/a.prism` and `app/b.prism` both import `/lib/box` and call `keep(1)`, `app/main.prism` imports both, and `prism check --explain-cache app/main.prism` runs
- **THEN** it prints `  instantiate lib/box::keep<Int>` once, after every file line

### Requirement: Reports List Workspace Files
The report of `prism check` MUST count and list only workspace files. Library modules MUST appear only in the lines of `--explain-cache`.

#### Scenario: A check report without library lines
- **WHEN** `prism check main.prism` runs on a file without imports
- **THEN** the report says `Total units: 1` and lists `main.prism` alone

## Design Rationale

Decisions from `abi-cache` (planned autonomously on 2026-10-17, reviewed on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **A per-user, content-addressed store outside the workspace** (D3). The cache lives in `$PRISM_CACHE` or the platform's per-user cache directory (Go's `os.UserCacheDir` rules), with a `CACHEDIR.TAG` so backup tools skip it. Writing nothing into the workspace means no `.gitignore` entry, read-only checkouts keep their cache, and nothing at the root can act as a marker file (the zero-marker invariant). No key holds an absolute path, so every checkout, worktree and copy of a workspace shares its entries. Rejected: a `<root>/.prism/` directory like Gradle's `.gradle/` (writes into the source tree, looks like a marker, and the root depends on the entry file); a per-user directory keyed by workspace path (identical checkouts would miss each other's entries).
- **What is cached before code generation** (D4). Check entries ("this file passes Phase 2 against these import interfaces", with its instance requests) and instance records, both replaced by `instance-cache`'s summaries, records and code entries (below). Failures are never cached: their diagnostics hold absolute paths and source excerpts, and a failing file is the one being edited. `.abi` entries are not cached yet, because the resolver parses every file to build the import graph and extracting an ABI from an AST in memory costs little next to parsing; they pay off together with cached import lists, once a warm run can skip parsing (a ROADMAP follow-up). Typed trees and MIR have no consumer across runs yet.
- **The compiler's identity is its version and a hash of its executable** (D5). During development the version never changes while the checker does, so a version-only key would reuse verdicts of an older compiler and hide regressions. Hashing the executable is how Go's build cache identifies development toolchains; `blake3` is built with `opt-level = 3` in the dev profile so this stays fast in debug builds. If the executable cannot be read, caching is off. Rejected: the version alone; a build-time hash of the sources (misses dependency and toolchain changes); the git commit (misses uncommitted edits).
- **Small, versioned, self-checking text entries** (D6; format 2 of `instance-cache` keeps the text header and checksum around a binary body, below). A header names the format version, kind and key, body lines hold instance requests, and a final line is a BLAKE3 checksum of everything before it. Text rather than a binary encoding: entries are a few hundred bytes, need no dependency and can be read with `cat`. A field containing a tab or newline cannot be written, so such a file is simply not cached. A format change that old readers would misread takes a new `vN` directory, so two compilers never fight over one entry.
- **Every cache problem is a miss** (D7). Reading never fails a check, and neither does writing: an unwritable cache is skipped silently, since it is not the user's error. An entry is written only after its file passes, so an interrupted check leaves only valid entries.
- **Atomic writes without locks** (D8). An entry is written to `<name>.tmp-<pid>-<counter>` in its final directory and renamed into place, which is atomic on POSIX and replaces the target on Windows, and the checksum catches a torn file after a crash. Entries are functions of their keys, so racing writers write identical bytes and need no lock. Rejected: lock files (stale after crashes, no benefit when writers agree); `fsync` per entry (a lost entry is only a miss).
- **The store is a module of `prism-cli`, and caching is opt-in** (D10). `check_path` stays uncached, so the fixture harness, library tests and embedders never touch a user's cache; `check_path_with` takes a cache explicitly and the `prism` binary opens one from the environment unless `--no-cache`. The store moves to its own crate when a driver outside `prism-cli` (the LSP) needs it; `prism run` uses it from the same crate (`instance-cache` D9). Rejected: caching in `check_path` by default, which would make every test depend on the state of the user's cache directory.
- **`--explain-cache` says what happened, not why** (D11). It appends to the unchanged default report, with root-relative `/`-separated paths so tests compare them on every OS. A content-addressed store keeps no history, so it cannot tell which input of a missed key changed; a per-module index of the last key's inputs would, and is a debugging aid to add if it is ever missed. The avoidance scenarios are integration tests in `crates/prism-cli/tests/cache_tests.rs` that edit files between runs of the binary, which the fixture harness cannot express.
- **Instances are shown by their identity** (D12, from the review). An extension's instance names its receiver by the type's name alone (`lib/box::Box.size<Int>`): the receiver's type arguments are the extension's own type parameters, whose names no interface hash covers, so showing them let a reused importer show a stale name.

Decisions from `instance-cache` (planned autonomously on 2026-10-17, implemented and reviewed on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **The cache also holds generated code** (D1). An instance key that guards no code is only bookkeeping, as `abi-cache`'s untested gaps showed; code keys are check keys or instance keys plus a target identity, sharing the entry format, and Cranelift compilation dominates Tier 1 time on large files. A warm `prism run` of an unchanged program therefore runs no Phase 2, lowering, instantiation or Cranelift pass. Rejected: a later `code-cache` change; caching only verdicts and records, which saves the cheaper half.
- **Summaries in check entries, records in entries of their own** (D5). A check entry holds the file's summary: whether it declares a valid `main`, the instance requests of its non-generic bodies, and each generic's path, type parameters, Monomorphization Hash and generic calls, a few dozen bytes per call, which is all `prism check` and planning read. Each record is a `record` entry keyed by its Monomorphization Hash, content-addressed across files and checkouts, read only when an instance's code misses; a missing or corrupt one makes the run check the generic's file again and rewrite its records, so it is an optimization, never a correctness input. Rejected: records inside check entries (a warm check would decode every generic's MIR); no record entries (every new instance would re-check its generic's file).
- **Code keys and the target identity** (D8). A file unit's code key is its check key and the target identity, since the check key already covers everything a file's code depends on; an instance's is its instance key (`compilation-and-abi`). The target identity is the triple and the `Display` of the shared and ISA-specific flags of both ISAs, detected CPU features included, so a cache on a shared home directory never runs code built for another CPU.
- **`prism run` uses the cache by default** (D9). Per file in topological order: `reused` when the check and code entries both hit (its artifact is defined as read), `compiled` when only the verdict hits (Phase 2 runs for its MIR), `checked` otherwise. The entry's `main` comes from its summary, and any error is found from source as without a cache, since errors are never cached. The library default has no cache, so the fixture harness and library tests stay hermetic; the harness runs every executable fixture cold and then warm on the cache the cold run filled. Rejected: an opt-in `--cache`; a separate `prism build`.
- **Instances are explained once per program** (D10, BREAKING). After the file lines, in symbol order, `instantiate` or `reused instance`, without the requester; under `prism check`, which generates no code, they say what the next run will do (one header read per instance, only under `--explain-cache`). Rejected: per-requester lines (an instance under every caller, transitive ones under none); instance lines in `prism run` alone.
- **Format 2: a text header, a binary body and a checksum** (D11). `prism-cache 2`, `kind`, `key` and `length` lines, then the body, then `checksum <hex>`; the kinds are `check`, `record`, `unit` and `instance`, and a body that does not decode is a miss like a bad checksum. Headers stay readable with `head`. Rejected: text bodies (a text codec for MIR and machine code); a store per kind.
- **Entries are written as soon as their unit passes** (D14). Every entry is a function of its key, a fact independent of whether another file of the run fails, so a run that fails later leaves only true entries; code entries are written only after `check_file` or `check_instances` accepts the unit, so a cached artifact never hides an `E6001`.
- **The executable's hash is recorded under its file metadata** (D16, from the review). Reading and hashing the whole executable was most of a warm run of a small program (3.7 of 4.1 ms in a release build). An `executable` entry under `<cache>/executables/` records it, keyed by the executable's path, size, modification time and, on Unix, its status-change time, device and inode, which a rebuild or replacement changes and a user cannot set back; it is written only when the metadata did not change while hashing and the file is at least two seconds old, as Git treats racily clean files. These entries live apart from `v2/`, because whether one is written depends on the file's age, so what a check or run writes among check, record and code entries stays a function of its inputs. Rejected: path, size and modification time alone (`ccache`'s default, which `cp -p` reproduces); a build-time identity (misses dependency changes, or is platform-specific).

Decisions from `standard-library` (2026-10-18):

- **Library modules are keyed like workspace files** (R2, D3 of `standard-library`). A library module's check entry is keyed by its module path, source text, imports' ABI hashes and the compiler identity, so its entries are shared by every workspace checked with one compiler and cache, and an injected test module never collides with the bundled one. A workspace file's key appends the prelude modules' ABI hashes after its explicit imports', since its Phase 2 sees them. The identity already covers the executable that embeds the library, which is redundant and cheap. `--explain-cache` lists library modules first, by module path, while the report counts workspace files only, so `Total units` keeps meaning the user's program.
