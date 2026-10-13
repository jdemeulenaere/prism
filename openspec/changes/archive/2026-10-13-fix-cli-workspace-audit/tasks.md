# Tasks

## 1. Reconcile Mutual Recursion & Cycle Tracing

- [x] 1.1 Remove fatal cycle check from `crates/prism-cli/src/workspace.rs` so mutually recursive SCC clusters succeed Phase 1 ABI extraction, and verify with unit tests
- [x] 1.2 Implement directed DFS cycle tracing in `extract_cycle_path` to avoid inverted or fabricated cycle diagnostics, and verify with unit tests

## 2. Workspace Boundary Hardening & Normalization

- [x] 2.1 Enforce workspace boundary checks for entry files and scanned directory entries against canonical root in `workspace.rs`, and verify symlink escape rejection (`E1005`) with a test
- [x] 2.2 Normalize empty relative workspace root (`""` -> `"."`) in `crates/prism-resolver/src/import.rs`, and verify relative workspace root boundary check with a test

## 3. Diagnostics & Integration Verification

- [x] 3.1 Preserve source code snippets and spans for syntax errors (`E2001`) in CLI workspace diagnostics, and verify with a test
- [x] 3.2 Update CLI integration tests in `crates/prism-cli/tests/cli_workspace_tests.rs` to verify mutual recursion succeeds and symlink jail escapes are rejected, and verify full workspace `cargo test` passes
