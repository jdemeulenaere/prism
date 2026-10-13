# Tasks

## 1. Scaffolding & Error Diagnostics

- [x] 1.1 Add `prism-resolver` dependency to `crates/prism-cli/Cargo.toml` and verify `cargo check -p prism-cli` compiles successfully
- [x] 1.2 Implement diagnostic error formatting module in `crates/prism-cli/src/diagnostics.rs` mapping `ResolveError` (`E1001` through `E1006`), parse errors, and ABI errors into compiler diagnostics, and verify formatting with unit tests

## 2. Single-File Transitive Check & Import-Directed Isolation

- [x] 2.1 Implement `check_file` in `crates/prism-cli`: discover workspace root, resolve all transitive imports with `prism-resolver`, construct `DependencyGraph`, and verify AST and ABI in topological order; verify with unit test
- [x] 2.2 Enforce import-directed isolation so unimported sibling files in the same directory are excluded from parsing; verify with unit test

## 3. Directory Scanning & Multi-Package Workspace Check

- [x] 3.1 Implement `check_directory` in `crates/prism-cli`: scan directory recursively, enforce package homogeneity per directory using `scan_directory`, build workspace-wide dependency graph, and extract ABIs; verify with unit test
- [x] 3.2 Implement circular import cycle detection and human-readable diagnostic reporting in `prism check`; verify with unit test

## 4. End-to-End Integration Testing & Roadmap Update

- [x] 4.1 Create multi-file integration test fixtures in `crates/prism-cli/tests/cli_workspace_tests.rs` covering root imports (`/core/fmt`), sibling imports, package homogeneity errors, and cycle diagnostics; verify `cargo test -p prism-cli` passes
- [x] 4.2 Update `ROADMAP.md` Phase 1 checklist to reflect completion of the frontend parser, AST validation, resolver, and CLI workspace checker, and verify full workspace `cargo test` passes
