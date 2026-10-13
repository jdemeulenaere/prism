# Proposal

## Why

The compiler frontend currently features a complete lexer, Pratt parser, surface ABI extractor, and package resolver, but the developer CLI (`prism check`) only operates on single isolated files and lacks integration with `prism-resolver`. Developers cannot check multi-file projects, package directories, or transitively imported source trees from the CLI. Connecting `prism-resolver` into `prism-cli` completes Phase 1 of the compiler roadmap by providing end-to-end multi-file workspace analysis and dependency-ordered ABI extraction.

## What Changes

- Add `prism-resolver` dependency to `crates/prism-cli/Cargo.toml`.
- Enhance `prism check <path>`:
  - If `<path>` is a `.prism` file: discover workspace root, transitively resolve all imported dependencies (import-directed isolation), construct the dependency graph, detect cycles, and extract ABIs in topological order.
  - If `<path>` is a directory: scan directory recursively, validate package homogeneity for each directory, discover workspace root, construct the dependency graph across all discovered files, and validate ABIs in topological order.
- Implement structured, user-friendly CLI diagnostic error formatting for resolver errors (such as `PackagePathMismatch`, `InvalidImportPath`, `FileNotFound`, `WorkspaceBoundaryViolation`, `PackageHeterogeneity`, and cyclic dependencies).
- Add end-to-end CLI integration test suite with multi-file project fixtures testing root imports, sibling imports, cycle detection diagnostics, and package directory checks.
- Update `ROADMAP.md` Phase 1 checklist to mark frontend, resolver, and CLI check deliverables complete.

## Capabilities

### New Capabilities
<!-- None -->

### Modified Capabilities
- `package-resolution`: Add requirement for CLI workspace analysis, entry-point transitive resolution, and dependency-ordered validation.

## Impact

- `crates/prism-cli`: Updated `Cargo.toml`, rewritten `run_check` in `src/main.rs`, and new multi-file CLI integration tests.
- `ROADMAP.md`: Phase 1 milestones updated.
