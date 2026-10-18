# Execution Delta

## ADDED Requirements

### Requirement: Runtime Errors in Library Code
A runtime error raised by code of a library module MUST name its location by the module path followed by `.prism` (`prism/collections.prism`) and the line and column of the failing operation in the library's source. Runtime errors raised by an intrinsic MUST be located at the call in the calling file.

#### Scenario: An overflow inside a library helper
- **WHEN** `fun main() { val xs = listOf(9223372036854775807, 1); println(xs.sumOf { x -> x }) }` runs
- **THEN** stderr holds the integer overflow runtime error, located `  at prism/collections.prism:<line>:<column>` at the addition in `sumOf`'s body, and the exit status is 101

## MODIFIED Requirements

### Requirement: Output Built-Ins Print Their Argument
`print(x)` MUST write the text of `x` to stdout, `println(x)` the text of `x` followed by a newline, and `println()` a newline. An `Int` MUST be written in decimal with a leading `-` when negative, a `Bool` as `true` or `false`, a `Char` as its UTF-8 encoding and a `String` as its text in UTF-8. Any other value MUST be written as the text its `toString()` returns, which MUST be called once per call of `print` or `println`, after the argument is evaluated, and whose result MUST be dropped after it is written.

#### Scenario: Printing scalars
- **WHEN** `fun main() { print(1); print(true); println('x'); println(-5); println() }` runs
- **THEN** stdout holds `1truex`, a newline, `-5`, a newline, and a newline

#### Scenario: Printing strings
- **WHEN** `fun main() { val s = "Grüße, \u{1F680}"; print(s); println("!"); println(s) }` runs
- **THEN** stdout holds `Grüße, 🚀!`, a newline, `Grüße, 🚀` and a newline

#### Scenario: Printing a value through its toString
- **WHEN** `struct P(val x: Int, val y: Int)`, `fun P.toString(): String = "(" + x.toString() + ", " + y.toString() + ")"` and `fun main() { val p = P(1, 2); println(p); print(p); println("!") }` run with `--check-allocations`
- **THEN** stdout holds `(1, 2)`, a newline, `(1, 2)!` and a newline, and every string `toString()` built is freed
