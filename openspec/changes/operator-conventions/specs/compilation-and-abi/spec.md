## ADDED Requirements

### Requirement: Operator Functions and Equality Operators in the ABI
Phase 1 MUST record whether an exported function is `operator`, and, in the record of an exported struct or enum and in its definition among the reachable types, whether it has an `equals` operator; the ABI hash and the structural hash of definitions MUST cover both. Editing an operator function's body MUST change neither.

#### Scenario: Marking a function operator changes the hash
- **WHEN** `public fun Vector2.plus(other: Vector2): Vector2` in `geometry/vector.prism` becomes `public operator fun Vector2.plus(other: Vector2): Vector2`
- **THEN** the exported ABI hash of `geometry/vector` changes, and importers are re-checked

#### Scenario: Adding an equality operator changes the type's definition
- **WHEN** `models/user.prism`, which exports `public struct User { val id: Int, val name: String }`, gains `public operator fun User.equals(other: &User): Bool = id == other.id`
- **THEN** its ABI records that `User` has an `equals` operator, its ABI hash changes, and the instance key of any instance whose type arguments reach `User` (`List<User>`'s glue) changes

#### Scenario: Editing an operator body recompiles nothing downstream
- **WHEN** the body of the exported `equals` or `plus` of `models/user` changes
- **THEN** the exported ABI hash is unchanged, so no importer is re-checked
