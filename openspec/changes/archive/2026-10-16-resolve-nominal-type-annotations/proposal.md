# Proposal

## Why

The Phase 2 typechecker lowers every non-primitive named type annotation (`user: User`, `&User`, `&mut User`, `User?`, `val p: Point`, and type strings read from imported `FileAbi`s) to a field-less placeholder `Type::Struct { name, fields: [] }` and never looks the name up again. Member access and field assignment then find no fields and emit `E4006`, so field access only works on values produced directly by a constructor call. This rejects ordinary programs — including `examples/demo/models/models.prism` — and blocks the upcoming borrow checker, which needs correct types through references.

Separately, a `public`/`package` function that omits its return type is reported as `E4004` (call signature mismatch) with help text about argument counts, which points the user at the wrong problem.

## What Changes

- Named type annotations resolve to the struct or enum declared in the current file or ingested from an imported `FileAbi`, so field reads, field writes, safe navigation, `&mut` field borrows, and `when` exhaustiveness work on parameters, references, nullable values, annotated locals, and imported types — including nested chains like `order.user.name`.
- Names that resolve to no declared type (e.g. not-yet-supported builtins like `UInt64`, or generic type parameters) keep today's behavior: they remain opaque nominal types and only fail if a member is accessed on them.
- A missing return type on an exported function gets a dedicated diagnostic, `MissingReturnType` (`E4011`), with help text telling the user to declare the return type. It no longer reuses `E4004`. Which programs are accepted or rejected does not change.
- Regression tests at the typechecker and CLI level for each affected case.

## Capabilities

### New Capabilities
<!-- None -->

### Modified Capabilities
- `type-system`: The struct/nominal type requirement gains scenarios saying field access resolves through annotated types (by value, `&T`, `&mut T`, `T?`, annotated `val`, imported ABI structs). The function signature requirement gains the `E4011` missing-return-type diagnostic, added to the error catalog.

## Impact

- `crates/prism-typechecker`: `checker.rs` (nominal type resolution at member access, field assignment, `&mut` field borrow, `when` exhaustiveness; `E4011` emission), `scope.rs` (type-namespace lookup), `error.rs` (new `MissingReturnType` variant).
- `crates/prism-cli`: `diagnostics.rs` help text for `E4011`; new integration tests in `tests/cli_workspace_tests.rs`.
- `openspec/specs/type-system`: new scenarios and catalog entry.
- No ABI format, hashing, or parser changes.
