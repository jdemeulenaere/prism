# Compilation and ABI Specification Delta

## ADDED Requirements

### Requirement: Extension Functions in the ABI
Phase 1 MUST record an exported extension as a function signature together with its receiver mode and its receiver type, bound like any signature type, and the ABI hash MUST cover both. Struct and enum records MUST NOT depend on extensions. The order of exported symbols MUST take the receiver into account, so that reordering declarations never changes the hash.

#### Scenario: Exported extension recorded with its receiver
- **WHEN** `models/user.prism` declares `public struct User { var name: String }` and `public mut fun User.rename(n: String): Unit`
- **THEN** its ABI records a function `rename` with the parameter `n: String`, the return type `Unit`, the receiver type `User` from module `models/user` and the mutable receiver mode

#### Scenario: Adding an extension in another file
- **WHEN** `ui/ui.prism`, which imports `/models/user`, gains `public fun User.shout(): String`
- **THEN** the ABI hash of `models/user` is unchanged, and the ABI hash of `ui/ui` changes

#### Scenario: Adding an extension in the receiver type's file
- **WHEN** `models/user.prism` gains `public fun User.shout(): String`
- **THEN** its ABI hash changes, while its record of `User` is unchanged

#### Scenario: Changing a receiver mode or type changes the ABI hash
- **WHEN** `public fun User.reset(): Unit` becomes `public mut fun User.reset(): Unit`, or `public fun User.size(): Int` becomes `public fun Team.size(): Int`
- **THEN** the exported ABI hash changes

#### Scenario: Extensions sharing a name
- **WHEN** a file exports `public fun describe(): String`, `public fun User.describe(): String` and `public fun Team.describe(): String`, and the three declarations are reordered
- **THEN** the ABI records three distinct functions, and the exported ABI hash is the same in every order

#### Scenario: Extension body or private extension edits
- **WHEN** the body of an exported non-generic extension changes, or a private extension is added, removed or modified
- **THEN** the exported ABI hash is unchanged
