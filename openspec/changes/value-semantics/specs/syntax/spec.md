# Spec Delta

## ADDED Requirements

### Requirement: Copy and Clone Markers on Struct and Enum Declarations
A struct or enum declaration MUST accept an optional marker, `: Copy` or `: Clone`, in Kotlin supertype position: after the name and type parameters, after a shorthand struct's parameter list, and before the body. Exactly one of the names `Copy` and `Clone` MUST follow the colon; anything else MUST be rejected with `E2001`. Both names stay ordinary identifiers everywhere else.

#### Scenario: Markers on a block struct and an enum
- **WHEN** parsing `struct Point : Copy { val x: Int, val y: Int }`, `struct User : Clone { val name: String }` and `enum Color : Copy { Red, Green }`
- **THEN** the declarations parse, marked `Copy`, `Clone` and `Copy` respectively

#### Scenario: Marker after type parameters and a shorthand parameter list
- **WHEN** parsing `struct Tagged<T> : Copy { val id: Int }` and `struct Name(val v: String) : Clone`
- **THEN** both declarations parse with their markers

#### Scenario: Declaration without a marker
- **WHEN** parsing `struct Point { val x: Int }`
- **THEN** the declaration parses with no marker

#### Scenario: Other names after the colon rejected
- **WHEN** parsing `struct P : Eq { val x: Int }`, `struct P : Copy, Clone { val x: Int }`, `struct P : Copy, Copy { val x: Int }` or `struct P : { val x: Int }`
- **THEN** the parser reports `E2001`, saying that exactly one of `Copy` and `Clone` may follow `:` in a struct or enum declaration, and that `Copy` already implies `Clone`

#### Scenario: Marker in the wrong place rejected
- **WHEN** parsing `struct P : Copy <T> { val x: T }` or `struct Id : Copy (val v: Int)`
- **THEN** the parser reports `E2001`

#### Scenario: Marker names remain identifiers
- **WHEN** parsing `fun f(Copy: Int, Clone: Int): Int = Copy + Clone` and `struct Clone { val x: Int }`
- **THEN** both declarations parse

## MODIFIED Requirements

### Requirement: Variable Bindings and Immutability
Variables MUST be immutable by default using `val`, while mutable variables MUST be explicitly declared with `var`. `var` declares a mutable local only: a top-level declaration is always a `val` (see the [`ownership`](../ownership/spec.md) spec, "No Mutable Top-Level State").

#### Scenario: Reassigning immutable val binding
- **WHEN** code attempts to reassign a variable declared with `val` (`val x = 10; x = 20`)
- **THEN** the compiler MUST reject the reassignment with a compile error

#### Scenario: Reassigning mutable var binding
- **WHEN** code reassigns a variable declared with `var` (`var x = 10; x = 20`)
- **THEN** the mutation is valid
