# Proposal

## Why

An adversarial compiler audit of `crates/prism-cli` revealed a critical specification and implementation contradiction: `cli-workspace-driver` unconditionally aborted on cyclic imports with `E1007`, directly breaking `package-resolution`'s core requirement that mutually recursive imports form SCC clusters and succeed Phase 1 ABI extraction. Additionally, the audit identified an inverted cycle path diagnostic bug, symlink workspace boundary jail escapes, degraded parse error reporting, and relative workspace root normalization gaps.

## What Changes

- Reconcile `package-resolution` specification: confirm that mutually recursive imports are partitioned into Strongly Connected Components (SCCs) and successfully extract Phase 1 ABIs.
- Remove unconditional fatal cycle rejection from `prism check` in `crates/prism-cli/src/workspace.rs`, allowing mutually recursive SCC clusters to extract ABIs in topological order.
- Implement directed DFS cycle tracing in `extract_cycle_path` to avoid inverted or fabricated cycle diagnostics.
- Validate canonicalized file paths against canonical workspace roots for CLI entry files and scanned directories to prevent symlink workspace escapes (`E1005`).
- Normalize empty relative workspace roots (`""` -> `"."`) in `import.rs` so workspace boundary checks are never bypassed.
- Preserve source spans and line pointers on syntax errors encountered during workspace resolution, rendering `E2001` with source snippets.
- Add regression tests covering mutual recursion ABI extraction, symlink jail escapes, and accurate cycle tracing.

## Capabilities

### New Capabilities
<!-- None -->

### Modified Capabilities
- `package-resolution`: Reconcile `CLI Workspace Analysis and Dependency-Ordered Validation` to ensure mutually recursive imports succeed Phase 1 declaration extraction as SCC clusters.

## Impact

- `crates/prism-cli`: Updated `workspace.rs`, `diagnostics.rs`, and tests.
- `crates/prism-resolver`: Hardened `import.rs` and `scanner.rs` against symlink jail escapes and relative roots.
- `openspec/specs/package-resolution`: Reconciled scenario for mutually recursive imports.
