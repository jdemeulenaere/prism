# Syntax Delta

## ADDED Requirements

### Requirement: Intrinsic Writable Properties
`intrinsic var`, optionally after a visibility modifier, MUST declare an extension property as `intrinsic val` does, with an optional type parameter list, a receiver type, `.`, the name, `:` and its type, and no initializer. A `var` declaration with a receiver and without `intrinsic`, and an `intrinsic var` with an initializer, MUST be rejected with `E2001`.

#### Scenario: A writable intrinsic property parses
- **WHEN** parsing `public intrinsic var <T> MutableState<T>.value: T`
- **THEN** it is an intrinsic extension property marked writable, with type parameter `T`, receiver `MutableState<T>`, name `value` and type `T`

#### Scenario: Writable extension properties need intrinsic
- **WHEN** parsing `var Int.twice: Int` or `intrinsic var Int.twice: Int = 2`
- **THEN** the parser reports `E2001`
