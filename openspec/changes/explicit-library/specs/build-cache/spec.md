## MODIFIED Requirements

### Requirement: Cache Keys Cover Exactly the Compilation Inputs
The key of a file's check entry MUST be a BLAKE3 hash of the cache format version, the compiler's identity, the file's module path, its exact source text, and the ABI hashes of its direct imports in import order, then, for a workspace file, those of the member modules `/prism/core` and `/prism/collections`, in that order. A library module MUST be keyed by the same inputs. The compiler's identity MUST cover the compiler version and the content of the compiler executable. No other input MUST affect a check key.

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
- **WHEN** two workspaces whose files import `/prism/compose/runtime` are checked one after the other with the same compiler and cache
- **THEN** the second check reuses the entries of `prism/core.prism`, `prism/collections.prism` and `prism/compose/runtime.prism` that the first wrote

#### Scenario: Reordering imports changes only the key
- **WHEN** a file's two imports are swapped and the workspace is checked again
- **THEN** that file is checked again, its importers are reused, and its diagnostics and ABI hash are unchanged
