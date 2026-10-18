# Execution Specification Delta

## MODIFIED Requirements

### Requirement: Program Arguments and Options
Arguments after `--` MUST be passed to the program and never interpreted by `prism`; no built-in reads them yet. The `--timings` option, before the file, MUST print to stderr the time spent in each stage (front end, code generation, linking) and the total time from the start of `prism` until `main` is called, before `main` runs. `--no-cache` and `--explain-cache`, before the file, MUST act as the `build-cache` spec states.

#### Scenario: Arguments after the separator
- **WHEN** `prism run main.prism -- --timings x` runs on `fun main() { println(1) }`
- **THEN** it prints `1` to stdout, nothing to stderr, and exits with status 0

#### Scenario: Reporting timings
- **WHEN** `prism run --timings main.prism` runs on `fun main() { println(1) }`
- **THEN** stderr holds one line per stage and one line with the total time to `main`, each in milliseconds, and stdout holds `1`

#### Scenario: Cache options before the file
- **WHEN** `prism run --no-cache --explain-cache main.prism` runs on `fun main() { println(1) }`
- **THEN** stderr holds `Cache: off` and `  checked main.prism`, stdout holds `1`, and it exits with status 0

## ADDED Requirements

### Requirement: A Run From Cached Code Behaves as a Cold Run
A program whose units and instances are loaded from the build cache MUST behave exactly as the same program compiled afresh: the same output, the same exit status, the same runtime errors at the same source locations, and the same result of the allocation check.

#### Scenario: A runtime error in a cached instance
- **WHEN** `lib/math.prism` declares `public fun <T> divide(x: T, n: Int): Int = 10 / n`, `app/main.prism` imports `/lib/math` and declares `fun main() { println(divide("a", 0)) }`, and `prism run app/main.prism` runs twice with the cache enabled
- **THEN** both runs print the same `panic: division by zero` diagnostic located in `lib/math.prism` at `10 / n` to stderr and exit with status 101

#### Scenario: Every executable fixture runs warm as cold
- **WHEN** each executable fixture is run with `prism run --check-allocations` on an empty cache and then again on the cache that run filled
- **THEN** the second run matches the same expected output and status, and writes no new entry
