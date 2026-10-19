# Syntax Delta

## ADDED Requirements

### Requirement: Intrinsic Struct Declarations
`intrinsic struct`, optionally after a visibility modifier, MUST declare a struct without fields or constructor: its name, an optional type parameter list and optional markers (`public intrinsic struct RecomposeScope : Copy`), and no parenthesized fields and no block. An intrinsic struct with fields or a block, and a struct with both `intrinsic` and `external`, MUST be rejected with `E2001`. Which files may use it is the rule of `standard-library`.

#### Scenario: Intrinsic structs parse
- **WHEN** parsing `public intrinsic struct Composition` and `public intrinsic struct RecomposeScope : Copy`
- **THEN** each parses as an intrinsic struct declaration with its visibility, name and markers

#### Scenario: An intrinsic struct with fields rejected
- **WHEN** parsing `intrinsic struct A(val n: Int)`, `intrinsic struct B { val n: Int }` or `external struct C`
- **THEN** the parser reports `E2001`
