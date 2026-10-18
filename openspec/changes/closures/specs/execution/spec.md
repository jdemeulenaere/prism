# Execution Specification Delta

## ADDED Requirements

### Requirement: Calling a Function Value Runs Its Closure
Calling a function value, also through references and boxes, MUST run the body of the closure, function or variant constructor it was made from, with the call's arguments, and give that body's result.

#### Scenario: Calling a closure passed to a function
- **WHEN** `fun apply(f: (Int, Int) -> Int): Int = f(3, 4)` is declared and `fun main() { println(apply { a, b -> a * b }) }` runs
- **THEN** it prints `12` and exits with status 0

#### Scenario: Calling a closure held in a local
- **WHEN** `fun main() { val inc: (Int) -> Int = { n -> n + 1 }; println(inc(1)); println(inc(inc(5))) }` runs
- **THEN** it prints `2` and then `7`

#### Scenario: A closure returning a string
- **WHEN** `fun make(f: () -> String): String = f()` is declared and `fun main() { println(make { "a" + "b" }) }` runs with `--check-allocations`
- **THEN** it prints `ab`, reports no live allocation and exits with status 0

#### Scenario: Calling a function value through a reference
- **WHEN** `enum Action { Run((Int) -> Int), Stop }`, `fun act(a: &Action): Int = when (a) { Action.Run(val f) -> f(5); Action.Stop -> 0 }` and `fun call(f: &(() -> Int)): Int = f()` are declared and `fun main() { println(act(Action.Run { n -> n * 3 })); val g: () -> Int = { 4 }; println(call(g)); println(call { 7 }); val h = Heap({ 6 }); println(h()) }` runs with `--check-allocations`
- **THEN** it prints `15`, `4`, `7` and `6`, reports no live allocation and exits with status 0

#### Scenario: Calling a function value given by an expression
- **WHEN** `fun main() { val opt: (() -> Int)? = null; println((opt ?: { 0 })()); val c = 1 > 0; val a: () -> Int = { 5 }; val b: () -> Int = { 6 }; println((if (c) a else b)()) }` runs with `--check-allocations`
- **THEN** it prints `0` and then `5`, reports no live allocation and exits with status 0: the callee is a temporary, which takes the value its `?:` or branch gives

### Requirement: Closures Use the Variables They Capture
A closure MUST read and write the captured variables themselves, not copies: it MUST see their values when it runs, and its writes MUST be seen by the enclosing body after the closure returns.

