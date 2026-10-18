# MIR Specification Delta

## ADDED Requirements

### Requirement: Compound Assignments Read Then Write Their Target
A compound assignment MUST evaluate its target's place once, then read the place's current value (a copy for a `Copy` type, a shared operator borrow otherwise), then evaluate the value, then apply the operator. A `Copy` result MUST be written to the place directly; any other result MUST be computed into a temporary, after which the old value is dropped and the temporary moved in. Both the read and the write MUST be at the target's span.

#### Scenario: Compound assignment of an integer local
- **WHEN** `fun f(): Int { var n = 1; n += 2; return n }` is lowered
- **THEN** `n` is assigned the sum of a copy of `n` and the constant `2`, with no temporary, at the span of `n`

#### Scenario: The target is read before the value is evaluated
- **WHEN** `fun inc(r: &mut Int): Int { *r += 1; return r }` is declared and `fun f(): Int { var n = 1; n += inc(&mut n); return n }` is lowered
- **THEN** `n` is copied into a temporary before `inc` is called, and `n` is then assigned the sum of that temporary and the call's result

#### Scenario: The target's place is evaluated once
- **WHEN** `fun pick(a: &mut Int, b: &mut Int): &mut Int = a` is declared and `fun f() { var a = 1; var b = 2; *pick(&mut a, &mut b) += 1 }` is lowered
- **THEN** `pick` is called once, and the dereference of its result is both read and written

#### Scenario: String compound assignment drops the old value
- **WHEN** `fun f(t: String): String { var s = "a"; s += t; return s }` is lowered
- **THEN** a temporary is assigned the sum of a shared operator borrow of `s` and a shared operator borrow of `t`, then `s` is dropped and assigned a move of the temporary, and `t` is not moved
