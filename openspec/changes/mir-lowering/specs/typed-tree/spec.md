# Typed Tree Specification Delta

## ADDED Requirements

### Requirement: The Typed Tree Records Its File's Types
The typed tree MUST record every struct and enum known to its file (its own declarations, those its direct imports export, and the built-in `Range`) with its `Copy` or `Clone` marker and, when the file knows them, its fields or variants in declaration order. The text dump MUST list the imported ones.

#### Scenario: Imported types are recorded
- **WHEN** `models/user.prism` declares `public struct User : Clone { val name: String }` and a file importing `/models/user` declares `fun f(u: &User): Int = 1`
- **THEN** the file's typed tree records `models/user::User` as a `Clone` struct whose field 0 is `name` of type `String`, and `prism dump-typed-tree` lists it as an imported struct
