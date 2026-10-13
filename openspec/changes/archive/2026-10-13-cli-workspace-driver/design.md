# Design

## Context

The compiler workspace contains all components needed for Phase 1 compilation:
- `prism-lexer`: Fast tokenization and spans.
- `prism-parser`: AST construction and Pratt parsing.
- `prism-abi`: Phase 1 surface declaration extraction and deterministic ABI hashing.
- `prism-resolver`: Zero-config root discovery, import path resolution, directory scanning, and dependency graph construction with Tarjan's SCC cycle decomposition.

Currently, `crates/prism-cli/src/main.rs` only processes single file arguments directly in `run_check` by calling `parse_source` and `FileAbi::extract`, omitting any interaction with `prism-resolver`. As a result, import statements (`import foo`, `import /core/fmt`) are never resolved against the disk, package homogeneity is unchecked, and multi-file projects cannot be verified.

## Goals / Non-Goals

**Goals:**
- Add `prism-resolver` to `prism-cli/Cargo.toml`.
- Dispatch `prism check <path>` based on file system metadata:
  - For a `.prism` file: run single-file entry check with transitive import resolution (import-directed isolation).
  - For a directory: scan directory recursively, enforce package homogeneity per directory, discover workspace root, and build workspace dependency graph.
- Execute Phase 1 AST parsing and ABI extraction on all reachable nodes in topological dependency order.
- Render user-friendly diagnostic messages for all `ResolveError` variants (`E1001` through `E1006`) with source spans and context.
- Create CLI integration tests testing multi-file workspaces, root imports, sibling imports, package mismatch errors, and circular imports.

**Non-Goals:**
- Phase 2 semantic analysis, type checking, or borrow checking.
- Code generation or native execution (Cranelift/LLVM).
- Code formatting command (`prism fmt`).

## Decisions

### Decision 1: Input Path Dispatching
`run_check(path)` will inspect `std::fs::metadata(path)`:
1. **File Target**:
   - Must end with `.prism`.
   - Read source text, parse AST.
   - Extract declared package from AST.
   - Discover workspace root using `discover_root(file_path, package)`.
   - Transitively resolve all imports using `resolve_import`, constructing a `DependencyGraph`.
   - Check and extract ABI for the entry file and all transitively imported modules in topological order.
2. **Directory Target**:
   - Discover all `.prism` files recursively.
   - For each directory containing `.prism` files, run `scan_directory` to enforce package homogeneity (`E1003`).
   - Discover workspace root from the files.
   - Build a `DependencyGraph` including all discovered files and resolve their imports.
   - Topologically sort the graph and verify AST + ABI for each file.

*Alternative Considered*: Require a project manifest file (like `Prism.toml`). Rejected because it violates non-negotiable Invariant 3 (Zero-Config, Zero-Marker Root Discovery).

### Decision 2: Import-Directed Isolation for Single File Invocations
When `prism check path/to/main.prism` is run:
Only files that are transitively imported from `main.prism` will be parsed and validated. Unimported sibling files in the same directory will be ignored. This strictly upholds `Requirement: Import-Directed Compilation Isolation`.

### Decision 3: Topological Execution & SCC Handling
The resolved files will be ordered using `DependencyGraph::topological_sort()`.
If the graph contains cycles (SCC clusters > 1), Phase 1 declaration ABI extraction still succeeds as long as mutual imports are valid file references, conforming to `Scenario: Mutually recursive sibling imports`. If an import cannot be resolved, an explicit `FileNotFound` (`E1002`) or `InvalidImportPath` (`E1006`) error diagnostic is printed and execution terminates with exit code 1.

### Decision 4: Compiler-Grade Diagnostic Formatting
Errors from lexing, parsing, ABI extraction, and resolution will be formatted consistently:
```text
error[E1001]: package path mismatch
  --> src/models.prism:1:1
   |
 1 | package app.views
   | ^^^^^^^^^^^^^^^^^
   = note: directory path 'src' does not match declared package 'app.views'
```
Where source spans are available, the snippet will be displayed. When resolving files or directory boundaries, canonicalized paths will be reported.

## Risks / Trade-offs

- **[Risk]** Differing path separators and case sensitivity across operating systems (macOS / Windows / Linux).
  → *Mitigation*: Rely strictly on `prism_resolver::normalize_path` and `verify_disk_casing_exists`, which already contain dedicated platform-aware normalization.
- **[Risk]** Large workspace directory traversal performance.
  → *Mitigation*: Directory scanning collects `.prism` files iteratively without loading full file bodies into memory until needed during the topological parsing stage.
