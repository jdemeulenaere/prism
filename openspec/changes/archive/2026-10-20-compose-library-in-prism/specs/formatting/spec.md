## ADDED Requirements

### Requirement: Annotations Are Printed
The formatter MUST print each annotation of a declaration on its own line, in the order written, directly above the declaration and at its indentation, keeping any comment above the first annotation.

#### Scenario: Annotations formatted
- **WHEN** formatting a library module holding `@Keeper public struct C(private val handle: Int)` and `@DrivesComposition   @ExplicitGroupsComposable public fun f(): Unit {}`
- **THEN** the results are `@Keeper\npublic struct C(private val handle: Int)` and `@DrivesComposition\n@ExplicitGroupsComposable\npublic fun f(): Unit {}`
