# Loops Specification Delta

## ADDED Requirements

### Requirement: Loop and Range Evaluation Semantics
A `while` loop MUST evaluate its condition before each iteration and a `do`/`while` loop after each iteration. A `for` loop MUST evaluate its range once and bind the loop variable to each `Int` in it in ascending order, without overflowing at the end of the `Int` range. `break` MUST leave the innermost enclosing loop and `continue` MUST go to its next condition check or element. `a..b` MUST include `b`, and `a..<b` MUST exclude it.

#### Scenario: Half-open range iteration
- **WHEN** `for (i in 0..<3)` runs, and `for (i in 3..<3)` runs
- **THEN** the first loop's body runs with `i` equal to 0, 1 and 2, and the second loop's body never runs

#### Scenario: Closed range iteration
- **WHEN** `for (i in 1..3)` runs
- **THEN** its body runs with `i` equal to 1, 2 and 3

#### Scenario: Range ending at the largest Int
- **WHEN** `for (i in 9223372036854775806..9223372036854775807)` runs
- **THEN** its body runs twice, and no integer overflow is raised

#### Scenario: Do-while body runs once
- **WHEN** `do { n = n + 1 } while (false)` runs with `n` equal to 0
- **THEN** `n` is 1

#### Scenario: While condition checked first
- **WHEN** `while (n < 3) { n = n + 1 }` runs with `n` equal to 5
- **THEN** its body never runs and `n` is still 5

#### Scenario: Continue skips to the next element
- **WHEN** `for (i in 0..<5) { if (i == 2) continue; sum = sum + i }` runs with `sum` equal to 0
- **THEN** `sum` is 8

#### Scenario: Break leaves only the innermost loop
- **WHEN** a `break` runs inside a `while` loop nested in a `for` loop
- **THEN** the `while` loop ends and the `for` loop continues with its next element

#### Scenario: Range bounds evaluated once
- **WHEN** `for (i in 0..<next())` runs, where `next()` returns 3
- **THEN** `next()` is called once, before the first iteration
