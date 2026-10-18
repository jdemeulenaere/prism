# Code Generation Specification Delta

## ADDED Requirements

### Requirement: Compiled Units Are Relocatable Artifacts
The backend MUST compile each file's and each instance's code unit into an artifact of machine code, data objects and relocations that name their targets by symbol, and MUST define every unit of a load from its artifact, whether it was just compiled or read from the build cache, so that cached code is linked exactly as fresh code is.

#### Scenario: Cached units run as fresh ones
- **WHEN** a program whose files and instances hold string literals, top-level values, drop glue and calls between files is run twice with `prism run --check-allocations` and the cache enabled
- **THEN** the second run loads every unit from the cache, and both runs print the same output, report no leak and exit with status 0

#### Scenario: Dumps never read the cache
- **WHEN** `prism dump-clif app/main.prism` runs before and after `prism run app/main.prism` fills the cache
- **THEN** it prints the same text both times

## MODIFIED Requirements

### Requirement: Tier 1 Compiles Elaborated MIR in Memory
`prism run` MUST compile the elaborated MIR of every file of the program into native code in the memory of the `prism` process, or load that code from the build cache (the `build-cache` spec), without invoking any external program such as a linker or an assembler. Every body MUST be compiled, closure bodies included, except generic bodies and the closures inside them, which are compiled only as the instances the program calls.

#### Scenario: Running a scalar program
- **WHEN** `fun square(x: Int): Int = x * x` and `fun main() { println(square(7)) }` are declared in `main.prism` and `prism run main.prism` runs
- **THEN** it prints `49` and exits with status 0, and no other process is started

#### Scenario: An uncalled generic function is not compiled
- **WHEN** `fun <T> keep(x: T): T = x` and `fun main() { println(1) }` are declared and `prism run main.prism` runs
- **THEN** it prints `1` and exits with status 0

#### Scenario: A called generic function is compiled per instance
- **WHEN** `fun <T> keep(x: T): T = x` and `fun main() { println(keep(1)); println(keep(true)) }` are declared and `prism run main.prism` runs
- **THEN** it prints `1` and `true` and exits with status 0

#### Scenario: A closure is compiled
- **WHEN** `fun twice(f: (Int) -> Int, x: Int): Int = f(f(x))` and `fun main() { println(twice({ n -> n * 3 }, 2)) }` are declared and `prism run main.prism` runs
- **THEN** it prints `18` and exits with status 0

#### Scenario: A warm run starts no compilation
- **WHEN** `prism run main.prism` runs twice, with the cache enabled, on the program that calls `keep(1)` and `keep(true)`
- **THEN** the second run prints `1` and `true` and compiles no function, every unit and instance being loaded from the cache
