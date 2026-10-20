## ADDED Requirements

### Requirement: Annotations
A top-level declaration MAY start with annotations, each `@` directly followed by an identifier, with no arguments, on the declaration's line or on lines of their own (`@Keeper public struct Composition { ... }`). Annotations MUST come before `context(...)`, the visibility and every other modifier. Which annotations exist and which files may use them is the rule of `standard-library`.

#### Scenario: Annotations before declarations
- **WHEN** parsing `@Keeper\npublic struct C { private val handle: Int }`, `@DrivesComposition public fun f(): Unit {}` and `@ExplicitGroupsComposable\npublic compose fun S(): Unit {}`
- **THEN** each parses as its declaration carrying the annotation named `Keeper`, `DrivesComposition` and `ExplicitGroupsComposable`, with its visibility and modifiers as written

### Requirement: Malformed Annotations Are Rejected
An `@` not directly followed by an identifier, an annotation with arguments, and an annotation anywhere but before a top-level declaration MUST be rejected with `E2001`.

#### Scenario: Malformed annotations rejected
- **WHEN** parsing `@ Keeper struct A(val n: Int)`, `@1 fun f() {}`, `@Keeper(1) struct B(val n: Int)` and `fun g() { @Keeper val x = 1 }`
- **THEN** the parser reports `E2001` for each, saying that an annotation is `@` followed by a name before a top-level declaration
