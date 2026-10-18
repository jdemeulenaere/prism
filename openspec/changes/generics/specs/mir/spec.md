# MIR Specification Delta

## ADDED Requirements

### Requirement: Generic Bodies Are Lowered Once
A generic declaration's body, and its default values and closures, MUST be lowered once, recording the declaration's type parameters, with locals typed by them. A call of a generic callee MUST record its type arguments and the callee's declared parameter types. A value of a type parameter that is not `Copy` MUST be dropped like any value that needs a drop, and one bounded by `Copy` MUST NOT be dropped.

#### Scenario: Dropping a type-parameter value
- **WHEN** `fun <T> discard(x: T) {}` and `fun <T : Copy> keep(x: T): Int = 1` are lowered
- **THEN** the body of `discard` records the type parameter `T` and drops its argument when it returns, and the body of `keep` has no drop

#### Scenario: A call with type arguments
- **WHEN** `fun <T> id(x: T): T = x` is declared and `fun f(s: String): String = id(s)` is lowered
- **THEN** the MIR of `f` calls `main::id` with the type argument `String`, the declared parameter type `T` and a move of `s`, and `prism dump-mir` prints the callee as `main::id<String>`

#### Scenario: A default value of a generic function
- **WHEN** `fun <T : Copy> f(x: T, y: T = x): T = y` is declared and `fun g(): Int = f(1)` is lowered
- **THEN** the generator of `y` records the type parameter `T : Copy`, and the MIR of `g` calls it with the type argument `Int`, which `prism dump-mir` prints as `default main::f<Int> y`

#### Scenario: Elaborated drops of a generic body
- **WHEN** `fun <T> pass(x: T): T = x` and `fun <T> pick(a: T, b: T, c: Bool): T = if (c) a else b` are lowered and elaborated
- **THEN** `pass` drops nothing, since `x` is moved into the return place, and `pick` drops the argument it did not return, under a drop flag or on each branch
