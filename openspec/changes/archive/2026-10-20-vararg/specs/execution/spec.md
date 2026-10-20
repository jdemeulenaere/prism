## ADDED Requirements

### Requirement: Vararg Calls Run
A call MUST pass a vararg the list of its elements, evaluated left to right in their place among the call's arguments and moved or copied in, or the spread's list itself; the callee MUST own that list, and every element MUST be freed exactly once, by the callee or by whoever it gives the list or an element to.

#### Scenario: Summing elements and a spread list
- **WHEN** `fun sum(vararg xs: Int): Int { var total = 0; for (x in xs) total += x; return total }` is declared and `fun main() { val more = listOf(4, 5); println(sum()); println(sum(1, 2, 3)); println(sum(*more)) }` runs with `--check-allocations`
- **THEN** it prints `0`, `6` and `9`, each on its own line, reports no live allocation and exits with status 0

#### Scenario: Strings moved into a vararg are freed once
- **WHEN** `fun shout(vararg words: String, end: String = "!") { for (w in words) print(w); println(end) }` is declared and `fun main() { val a = "x".clone(); shout(a, "y" + "z", end = "?"); shout() }` runs with `--check-allocations`
- **THEN** it prints `xyz?` and `!`, each on its own line, frees every string exactly once and exits with status 0

#### Scenario: Taking an element out of a vararg
- **WHEN** `fun <T> lastOf(vararg xs: T): T { var list = xs; return list.removeLast() }` is declared and `fun main() { println(lastOf("a".clone(), "b".clone())); println(lastOf(1, 2, 3)) }` runs with `--check-allocations`
- **THEN** it prints `b` and `3`, frees `a` with its list and `b` after printing it, and exits with status 0

#### Scenario: Closures as vararg elements
- **WHEN** `fun runAll(vararg actions: () -> Unit) { for (a in actions) a() }` is declared and `fun main() { val n = 7; val s = "s".clone(); runAll({ println(n) }, { println(s) }) }` runs with `--check-allocations`
- **THEN** it prints `7` and `s`, each on its own line, frees the closure owning `s` exactly once and exits with status 0

#### Scenario: Vararg arguments run in source order
- **WHEN** `fun tag(n: Int): Int { println(n); return n }` and `fun f(a: Int, vararg xs: Int, last: Int = 0): Int = a * 100 + xs.size * 10 + last` are declared and `fun main() { println(f(tag(1), tag(2), tag(3), last = tag(4))); println(f(last = tag(5), tag(6), tag(7))) }` runs
- **THEN** it prints `1`, `2`, `3`, `4`, `124`, `5`, `6`, `7` and `615`, each on its own line
