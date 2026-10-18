# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/<spec>/`, one case per scenario, or per form of a scenario with several (`<scenario-slug>--<form>`), as the header of `fixture_tests.rs` describes. `typed-tree` and `mir` cases hold snapshots (`PRISM_BLESS=1`), each read against its scenario's THEN before committing.

## 1. Tokens and grammar (D1, D8)

- [x] 1.1 In `prism-lexer`, add `PlusEqual`, `MinusEqual`, `StarEqual`, `SlashEqual`, `PercentEqual`, `PlusPlus` and `MinusMinus` (with `Display`), lexed greedily after `+`, `-`, `*`, `/`, `%` (keeping `->` and comments). Verify with lexer tests for "Compound operators are single tokens", including `a = -b`, `a->b`, `a == b` and `a /* c */ = b`, and that `cargo test -p prism-lexer` passes.
- [x] 1.2 In `prism-ast`, add `StatementKind::CompoundAssign { operator: BinaryOp, target, value }` with `Display` (`x += 1`). In `prism-parser`, accept the compound operators in `parse_assignment_or_expression` with the target check of `=`, give them `Precedence::Assign`, extend the "assignment is a statement" error in `parse_infix` and the chained checks (statement and `parse_initializer`) to them, and reject `++`/`--` in `parse_prefix` and `parse_infix` (at `Precedence::Postfix`) with the D8 message. In `prism-abi`, hash the new statement in `hash_statement_span_free` with its own tag and the operator. Verify with parser tests for every scenario of "Compound Assignment", "No Increment or Decrement Operators" and the modified "Assignment Is a Statement", fixtures under `syntax/` for each (expecting `ok` or `E2001`), and that `cargo test -p prism-parser -p prism-abi` passes.

## 2. Typing (D3, D7)

- [x] 2.1 In `prism-typechecker`, split the target typing out of `check_assignment` and add `check_compound_assignment`: target typing and the `val`-binding check shared with `=`; a reference binding target rejected per D7 (`E4003` for a `val`, `E4002` for a `var`, help `*r += v`); the value inferred with `infer_operand` outside a consuming position; the arithmetic rule shared with `infer_binary_op`; a mismatch reported as the new `TypeError::OperatorMismatch` (`E4002`) with D7's help. Build `TreeStatementKind::CompoundAssign` (added in 3.1, so do 3.1 first or together). Render `OperatorMismatch` in `prism-cli`. Verify with fixtures under `type-system/` for every scenario of "Compound Assignment Typing" and "Compound Assignment to a Reference Binding" (one case per form where a scenario has several), and that `cargo test -p prism-typechecker` passes.
- [x] 2.2 In `openspec/specs/diagnostics/spec.md`, widen the `E2001` row (`++`/`--`, compound assignment in expression position), the `E4002` row (`OperatorMismatch`: an operator that does not apply to a compound assignment's target and value) and the `E4003` row (compound assignment to a non-writable target). Verify that the fixture harness's catalog check passes.

## 3. Typed tree (D4)

- [x] 3.1 In `prism-typed-tree`, add `StatementKind::CompoundAssign { operator: BinaryOperator, target, value }`, its dump line (`compound-assign + @l:c` with `target:` and `value:` children), its validation (target a place, operator arithmetic, value of the target's type) and its children in the binding collector. Verify with a validator unit test rejecting a compound assignment whose value has another type, fixtures under `typed-tree/` with snapshots for every scenario of "Compound Assignments Are Statements Over Places", and that every fixture's tree still validates.

## 4. MIR and the borrow checker (D5, D6)

- [x] 4.1 In `prism-mir`, lower `CompoundAssign` per D5 in its own temporary scope: the place once, the old value read (copy, stabilized when the value may write, or a shared operator borrow), the value, the operator, and for a type that needs a drop a temporary, a drop and a move. Verify with fixtures under `mir/` with snapshots for every scenario of "Compound Assignments Read Then Write Their Target", and that the MIR validator accepts every fixture and example.
- [x] 4.2 Confirm that `prism-borrowck` needs no change (D6): a compound assignment conflicting with one borrow is reported once, as `E5004`. Verify with fixtures under `references/` for every scenario of "Compound Assignments Read and Write Their Target" (one case per form), and that the existing `references/` and `ownership/` fixtures, including `x = x + 1` reporting both codes, still pass.

## 5. Integration

- [x] 5.1 Use compound assignment in `examples/` where a counter or accumulator reads better with it. Run the apply guidance in `openspec/config.yaml`: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets` (no new warnings), `cargo test --workspace`, `prism check` on every file under `examples/` (only `examples/demo/ui` may fail), and `openspec validate --all --strict`. Verify all pass.

## Workflow follow-up

- On archive, promote D1 and D8 into `syntax`'s `## Design Rationale`, D2, D3 and D7 into `type-system`'s, D4 into `typed-tree`'s, D5 into `mir`'s and D6 into `references`'. D9's analysis goes with the ROADMAP item.
- In `ROADMAP.md`, tick item 11 of "Then: the road to Milestone 2" and the compound-assignment half of the Phase 2 item "Compound assignment (`+=`, `-=`) and bitwise operators", and add as unchecked items, each naming `compound-assignment`:
  - a `bitwise-operators` change deciding the spelling (`&`/`|`/`^`/`<<`/`>>` versus Kotlin's `and`/`or`/`xor`/`shl`/`shr`), with sized integer types, and their compound forms (D9);
  - an in-place `String` append for `s += t` in the backend (`runtime-strings-and-drops`, D3);
  - moving the binary arithmetic operators' mismatch to `OperatorMismatch` (D7);
  - user-defined operators (`plus`, `plusAssign`) once traits exist.
