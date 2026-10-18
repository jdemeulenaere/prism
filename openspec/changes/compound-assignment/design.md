# Design: compound-assignment

## Context

See `proposal.md` for motivation. This change is item 11 of "Then: the road to Milestone 2" in `ROADMAP.md`, planned autonomously on 2026-10-17; every decision the user might make differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`.

Current state:

- **Lexer** (`crates/prism-lexer/src/lexer.rs`): `+`, `-`, `*`, `/` and `%` are always single tokens, so `x += 1` lexes as `x`, `+`, `=`, `1`. `++` and `--` lex as two tokens: `x++` is `E2001` ("Unexpected token in expression: '+'"), and `a--b` parses as `a - (-b)`. No `.prism` file in the repository (fixtures, examples) contains `++` or `--`.
- **Parser** (`parse_assignment_or_expression`): parses a target at `Precedence::Assign` (or a `*` write-through target), then requires `=` on the same line, rejects a chained `=`, and checks that the target is a variable, `this`, a non-safe field access, an index or a `Deref`. `Eq` has `Precedence::Assign`, and `parse_infix` turns an `=` in expression position into "An assignment is a statement, not an expression". `parse_initializer` rejects `val a = b = c` as chained.
- **Type checker** (`check_assignment`): types the target (binding, field path with `contents_writable`, `Deref` through `write_through`, implicit `this` field), reports `E4003` for non-writable targets, re-routes reference bindings to `check_repoint` (D6 of `reference-access`), and checks the value in a consuming position. Arithmetic (`infer_binary_op`) infers both operands with `infer_operand`, which peels references and reads places in place, and accepts `Int`/`Int`, `Float`/`Float` and, for `+`, `String`/`String`; anything else is `E4002` `TypeMismatch { expected: left, found: right }`.
- **Typed tree**: `StatementKind::Assign { target, value }`; the validator requires the target to be a place and the value to have its type. Places appear once (single-parent rule).
- **MIR** (`lower_assign`): evaluates the value first, then the target place, drops the old value when the type needs a drop, then writes; a binding with no drop is written directly, and the write is relocated to the target's span (D6 of `borrowck-exclusivity`). `lower_binary` reads `Copy` operands through `ordered_operands`, which stabilizes an operand into a temporary when a later operand may write (`n + inc(&mut n)`), and borrows non-`Copy` operands (`comparison_operand`) with origin `Operator`.
- **Borrow checker**: works on MIR only. `x = x + 1` while `x` is mutably borrowed reports both `E5004` (the write) and `E5003` (the read), at different spans. Of several accesses at one span that conflict with one loan, only the last is reported (`conflicts.rs`).
- **ABI**: `hash_statement_span_free` hashes every statement kind of a body (used for bodies whose text is part of an ABI).

## Goals / Non-Goals

**Goals:**
- `t op= v` means exactly `t = t op v` with `t`'s place evaluated once, so every later stage (borrow checker, backend) gets its meaning from rules that already exist.
- No new MIR construct and no borrow-checker change.
- A fixture for every new scenario, and typed-tree and MIR snapshots for the lowering.

**Non-Goals:**
- Bitwise operators and their compound forms (D9).
- User-defined operators (`plus`, `plusAssign`), which need traits.
- `a?.b += 1`, rejected like `a?.b = 1` (an open ROADMAP item already covers safe-navigation assignment).
- An in-place `String` append: the semantics leave the backend free to implement `s += t` as an append, since nothing can observe the intermediate string (D3).

## Decisions

### D1. Compound assignment is a statement with the targets of `=`

`+=`, `-=`, `*=`, `/=` and `%=` are single tokens (`PlusEqual`, `MinusEqual`, `StarEqual`, `SlashEqual`, `PercentEqual`; spelled out per the naming rule, although older tokens are `BangEq` and `LtEq`). `parse_assignment_or_expression` accepts any of them where it accepts `=`: same target check, same "on the same line" rule, the same chained check after the value. They get `Precedence::Assign` in `current_precedence`, so in expression position (`(x += 1)`, `g(x += 1)`) `parse_infix` reports "An assignment is a statement", and `parse_initializer` reports `val y = x += 1`. The AST gets `StatementKind::CompoundAssign { operator: BinaryOp, target, value }`, whose operator is one of the five arithmetic ones. This follows "Assignment Is a Statement": a compound assignment is an assignment.

A separate AST variant rather than `Assign { operator: Option<BinaryOp>, … }`: every consumer must decide what a compound assignment means (the type checker, the formatter, the ABI body hash), and a variant makes that a compile error instead of a silent plain assignment.

### D2. `t op= v` evaluates the place, reads it, then evaluates the value (autonomous)

The order is: evaluate the target's place (a call under `*`, an index), read its current value, evaluate `v`, apply the operator, drop the old value if needed, write. This is Kotlin's and Java's order (`a[i] += f()` reads `a[i]` before calling `f`) and it is what `t = t op v` already does under "Operands Are Evaluated in Order": `n += inc(&mut n)` adds the old `n`, exactly like `n = n + inc(&mut n)`.

Rejected: Rust's order for primitives, which evaluates the value first and the place last (`n += inc(&mut n)` adds the new `n`), and uses the opposite order for overloaded operators; one order for every type is simpler to explain and to lower. Rejected: desugaring to `t = t op v` with the target duplicated, which evaluates a call under `*` or an index twice.

Plain `=` keeps evaluating its value before its place (`lower_assign`); for `=` the old value is never read, so the two orders cannot be confused.

### D3. Operators and operands follow the binary operators (autonomous)

The operator applies to the target's type `T` and the value's type exactly as the binary operator does: `Int` with `Int`, `Float` with `Float`, `String` with `String` for `+=`. The result then always has type `T`, so no extra assignability check is needed. The value is inferred as an operator operand (`infer_operand`): references are peeled (`n += k` for `k: &Int`), and a place is read in place, never consumed, so `s += t` leaves `t` usable. The target is not peeled: a binding of reference type is D7's case.

`String += String` is accepted, as in Kotlin (`s += "x"` on a `var`). It means `s = s + t`: a new string from borrows of both, the old one dropped. An append in place would be observably identical, so the backend may implement it that way later without a language change.

Rejected: consuming the value (Rust's `String += String` does not exist; `AddAssign<&str>` borrows); rejecting `String +=` until an append exists in the runtime, which would make Kotlin's most common string idiom an error for a backend detail.

### D4. A dedicated typed-tree statement (autonomous)

`StatementKind::CompoundAssign { operator: BinaryOperator, target, value }`: the target is a place read in place and written, the value an operand of the target's type. The validator checks that the target is a place, the operator is arithmetic, and the value has the target's type.

Rejected: desugaring in the type checker to `Assign { target, value: Binary(op, target', v) }`. The single-parent rule forbids sharing `target`, and a copy of it would evaluate its calls and indices twice; a new "same place as the target" node would be a special case that every consumer must understand anyway. A dedicated node also keeps the operator visible to the formatter, the LSP and a backend that wants to append in place.

### D5. MIR lowers it to existing statements (autonomous)

`lower_compound_assign` evaluates the target's place once (`place`), then:

- **`Copy` type** (`Int`, `Float`): the left operand is `copy place`, stabilized into a temporary when the value may write (`may_write`), as `ordered_operands` does. The right operand is the value's operand. Then `place = op(left, right)`, written at the target's span: `_1 = copy _1 + const 2`.
- **Other types** (`String`): a shared `Operator` borrow of the place, then the value's borrow (`comparison_operand`), then `tmp = op(borrow, borrow)`, a drop of the place, and `place = move tmp` at the target's span.

The whole statement runs in its own temporary scope, like `Assign`. The borrow checker then sees a read of the place followed by a write, with no new rule: a live borrow makes the write `E5004` (D6), a closure's write to a capture makes it mutable, and a moved `String` target is `E5001` at its borrow.

Rejected: a MIR `CompoundAssign` statement, which every borrow-checker analysis (uses, conflicts, moves, captures, elaboration) and the backend would have to learn. rustc's MIR also lowers primitive compound assignment to `_1 = Add(copy _1, …)`.

### D6. One conflict per compound assignment (autonomous)

With a mutable borrow of `x` live, `x += 1` both reads and writes `x` at the same span, which could report `E5003` for the read and `E5004` for the write, one mistake twice at one caret. The borrow checker already reports, of the accesses at one span that conflict with one loan, only the last (the rule written for optional reborrows in `borrowck-elision`), so it reports the write's `E5004` alone, "cannot assign to 'x' because it is borrowed", whether the borrow is shared or mutable. `x = x + 1` keeps reporting both, since its read and its write are at different spans and the user wrote both. Nothing changes in `prism-borrowck`; fixtures pin the behaviour.

Rejected: putting the read at the operator's span so both are reported, and reporting the read's `E5003` instead, which for a shared borrow would not apply (a read does not conflict with a shared borrow) and so would make the code depend on the borrow's kind.

### D7. Diagnostics reuse existing codes

- Non-writable targets: the target typing is shared with `check_assignment`, so a `val` binding, a `val` field, a field of a non-mutable value, a write through `&` and an invalid target give the same `E4003` messages as `=`.
- An operator that does not apply: `E4002`, as for the binary operator, but through a new error kind, `OperatorMismatch { operator, left, right }`, whose message is "operator '-=' cannot be applied to 'String' and 'String'" instead of `TypeMismatch`'s "expected 'String', found 'String'", with help naming the accepted operand types (`'-=' needs two 'Int' or two 'Float' operands`; `+=` also two `String`s). This is not a new kind of error, which would need a new code: `E4002` already covers "operator operand" mismatches, and the new variant only gives that kind a message that fits operators. The binary operators keep `TypeMismatch` in this change; moving them to `OperatorMismatch` is a follow-up (it changes existing messages only).
- A reference binding as target (`r += 1`): rejected like `r = 5` (D6 of `reference-access`): `E4003` alone for a `val` binding, `E4002` for a `var`, with help "to write through the reference, use '*r += 1'" for `&mut`, or that a shared borrow cannot be written through. The value is still checked as an operand, so its own errors are reported.

No new code, so `diagnostics/spec.md` only widens the `E2001`, `E4002` and `E4003` rows.

### D8. `++` and `--` are reserved and rejected (autonomous)

The lexer reads `++` and `--` as tokens (as Kotlin's lexer does) and the parser rejects them in prefix and postfix position with `E2001`: "Prism has no '++' operator; use 'x += 1'". As postfix operators they get `Precedence::Postfix`, so `x++` reaches `parse_infix` and its message; after a newline, `++x` starts a statement and reaches `parse_prefix`.

Rejected: Kotlin's `++`/`--` expressions, whose value (old or new, by position) makes an assignment an expression, against "Assignment Is a Statement"; statement-only `x++`, a half feature that still invites `val y = x++` (Swift removed both operators for these reasons, SE-0004); and leaving the lexer alone, which keeps `a--b` meaning `a - (-b)`, a reading Kotlin does not have, and gives `x++` a confusing message. Reserving the tokens keeps adding the operators later a non-breaking change. **BREAKING:** `a--b` and `--x` need a space or parentheses.

### D9. Bitwise operators are deferred (autonomous)

The `lexer-parser` item grouped bitwise operators with compound assignment, but their spelling is a separate decision with real ambiguity costs: `&` is the prefix borrow and `&&` is split into two borrows (`&&x`), so a binary `&` would make `a & &b` and `a&&b` hinge on spacing; `|` is a lexer error today; `>>` would have to be split back into two `>` wherever a nested type-argument list closes (`Box<Box<Int>>`), as `>=` already is. Kotlin avoids all of it with named infix functions (`and`, `or`, `xor`, `shl`, `shr`, `ushr`, `inv()`), which Prism could adopt as soft keywords or through `infix` functions. The choice also depends on sized integer types (the ROADMAP item on built-in type names mentions `UInt64`), since shifts and masks are mostly used on them. A `bitwise-operators` change can decide both; its compound forms (`&=` or `and=`) then extend D1's token set. Nothing in this change constrains it.

### D10. Spec placement (autonomous)

No new capability: the grammar goes to `syntax`, the typing to `type-system` (next to the operator rules), the tree shape to `typed-tree`, the lowering to `mir` and the borrow-checker view to `references`, where conflicts and captures live. A `compound-assignment` capability was rejected because it would split each of these subjects across two specs.

## Risks / Trade-offs

- [`String += String` allocates a new string each time, so `s += x` in a loop is quadratic until the backend appends in place] → the semantics already allow an in-place append (D3); `runtime-strings-and-drops` can implement it, and is listed as the follow-up.
- [Breaking `a--b`] → no fixture or example uses it; the diagnostic says how to write it.
- [D6 reports a compound assignment under a mutable borrow as an assignment, not as a use] → both conflicts are with the same borrow, and the fix (end the borrow before the statement) is the same.
- [Compound assignment and plain assignment evaluate in different orders (D2)] → only observable when the place itself has side effects (a call under `*`, an index) and the value writes what the place reads; the MIR snapshot fixtures pin both orders.

## Migration Plan

No external users. In-repo, no fixture or example uses `++`, `--` or a pattern that the new tokens change. Rollback is a revert.

## Open Questions

- The exact wording of the help lines can be tuned during implementation.
