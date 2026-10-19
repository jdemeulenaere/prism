# Formatting Delta

## ADDED Requirements

### Requirement: Intrinsic Structs Are Printed
`prism fmt` MUST print an intrinsic struct declaration on one line, as its visibility, `intrinsic struct`, its name, its type parameters and its markers, with the spacing of other struct declarations.

#### Scenario: Intrinsic structs formatted
- **WHEN** `prism fmt -` reads `public  intrinsic struct RecomposeScope:Copy`
- **THEN** it writes `public intrinsic struct RecomposeScope : Copy` and a newline
