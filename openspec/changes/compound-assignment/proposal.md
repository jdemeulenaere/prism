# Proposal: compound-assignment

## Why

Prism has no compound assignment, so every counter, accumulator and string builder is written `total = total + x`, and a target that is expensive or impossible to repeat (`*pick(&mut a, &mut b) = *pick(&mut a, &mut b) + 1`) cannot be updated at all without a temporary reference. Kotlin users expect `+=`. This is item 11 of "Then: the road to Milestone 2" in `ROADMAP.md` (and the Phase 2 item deferred by `lexer-parser`). It lands before code generation so the backend lowers it from day one, and before compose, whose state updates (`count += 1`) are its most common use.

## What Changes

- **Compound assignment statements** `target += value`, `-=`, `*=`, `/=` and `%=`. They are statements wherever `=` is: in a block or closure body, and as the unbraced body of an `if`/`else` branch, a `when` branch or a `while`/`for` loop. Their targets are those of `=`: a variable, a `.` field path (including an implicit `this` field), an index, or a write through `*` (`*r += 1`). Chained (`a += b += c`), parenthesized or used as a value (`val y = (x += 1)`, `g(x += 1)`), they are `E2001`. A newline before the operator ends the previous expression, as for `=`.
- **Meaning: `t op= v` is `t = t op v` with `t` evaluated once.** The target's place is evaluated first, then its current value is read (copied for a `Copy` type, borrowed for `String`), then the value is evaluated, then the operator runs and the result is written, dropping the old value when the type needs a drop. The operator follows the existing operator rules: `Int` and `Float` operands of the same type, and `String += String` (concatenation). The value is an operand, read through references and never consumed, so `s += t` leaves `t` usable.
- **Diagnostics reuse existing codes.** A `val` binding, a `val` field, a field of an immutable value and a write through `&` are `E4003`, as for `=`. An operator that does not apply to the target and value types (`x += "a"` for `x: Int`, `s -= "a"`, `n += 1` for `n: Int?`) is `E4002`, with help naming the operand types the operator accepts. A compound assignment to a binding of reference type (`r += 1` for `r: &mut Int`) is `E4002` for a `var` and `E4003` for a `val`, with help suggesting `*r += 1`, as `r = 5` already is.
- **Typed tree:** a dedicated compound assignment statement over a place, recording its operator, so the target appears once. **MIR:** no new construct. The statement lowers to an ordinary operator rvalue assigned to the place, reading the place (`_1 = copy _1 + const 1`), so the borrow checker sees a read and then a write of the same place: `x += 1` while `x` is borrowed is `E5004` (also `E5003` when the borrow is mutable), a closure doing `count += 1` captures `count` mutably, and a moved `String` target is `E5001`.
- **`++` and `--` are reserved and rejected.** The lexer reads `++` and `--` as tokens, and the parser rejects them anywhere with `E2001` and help suggesting `x += 1` / `x -= 1`. **BREAKING:** `a--b` and `--x`, which parse today as subtracting or negating a negation, now need a space or parentheses (`a - -b`, `-(-x)`). No fixture or example uses them.
- **Not in scope (deferred):** bitwise operators and their compound forms (`&=`, `|=`, `shl`), which need a decision on spelling (`&` is the borrow operator, `>>` closes nested type arguments, Kotlin uses named infix functions) and on sized integer types; overloaded operators for user types (`plus` and `plusAssign`, which need traits); compound assignment through safe navigation (`a?.b += 1`, rejected like `a?.b = 1`); an in-place `String` append in the backend (the semantics already allow it).

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `syntax`: compound assignment statements and their targets, `++`/`--` rejected, and "Assignment Is a Statement" covering both forms.
- `type-system`: typing of compound assignment: target mutability, operator applicability, the value as a non-consuming operand, reference-binding targets.
- `typed-tree`: a compound assignment statement over a place, with its operator.
- `mir`: lowering of compound assignment: the place evaluated once, the old value read before the value, the result written with a drop of the old value.
- `references`: the borrow checker treats a compound assignment as a read and then a write of its target, including closure capture modes.

## Impact

- `crates/prism-lexer`: tokens `PlusEqual`, `MinusEqual`, `StarEqual`, `SlashEqual`, `PercentEqual`, and the reserved `PlusPlus` and `MinusMinus`.
- `crates/prism-ast`: `StatementKind::CompoundAssign { operator, target, value }`.
- `crates/prism-parser`: `parse_assignment_or_expression` accepts the compound operators wherever it accepts `=`, with the same target check; the chained-assignment and "assignment is a statement" errors cover them; `++`/`--` get a targeted `E2001`.
- `crates/prism-abi`: the span-free body hash covers the new statement.
- `crates/prism-typechecker`: `check_compound_assignment`, sharing target typing with `check_assignment` and the arithmetic rule with `infer_binary_op`.
- `crates/prism-typed-tree`: `StatementKind::CompoundAssign`, its dump line and its validation.
- `crates/prism-mir`: lowering of the new statement. `crates/prism-borrowck` needs no new analysis; at most a deduplication so one compound assignment does not report two conflicts at the same span.
- `crates/prism-cli`: fixtures under `tests/fixtures/{syntax,type-system,typed-tree,mir,references}/` for every new scenario.
- `openspec/specs/diagnostics/spec.md`: the `E2001`, `E4002` and `E4003` rows mention compound assignment. No new code.
- `ROADMAP.md`, `docs/decisions-for-review.md`: the decisions below, and on archive the deferred items.
