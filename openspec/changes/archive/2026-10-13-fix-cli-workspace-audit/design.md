# Design

## Context

An adversarial audit of `crates/prism-cli` revealed that `cli-workspace-driver` introduced an unconditional fatal check for cycles (`if graph.is_cyclic()`), contradicting `package-resolution/spec.md` (which explicitly supports mutually recursive imports in Phase 1 via SCC clustering). Additionally, `extract_cycle_path` inverted dependency edges due to alphabetical sorting, symlinks allowed workspace boundary escapes, and relative workspace roots bypassed jail validation.

## Goals / Non-Goals

**Goals:**
- Allow mutually recursive file imports to succeed Phase 1 ABI extraction in `prism check`, matching `DependencyGraph`'s SCC design and Invariant 6.
- Implement directed DFS in `extract_cycle_path` to find the exact directed cycle when needed.
- Enforce workspace boundary checks on entry files and scanned directory entries to prevent symlink jail escapes (`E1005`).
- Fix `import.rs` to treat empty relative workspace roots as `.` for disk existence checks.
- Connect parse errors to `E2001` with source code snippets.

**Non-Goals:**
- Checking recursive type layout without `Heap<T>` (belongs in Phase 2 semantic analysis / type checking).
- Whole-program codegen.

## Decisions

### Decision 1: Permit Mutually Recursive Imports in Phase 1
In `crates/prism-cli/src/workspace.rs`, remove the fatal check `if graph.is_cyclic()`. The topological order returned by `DependencyGraph` already yields SCC clusters dependency-first. For each cluster (whether size 1 or >1), `validate_and_extract_abi` iterates through all files and extracts `FileAbi`. Because all exported declarations require explicit types (Invariant 2), mutual recursion does not block Phase 1 ABI extraction.

### Decision 2: Directed DFS for Cycle Path Extraction
Replace the alphabetical slice in `extract_cycle_path` with a directed depth-first search that follows `node.direct_imports` to locate an actual cycle `u -> v -> ... -> u`.

### Decision 3: Workspace Boundary Validation for CLI Inputs
In `check_file` and `check_directory`, compute `canon_root = workspace_root.canonicalize()`. For every source file, verify `file.canonicalize()?.starts_with(&canon_root)`. If a symlink points outside the workspace root, fail with `ResolveError::WorkspaceBoundaryViolation` (`E1005`).

### Decision 4: Relative Root Normalization in `import.rs`
In `crates/prism-resolver/src/import.rs`, if `workspace_root == Path::new("")`, normalize to `Path::new(".")` when performing `.exists()` checks so canonicalization and boundary checks are not bypassed.

## Risks / Trade-offs

- **[Risk]** Self-referential imports (`import main` inside `main.prism`).
  → *Mitigation*: Handled naturally by `DependencyGraph`, which self-groups or can be flagged if self-import is disallowed.
