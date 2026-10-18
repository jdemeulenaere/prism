# Execution Specification Delta

## ADDED Requirements

### Requirement: Closures With an Implicit Parameter Run Like Declared Ones
A closure using its implicit `it` MUST run exactly as the closure `{ it -> ... }`: each call binds `it` to the argument, a reference `it` reads and lends what the caller passed, an owned `it` is dropped when the call ends unless moved, and nested closures each read their own `it`.

#### Scenario: Iterating a list with an implicit parameter
- **WHEN** `fun <T> List<T>.forEach(f: (&T) -> Unit) { for (x in this) f(x) }` is declared and `fun main() { val xs = listOf(1, 2, 3); var total = 0; xs.forEach { total += it }; xs.forEach { println(it * 10) }; println(total) }` runs with `--check-allocations`
- **THEN** it prints `10`, `20`, `30` and then `6`, reports no live allocation and exits with status 0

#### Scenario: A generic higher-order function with an implicit parameter
- **WHEN** `fun <T, R> map(x: T, f: (T) -> R): R = f(x)` is declared and `fun main() { println(map(4) { it * it }); println(map("ab") { it + "!" }); println(map(7) { it.toString() + "?" }) }` runs with `--check-allocations`
- **THEN** it prints `16`, `ab!` and `7?`, reports no live allocation and exits with status 0

#### Scenario: An implicit reference parameter reads and lends its referent
- **WHEN** `struct Todo { val title: String; var done: Bool }`, `fun <T> List<T>.forEach(f: (&T) -> Unit) { for (x in this) f(x) }`, `mut fun <T> List<T>.edit(f: (&mut List<T>) -> Unit) { f(this) }` and `fun shout(s: &String): String = s + "!"` are declared, and `main` builds `var todos = listOf(Todo("a", false))`, runs `todos.edit { it.add(Todo("b", true)) }` and `todos.forEach { println(shout(it.title)); println(it.done) }`
- **THEN** it prints `a!`, `false`, `b!` and `true`, reports no live allocation and exits with status 0

#### Scenario: Nested closures with their own implicit parameters
- **WHEN** `fun <T> List<T>.forEach(f: (&T) -> Unit) { for (x in this) f(x) }` and `fun run(f: () -> Unit) { f() }` are declared and `fun main() { val rows = listOf("ab", "c"); val columns = listOf(1, 2); rows.forEach { val row = it; columns.forEach { println(row + it.toString()) }; run { println(it.length) } } }` runs with `--check-allocations`
- **THEN** it prints `ab1`, `ab2`, `2`, `c1`, `c2` and `1`, reports no live allocation and exits with status 0
