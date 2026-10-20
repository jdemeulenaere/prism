## ADDED Requirements

### Requirement: Int and Double Conversions
`n.toDouble()` MUST give the `Double` nearest to the `Int` `n`, rounding to nearest even. `d.toInt()` MUST give the `Double` `d` truncated toward zero, `0` for NaN, `Int`'s maximum for values at or above it, positive infinity included, and `Int`'s minimum for values at or below it. Neither MUST raise a runtime error.

#### Scenario: Converting numbers
- **WHEN** a file importing `/prism/core` runs `fun main() { println(7.toDouble() / 2.0); println((3.9).toInt()); println((-3.9).toInt()); println((0.0 / 0.0).toInt()); println((1.0 / 0.0).toInt()); println((-1.0 / 0.0).toInt()) }` with `--check-allocations`
- **THEN** stdout is `3.5`, `3`, `-3`, `0`, `9223372036854775807` and `-9223372036854775808`, one per line, and the exit status is 0
