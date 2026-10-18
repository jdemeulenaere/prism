# Formatting Delta

## ADDED Requirements

### Requirement: Library Declarations Are Printed
`prism fmt` MUST print `external` and `intrinsic` declarations, and intrinsic extension properties, with their modifiers in the canonical order (visibility, `external` or `intrinsic`, `compose`, receiver mode) and no body, each on its own line, and MUST break their parameter lists as any function's.

#### Scenario: Library declarations formatted
- **WHEN** `prism fmt -` reads `public   intrinsic  mut fun <T> List<T>.add( element :T )` and `public intrinsic val <T> List<T>.size :Int`
- **THEN** it writes `public intrinsic mut fun <T> List<T>.add(element: T)` and `public intrinsic val <T> List<T>.size: Int`, each on its own line
