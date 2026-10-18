# Build Cache Specification Delta

## MODIFIED Requirements

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
The key of a file's check entry MUST be a BLAKE3 hash of the cache format version, the compiler's identity, the file's module path, its exact source text, and the ABI hashes of its direct imports in import order. The compiler's identity MUST cover the compiler version and the content of the compiler executable. No other input MUST affect a check key. The hash of the executable's content MAY be read from an entry under the cache's `executables` directory keyed by the executable's path, size, modification and status-change times and file identity, which MUST be written only for an executable whose metadata did not change while it was hashed and that is at least two seconds old.

#### Scenario: Unchanged workspace reuses every file
- **WHEN** `prism check` runs twice on an unchanged workspace with the same compiler
- **THEN** the second run reuses every file

#### Scenario: Comment-only edit re-checks the file alone
- **WHEN** a comment is added to a file imported by others
- **THEN** that file is checked again, because its source text changed, and its importers are reused, because its ABI hash did not change

#### Scenario: A different compiler re-checks everything
- **WHEN** a workspace checked by one compiler executable is checked by a different one, of the same version or not
- **THEN** no entry of the first is reused

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
`prism check --explain-cache` MUST print, after a successful check's report, a line `Cache: <directory>` (or `Cache: off`), then for each file in check order `  checked <path>` or `  reused <path>`, with the path relative to the workspace root and `/` separators, then the program's instances as "Instances Explained Once per Program" states.

#### Scenario: Explaining a warm check
- **WHEN** `app/main.prism` imports `/lib/box`, which exports `public fun <T> keep(x: T): T`, `app/main` calls `keep(1)`, and `prism check --explain-cache app/main.prism` runs, then `prism run app/main.prism`, then `prism check --explain-cache app/main.prism` again with no edit
- **THEN** the first check prints `  checked lib/box.prism`, `  checked app/main.prism` and `  instantiate lib/box::keep<Int>`, and the second prints `  reused lib/box.prism`, `  reused app/main.prism` and `  reused instance lib/box::keep<Int>`

### Requirement: Instances Shown by Their Identity
`--explain-cache` MUST show an instance as the generic's module, `::`, for an extension the name of its receiver type without type arguments and `.`, the generic's name, and its type arguments in angle brackets. What it shows for an instance requested by a reused file MUST be what checking that file again would show.

#### Scenario: Explaining an instance of a generic extension
- **WHEN** `lib/box.prism` exports `public struct Box<T>(val value: T)` and `public fun <T> Box<T>.size(): Int`, `app/main.prism` calls `Box(1).size()`, and the extension's type parameter is renamed to `U` between two runs of `prism check --explain-cache app/main.prism`
- **THEN** the first run prints `  instantiate lib/box::Box.size<Int>`, and the second reuses `app/main.prism`, whose import's ABI hash ignores type parameter names, and shows the instance as `lib/box::Box.size<Int>` again

## ADDED Requirements

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
