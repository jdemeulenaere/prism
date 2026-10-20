## ADDED Requirements

### Requirement: Converting Between Int and Double
`/prism/core` MUST declare, as public intrinsics, `fun Int.toDouble(): Double` and `fun Double.toInt(): Int`. As members of built-in types, they MUST need no import. Their behavior is that of the execution requirement "Int and Double Conversions".

#### Scenario: Conversions need no import
- **WHEN** a file without imports declares `fun half(n: Int): Double = n.toDouble() / 2.0` and `fun whole(d: Double): Int = d.toInt()`
- **THEN** it typechecks

#### Scenario: Conversions in the ABI of the core module
- **WHEN** `prism abi` prints the ABI of `/prism/core`
- **THEN** it holds `public intrinsic fun Int.toDouble(): Double` and `public intrinsic fun Double.toInt(): Int`
