# Tasks: typechecker-soundness

Each task's tests go in `crates/prism-typechecker/tests/regressions.rs`, which runs real `.prism` source, or in `crates/prism-cli/tests/` when the case needs module paths from a real workspace. A task is done when its tests pass and `cargo test --workspace` is green.

## 1. Rebase

- [x] 1.1 Confirm the plain-fix commits on `crates/prism-typechecker` are in place and `harden-parser-grammar` is applied. Rebase onto both. Verify `cargo test --workspace` passes, and that the repros for findings 3, 6, 7, 11, 12, 15, 16 and 20 still fail as described in proposal.md.
- [x] 1.2 Add MODIFIED blocks for `harden-parser-grammar`'s requirements "Parameterless Closures Are Checked as Closures" and "Composable Function Types Keep Their Compose Flag": their argument-site scenarios become `E4004` (D2). Verify with `openspec validate typechecker-soundness --strict`. Skip this task if the user picks D2's alternative.

## 2. Nominal identity (D3)

- [x] 2.1 Introduce `NominalId`, `TypeTable`/`TypeDef`, `Type::Struct(NominalId)`, `Type::Enum(NominalId)` and `Type::Opaque`. Thread the module path through `TypeChecker::new`, with the CLI deriving it from the root-relative path. Port the hand-built unit tests. Verify `cargo test --workspace` stays green with no behavior change.
- [x] 2.2 Bind annotation names eagerly to an ID, remove `resolve_nominal`, and make `Range` a `<builtin>` type. Verify with tests for the scenarios "Built-in Range distinct…", "Struct and enum never equal" and "Annotation and constructor denote the same type".
- [x] 2.3 Add `module` to `FileAbi`. On ingestion, resolve ABI names against the exporter's own exports first (the interim rule). Verify with a multi-file test for "Same-named structs from different modules stay distinct".

## 3. Mismatch codes (D2)

- [x] 3.1 Add the expectation origin and `TypeError::ArgumentTypeMismatch` (`E4004`). Update the `diagnostics` catalog descriptions of `E4002`, `E4004` and `E4005` and the CLI help lines. Update the existing tests that change code (`mutable_reference_stays_invariant`, `branches_join_with_null_into_nullable_type`, and the `checker.rs` unit tests). Verify with tests for every scenario of "Return and Argument Mismatch Codes".

## 4. Reading through references (D1)

- [x] 4.1 Peel references for operator operands and conditions. Verify with tests for every scenario of "Operators Read Through References".

## 5. Inference (D5, D6)

- [ ] 5.1 Add `TypeError::CannotInferType` (`E4013`), with its `diagnostics` catalog row and CLI help line. Add inference states and the fresh body context. Verify with tests for every scenario of "Order-Independent Inference of Private Declarations".
- [ ] 5.2 Report `E4013` for closure parameters that have no expected type. Verify with tests for every scenario of "Closure Parameter Inference".

## 6. Ambiguous imports (D7)

- [ ] 6.1 Add the import layer with candidate sets, and `TypeError::AmbiguousImport` (`E4012`) with its catalog row and CLI help line. Verify with multi-file tests for every scenario of "Ambiguous Imported Names Rejected at Use".

## 7. Discarded block values (D8)

- [ ] 7.1 Add the `discard` flag for `Unit` function and closure bodies. Reuse the flag if the plain fix for finding 19a has already added one. Verify with tests for every scenario of "Discarded Block Values".

## 8. Structured ABI types (D4)

- [ ] 8.1 Add `AbiType` with tagged structural hashing and `FileAbi::extract(file, module, resolver)`. Verify with `prism-abi` tests:
  - the hash differs between `(Int) -> Int?` and `((Int) -> Int)?`;
  - the existing relative-hash tests still pass;
  - the compilation-and-abi scenarios hold.
- [ ] 8.2 Add Phase 1b binding in the CLI, delete `Type::parse_str` and the interim rule from task 2.3, and convert `AbiType` to `Type` directly. Verify with multi-file tests for "Imported Signatures Keep Their Declared Types".

## 9. Types reachable only through an import (D9)

- [ ] 9.1 Add `TypeError::MemberOfUnimportedType` (`E4014`), with its `diagnostics` catalog row and CLI help line. Report it on member access, field assignment, variant matching and construction of a nominal type whose defining module is not a direct import. Verify with multi-file tests for every scenario of "Members of a Type Require Importing Its Module".

## 10. Integration

- [ ] 10.1 Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`, `prism check` on every file under `examples/` (only `examples/demo/ui` may fail), and `openspec validate --all --strict`. Verify that all pass.

## Workflow follow-up

- At archive:
  - rewrite the type-system rationale bullet "Nominal types resolve lazily" to describe D3;
  - promote D1, D2, D5, D6, D7, D8 and D9 into the type-system rationale;
  - amend the compilation-and-abi rationale with Phase 1b (D4).
- Add these follow-ups to `ROADMAP.md`, naming `typechecker-soundness`:
  - writing through a `&mut` primitive;
  - copying a value out of a borrow (`Copy` rules);
  - whether local declarations should shadow imports;
