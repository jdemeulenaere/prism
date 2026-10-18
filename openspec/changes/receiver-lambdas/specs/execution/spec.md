# Execution Specification Delta

## ADDED Requirements

### Requirement: Receiver Lambdas Run as Closures Over Their Receiver
A lambda with a receiver MUST run as a closure whose first parameter is the receiver: each call binds `this` to the receiver passed, by `f(r, a)`, `r.f(a)` or `f(a)`, reads and writes through it as through an extension's `this`, and frees what it owns as any closure does.

#### Scenario: A builder with a mutable receiver
- **WHEN** `struct Builder { var lines: List<String> }`, `mut fun Builder.line(s: String) { lines.add(s) }` and `fun build(content: mut Builder.() -> Unit): List<String> { var b = Builder(listOf()); b.content(); content(&mut b); val (lines) = b; return lines }` are declared and `fun main() { var n = 0; val out = build { n += 1; line("a" + n.toString()); lines.add("b") }; out.forEach { println(it) } }` runs with `--check-allocations`
- **THEN** it prints `a1`, `b`, `a2` and `b`, reports no live allocation and exits with status 0

#### Scenario: Receiver function values called three ways
- **WHEN** `struct Counter { val n: Int }` and `fun Counter.twice(f: Counter.(Int) -> Int): Int = f(1) + f(this, 2)` are declared and `fun main() { val c = Counter(10); val f: Counter.(Int) -> Int = { n + it }; println(c.f(5)); println(f(c, 6)); println(c.twice(f)) }` runs with `--check-allocations`
- **THEN** it prints `15`, `16` and `23`, reports no live allocation and exits with status 0

#### Scenario: An escaping receiver lambda owns its captures
- **WHEN** `struct Scope : Copy { val depth: Int }`, `struct Section { val title: String, val content: Scope.() -> String }` and `fun section(title: String, content: escaping Scope.() -> String): Section = Section(title, content)` are declared and `fun main() { val mark = "!" + "?"; val s = section("intro") { "depth " + depth.toString() + mark }; println(s.title); println(s.content(Scope(2))); println(s.content(Scope(3))) }` runs with `--check-allocations`
- **THEN** it prints `intro`, `depth 2!?` and `depth 3!?`, reports no live allocation and exits with status 0

#### Scenario: A generic function with a receiver lambda
- **WHEN** `struct User { val name: String }` and `fun <T> with(receiver: &T, block: T.() -> String): String = block(receiver)` are declared and `fun main() { val u = User("Ada"); println(with(u) { name + "!" }); println(with(3) { (this * 2).toString() }) }` runs with `--check-allocations`
- **THEN** it prints `Ada!` and `6`, reports no live allocation and exits with status 0

### Requirement: Context Extensions Run the Extension of Their Context
A call of an extension with a context parameter MUST run the extension its context chose, with the candidate as its context parameter.

#### Scenario: A layout DSL with scoped modifiers
- **WHEN** `struct Modifier : Copy { val fraction: Double }`, `struct ColumnScope : Copy { val factor: Double }`, `struct RowScope : Copy {}`, `context(scope: RowScope) fun Modifier.weight(w: Double): Modifier = Modifier(w)`, `context(scope: ColumnScope) fun Modifier.weight(w: Double): Modifier = Modifier(w * scope.factor)`, `fun Column(content: ColumnScope.() -> Unit) { println("column {"); content(ColumnScope(2.0)); println("}") }`, `fun Row(content: RowScope.() -> Unit) { println("row {"); RowScope().content(); println("}") }` and `fun Text(text: String, modifier: Modifier = Modifier(0.0)) { println(text + " " + modifier.fraction.toString()) }` are declared, and `fun main() { Column { Text("a", Modifier(0.0).weight(1.0)); Row { Text("b", Modifier(0.0).weight(1.0)); Text("c") }; listOf(3.0).forEach { Text("d", Modifier(0.0).weight(it)) } } }` runs with `--check-allocations`
- **THEN** it prints `column {`, `a 2.0`, `row {`, `b 1.0`, `c 0.0`, `}`, `d 6.0` and `}`, reports no live allocation and exits with status 0
