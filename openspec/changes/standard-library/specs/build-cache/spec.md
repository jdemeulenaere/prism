# Build Cache Delta

## MODIFIED Requirements

### Requirement: Cache Keys Cover Exactly the Compilation Inputs
The key of a file's check entry MUST be a BLAKE3 hash of the cache format version, the compiler's identity, the file's module path, its exact source text, and the ABI hashes of its direct imports in import order, followed, for a workspace file, by those of the prelude modules in prelude order. A library module's entry MUST be keyed by the same inputs, so its source text is in its key as a workspace file's is. The compiler's identity MUST cover the compiler version and the content of the compiler executable. No other input MUST affect a check key.

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

### Requirement: Explaining Cache Use
`prism check --explain-cache` MUST print, after a successful check's report, a line `Cache: <directory>` (or `Cache: off`), then for each file in check order, library modules first, `  checked <path>` or `  reused <path>`, with the path relative to the workspace root and `/` separators, or for a library module its module path and `.prism` (`prism/core.prism`), then the program's instances as "Instances Explained Once per Program" states.

#### Scenario: Explaining a warm check
- **WHEN** `app/main.prism` imports `/lib/box`, which exports `public fun <T> keep(x: T): T`, `app/main` calls `keep(1)`, and `prism check --explain-cache app/main.prism` runs, then `prism run app/main.prism`, then `prism check --explain-cache app/main.prism` again with no edit
- **THEN** the first check prints `  checked prism/core.prism` and `  checked prism/collections.prism` (or `reused`, when another workspace already checked them), `  checked lib/box.prism`, `  checked app/main.prism` and `  instantiate lib/box::keep<Int>`, and the second prints `  reused lib/box.prism`, `  reused app/main.prism` and `  reused instance lib/box::keep<Int>`

## ADDED Requirements

### Requirement: Reports List Workspace Files
The report of `prism check` MUST count and list only workspace files. Library modules MUST appear only in the lines of `--explain-cache`.

#### Scenario: A check report without library lines
- **WHEN** `prism check main.prism` runs on a file without imports
- **THEN** the report says `Total units: 1` and lists `main.prism` alone