#### Scenario: Reading captured variables
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun main() { val a = 2; var b = 5; b = 6; println(run { a * 10 + b }) }` runs
- **THEN** it prints `26`

#### Scenario: Writing a captured variable
- **WHEN** `fun repeat(n: Int, f: () -> Unit) { var i = 0; while (i < n) { f(); i += 1 } }` is declared and `fun main() { var count = 0; repeat(3) { count += 1 }; println(count) }` runs
- **THEN** it prints `3`

#### Scenario: Writing a captured string
- **WHEN** `repeat` is declared as above and `fun main() { var s = "a"; repeat(2) { s = s + "b" }; println(s) }` runs with `--check-allocations`
- **THEN** it prints `abb`, reports no live allocation and exits with status 0

#### Scenario: Writing a field of a captured struct
- **WHEN** `repeat` is declared as above, `struct Counter { var n: Int }` is declared and `fun main() { var c = Counter(0); repeat(4) { c.n += 2 }; println(c.n) }` runs
- **THEN** it prints `8`

#### Scenario: Nested closures
- **WHEN** `repeat` is declared as above and `fun main() { var total = 0; repeat(2) { repeat(3) { total += 1 } }; println(total) }` runs
- **THEN** it prints `6`

#### Scenario: A closure returning a borrow of a capture
- **WHEN** `struct User { val name: String }` and `fun size(f: () -> &String): Int = f().length` are declared and `fun main() { val u = User("Ada"); println(size { &u.name }) }` runs with `--check-allocations`
- **THEN** it prints `3`, reports no live allocation and exits with status 0

### Requirement: Function Names and Variant Constructors Are Function Values
A top-level function, of the file or of an import, and a variant constructor used as a value MUST behave as a closure that calls the function, or builds the variant, with its arguments converted to the declared parameter types.

#### Scenario: A function passed as a value
- **WHEN** `fun double(n: Int): Int = n * 2` and `fun twice(f: (Int) -> Int, x: Int): Int = f(f(x))` are declared and `fun main() { println(twice(double, 3)) }` runs
- **THEN** it prints `12`

#### Scenario: An imported function passed as a value
- **WHEN** `lib/math.prism` declares `public fun triple(n: Int): Int = n * 3`, and `app/main.prism` imports `/lib/math` and declares `fun twice(f: (Int) -> Int, x: Int): Int = f(f(x))` and `fun main() { println(twice(triple, 1)) }`
- **THEN** `prism run app/main.prism` prints `9`

#### Scenario: A function taking a nullable used where an integer is passed
- **WHEN** `fun show(n: Int?) { println(n ?: 0) }` and `fun each(f: (Int) -> Unit) { f(4); f(5) }` are declared and `fun main() { each(show) }` runs
- **THEN** it prints `4` and then `5`

#### Scenario: A variant constructor passed as a value
- **WHEN** `enum Shape { Circle(Int), Dot }` and `fun build(f: (Int) -> Shape): Shape = f(3)` are declared and `fun main() { println(build(Shape.Circle) == Shape.Circle(3)) }` runs
- **THEN** it prints `true`

### Requirement: Function Values Are Held, Moved and Dropped Like Other Values
A function value MUST be storable in a struct field, an enum payload, a nullable, a local or a top-level `val`, and moved and returned like any value. Calling it after a move MUST run the same closure, and dropping it MUST release whatever it owns exactly once.

#### Scenario: A struct holding a function value
- **WHEN** `struct Task { val name: String, val run: () -> Int }` and `fun start(t: &Task): Int = t.run()` are declared and `fun main() { val t = Task("a".clone(), { 42 }); println(start(t)); val u = t; println(u.run()) }` runs with `--check-allocations`
- **THEN** it prints `42` twice, reports no live allocation and exits with status 0

#### Scenario: A nullable function value
- **WHEN** `fun call(f: (() -> Int)?): Int { val g = f ?: { 0 }; return g() }` is declared and `fun main() { println(call(null)); val f: () -> Int = { 5 }; println(call(f)) }` runs
- **THEN** it prints `0` and then `5`

#### Scenario: A function value in a top-level value
- **WHEN** `fun double(n: Int): Int = n * 2`, `val TWICE: (Int) -> Int = double`, `val SEVEN: () -> Int = { 7 }` and `fun main() { println(TWICE(4)); println(TWICE(5)); println(SEVEN()) }` are declared and run with `--check-allocations`
- **THEN** it prints `8`, `10` and `7`, reports no live allocation and exits with status 0

#### Scenario: Function values returned from a function
- **WHEN** `fun pick(big: Bool): (Int) -> Int { if (big) return { n -> n * 100 }; return { n -> n + 1 } }` is declared and `fun main() { val f = pick(true); val g = pick(false); println(f(2)); println(g(2)) }` runs
- **THEN** it prints `200` and then `3`

### Requirement: Closures Run in Generic Code and Default Values
A closure passed to a generic function, written inside a generic body or written in a parameter's default value MUST run as any closure does, at every instantiation.

#### Scenario: A generic function calling a closure
- **WHEN** `fun <T> apply(x: T, f: (T) -> T): T = f(x)` is declared and `fun main() { println(apply(2) { n -> n + 1 }); println(apply("a") { s -> s + "b" }) }` runs with `--check-allocations`
- **THEN** it prints `3` and then `ab`, reports no live allocation and exits with status 0

#### Scenario: A closure inside a generic body
- **WHEN** `fun run(g: () -> Int): Int = g()` and `fun <T : Copy> both(x: T, f: (T, T) -> Int): Int = run { f(x, x) }` are declared and `fun main() { println(both(4) { a, b -> a * b }) }` runs
- **THEN** it prints `16`

#### Scenario: A closure in a default value
- **WHEN** `fun greet(name: &String, format: (&String) -> String = { n -> "Hi " + n }): String = format(name)` is declared and `fun main() { println(greet("Ada")); println(greet("Bo") { n -> n + "!" }) }` runs with `--check-allocations`
- **THEN** it prints `Hi Ada` and then `Bo!`, reports no live allocation and exits with status 0

### Requirement: Runtime Errors Inside Closures Are Located in Them
A runtime error raised by a closure's body MUST report the location of the failing operation inside the closure, in the closure's file.

#### Scenario: Division by zero in a closure
- **WHEN** `app/main.prism` declares `fun run(f: () -> Int): Int = f()` on line 2 and `fun main() { val zero = 0; println(1); println(run { 10 / zero }) }` on line 3, with `10 / zero` starting at column 54
- **THEN** `prism run app/main.prism` prints `1` to stdout, stderr is exactly `panic: division by zero` and `  at app/main.prism:3:54`, each followed by a newline, and the exit status is 101
