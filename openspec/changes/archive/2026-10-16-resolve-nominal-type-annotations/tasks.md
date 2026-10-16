# Tasks

## 1. Nominal Type Resolution

- [x] 1.1 Add `ScopeStack::lookup_type` in `crates/prism-typechecker/src/scope.rs` returning the innermost `Struct`/`Enum` symbol and skipping value symbols, and verify with a unit test that a shadowing local variable does not hide the struct
- [x] 1.2 Add `TypeChecker::resolve_nominal` in `checker.rs` (D1) and use it at the four member-reading sites: `infer_member_access`, `check_assignment`, the `BorrowMut` field check, and `infer_when_expr` exhaustiveness; verify that `./target/debug/prism check examples/demo/models/models.prism` passes
- [x] 1.3 Add typechecker regression tests (parsing Prism source) for: field read by value, through `&T`, `&mut T` field assignment (plus `val` field rejected with `E4003`), nullable safe navigation yielding `String?`, annotated `val p: Point`, chained `order.user.name`, an enum-typed parameter in an exhaustive `when`, and an unknown nominal type still emitting `E4006`; verify they pass with `cargo test -p prism-typechecker`
- [x] 1.4 Add a typechecker regression test for a struct ingested from a `FileAbi` produced by `FileAbi::extract` on another source file, accessed through a parameter annotation, and verify it passes
- [x] 1.5 Add CLI integration tests in `crates/prism-cli/tests/cli_workspace_tests.rs` for the byValue/byRef/byMut repro and for field access on a struct imported via sibling import, and verify they pass with `cargo test -p prism-cli`

## 2. Missing Return Type Diagnostic

- [x] 2.1 Add `TypeError::MissingReturnType` (`E4011`) in `error.rs` and emit it from `register_function_decl` instead of `CallSignatureMismatch`; update the unit test expectation and verify that `public fun f(x: Int) { }` reports `E4011` and not `E4004`
- [x] 2.2 Add an `E4011` help line in `crates/prism-cli/src/diagnostics.rs` and a CLI integration test asserting the code and help text, and verify it passes

## 3. Integration Verification

- [x] 3.1 Run `cargo test --workspace`, `cargo clippy --workspace --all-targets` (0 warnings), `cargo fmt --all -- --check`, and `openspec validate --all --strict`, and verify all are clean

## Workflow follow-up

- Archive the change with `/opsx:archive`, merging the `type-system` delta into `openspec/specs/type-system/spec.md`.
- Verify `openspec validate --all --strict` still passes after archive.
