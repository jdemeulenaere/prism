# Typed Tree Specification Delta

## ADDED Requirements

### Requirement: Compound Assignments Are Statements Over Places
A compound assignment MUST be a statement recording its arithmetic operator, its target and its value. The target MUST be a place that appears once, read in place and then written; the value MUST be an operator operand of the target's type, read in place when it is a place and never consumed.

#### Scenario: Compound assignment of a local
- **WHEN** a body of `fun f(k: &Int)` runs `var n = 1; n += k`
- **THEN** the second statement is a compound assignment `+` whose target is the binding `n` and whose value is the dereference of the binding `k`, read in place

#### Scenario: Compound assignment through a reference
- **WHEN** `struct Counter { var count: Int }` is declared and `fun f(c: &mut Counter) { c.count *= 2 }` is checked
- **THEN** the statement is a compound assignment `*` whose target is field 0 of the dereference of the binding `c`, and whose value is the literal `2`

#### Scenario: String compound assignment reads its value in place
- **WHEN** `fun f(t: String) { var s = "a"; s += t }` is checked
- **THEN** the compound assignment `+` has the binding `s` as its target and the binding `t` itself as its value, neither moved nor copied
