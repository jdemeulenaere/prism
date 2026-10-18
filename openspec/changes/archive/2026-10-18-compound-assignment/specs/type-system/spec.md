# Type System Specification Delta

## ADDED Requirements

### Requirement: Compound Assignment Typing
`t op= v` MUST typecheck as `t = t op v` with `t` written once. The target MUST be writable under the rules for `=` (`E4003` otherwise). The operator MUST apply to the target's type and the value's type under the operator rules: `Int` with `Int` and `Float` with `Float` for every compound operator, and `String` with `String` for `+=` only. Otherwise the type checker MUST report `E4002`, with help naming the operand types the operator accepts.

#### Scenario: Compound assignment of numbers and strings
- **WHEN** a body runs `var n = 1; n += 2; n -= 1; n *= 3; n /= 2; n %= 2`, `var d = 1.5; d += 0.5; d %= 1.0` and `var s = "a"; s += "b"`
- **THEN** the body typechecks successfully

#### Scenario: Compound assignment through fields and references
- **WHEN** `struct Counter { var count: Int }` and `mut fun Counter.bump() { count += 1; this.count *= 2 }` are declared, and `fun f(r: &mut Int, c: &mut Counter, k: &Int) { *r += k; c.count -= 1 }` is checked
- **THEN** the file typechecks successfully, and the value `k` is read through its reference

#### Scenario: Compound assignment to an immutable target rejected
- **WHEN** `struct P { val x: Int; var y: Int }` is declared and a body of `fun f(r: &Int, q: P)` runs `val n = 1; n += 1`, `var p = P(1, 2); p.x += 1`, `q.y += 1` or `*r += 1`
- **THEN** the type checker emits `E4003` for each, as it does for `=`

#### Scenario: Compound operator not applicable rejected
- **WHEN** a body runs `var n = 1; n += "a"`, `var s = "a"; s -= "b"`, `var m: Int? = 1; m += 1`, `var x = 1; x += 1.0` or `var b = true; b += true`
- **THEN** the type checker emits `E4002` for each, with help saying which operand types the operator accepts

#### Scenario: Compound assignment does not consume its value
- **WHEN** `fun take(s: String) {}` is declared and `fun f(t: String) { var s = "a"; s += t; take(t) }` is checked
- **THEN** the file passes `prism check`, because `+=` reads `t` in place

### Requirement: Compound Assignment to a Reference Binding
A compound assignment whose target is a binding of reference type MUST be rejected, because it would re-point the binding to a number or a string: with `E4002` for a `var` binding and `E4003` alone for a `val`, with help suggesting `*r op= v` when the reference is `&mut`, or saying that a shared borrow cannot be written through.

#### Scenario: Compound assignment to a reference binding rejected
- **WHEN** `fun f(r: &mut Int) { r += 1 }` and `fun g(a: Int) { var x = 1; var r: &mut Int = &mut x; r += a }` are checked
- **THEN** the type checker emits `E4003` for `f` and `E4002` for `g`, with help suggesting `*r += 1` and `*r += a`
