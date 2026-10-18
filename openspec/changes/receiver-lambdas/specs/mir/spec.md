# MIR Specification Delta

## ADDED Requirements

### Requirement: Context Arguments Lead the Operands of Calls
A call of a function with a context parameter MUST pass the context argument as its first operand, before the receiver and the other arguments. A lambda with a receiver MUST lower like any closure, its receiver being its first parameter.

#### Scenario: A context call and a receiver lambda in MIR
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct RowScope : Copy {}`, `fun row(f: RowScope.() -> Modifier): Modifier = f(RowScope())` and `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)` are declared and `prism dump-mir` runs on a file adding `fun g(): Modifier = row { Modifier(0.0).weight(1.0) }`
- **THEN** the closure's body is a separate body whose first parameter is `this` of type `&RowScope`, and its call of `main::Modifier.weight#context(RowScope)` passes a shared borrow of `*this` first, then a shared borrow of the `Modifier` temporary, then `1.0`
