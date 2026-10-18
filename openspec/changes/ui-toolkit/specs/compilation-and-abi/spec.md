# Compilation and ABI Delta

## ADDED Requirements

### Requirement: Companion Values in the ABI
A public struct and its public companion value MUST both be exported, as two symbols with one name: the struct with its definition and the value with its declared type. Adding, removing or retyping the companion value MUST change the file's ABI hash.

#### Scenario: A struct and its companion value are exported
- **WHEN** `prism abi` runs on a file declaring `public struct Gap : Copy { val size: Int }`, `public struct Gaps : Copy { val None: Gap }` and `public val Gap: Gaps = Gaps(Gap(0))`
- **THEN** it lists both `public struct Gap : Copy` and `public val Gap: Gaps`

#### Scenario: Adding a companion value changes the ABI hash
- **WHEN** `public val Gap: Gaps = Gaps(Gap(0))` is added to a file that declares `public struct Gap` and `public struct Gaps`
- **THEN** the file's ABI hash changes
