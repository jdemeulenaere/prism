# Execution Specification Delta

## ADDED Requirements

### Requirement: Templates Give the Text of Their Parts
A string template MUST evaluate to a new string holding, in order, its text runs and the text of each part: a `String` part's text, the text that `print` writes for a scalar part, and the result of `toString()` for a part of another type. A template MUST leave its parts unchanged.

#### Scenario: Names and expressions in templates
- **WHEN** `fun main() { val count = 3; val name = "Ada"; println("Count: $count"); println("${count * 2} items for $name's team"); println("$name$count"); println("${if (count > 1) "many ($count)" else "one"}") }` runs
- **THEN** it prints `Count: 3`, `6 items for Ada's team`, `Ada3` and `many (3)`

#### Scenario: Scalars in templates print as print does
- **WHEN** `fun main() { val n = -42; val d = 1.0E7; println("$n|$d|${0.1 + 0.2}|${n > 0}|${'é'}|${4.9E-324}") }` runs
- **THEN** it prints `-42|1.0E7|0.30000000000000004|false|é|4.9E-324`

#### Scenario: Dollar signs that start no part
- **WHEN** `fun main() { val count = 2; println("\$count costs $5, $ 6 or ${count}$") }` runs
- **THEN** it prints `$count costs $5, $ 6 or 2$`

#### Scenario: Nested templates and braces
- **WHEN** `fun describe(n: Int): String = when (n) { 0 -> "none"; 1 -> "one {item}"; else -> "${n} ${if (n > 9) "many" else "few"} items" }` is declared and `fun main() { println("[${describe(0)}] [${describe(1)}] [${describe(12)}]") }` runs
- **THEN** it prints `[none] [one {item}] [12 many items]`

#### Scenario: Templates with toString extensions
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }`, `struct User { val name: String, val home: Point }`, `fun Point.toString(): String = "($x, $y)"` and `fun User.toString(): String = "$name at $home"` are declared and `fun main() { val u = User("Ada" + "!", Point(1, 2)); println("user: $u"); println(u.name) }` runs
- **THEN** it prints `user: Ada! at (1, 2)` and then `Ada!`

### Requirement: Template Parts Are Evaluated Once, Left to Right
The parts of a template MUST be evaluated left to right, each once, and every temporary a template makes MUST be freed by the end of the enclosing statement.

#### Scenario: Template parts are evaluated left to right
- **WHEN** `fun bump(r: &mut Int): Int { *r = r + 1; return r }` is declared and `fun main() { var n = 1; println("$n ${bump(&mut n)} $n") }` runs
- **THEN** it prints `1 2 2`

#### Scenario: Templates free their temporaries and results
- **WHEN** `fun main() { var i = 0; var all = ""; while (i < 3) { val line = "line ${i.toString() + "."} of ${"" + "3"}"; println(line); all = "$all$line;"; i += 1 }; println(all.length) }` runs with `--check-allocations`
- **THEN** it prints `line 0. of 3`, `line 1. of 3`, `line 2. of 3` and `39`, writes nothing to stderr and exits with status 0
