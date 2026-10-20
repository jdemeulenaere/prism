# Execution Specification

## Purpose

The execution specification defines what happens when a Prism program runs: how `prism run` starts it and reports its end, the program's entry point, what the output built-ins print, the runtime meaning of scalar arithmetic and evaluation order, how runtime errors end a program, and when top-level values are initialized.

## Requirements

### Requirement: Running a Program
`prism run <file.prism>` MUST check the entry file and every file it transitively imports as `prism check` does, compile them, and call the entry file's `main`. Diagnostics MUST go to stderr and the program's output to stdout. It MUST exit with status 0 when `main` returns, and with status 1, without running any of the program, when the program does not compile.

#### Scenario: Running a multi-file program
- **WHEN** `lib/math.prism` declares `public fun twice(n: Int): Int = n * 2`, and `app/main.prism` imports `/lib/math` and declares `fun main() { println(twice(21)) }`
- **THEN** `prism run app/main.prism` prints `42` followed by a newline to stdout, nothing to stderr, and exits with status 0

#### Scenario: A program that does not compile
- **WHEN** `prism run main.prism` runs on `fun main() { println(1); val x: Int = true }`, or on `fun take(s: String) {}` and `fun main() { val s = "a"; take(s); take(s) }`
- **THEN** it reports `E4002`, or `E5001`, on stderr, prints nothing to stdout and exits with status 1

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

### Requirement: Watch Options
`--watch`, before the file, MUST run the program in a watch session that reloads it when its files change (`hot-reload`), passing the same arguments to every run of `main`. `--edits <script>` MUST drive that session's reloads from a script, and without `--watch` is a usage error with status 1. Under `--watch`, `--timings` MUST also print the stages of each reload, `--no-cache` keeps the whole session out of the cache, and `--explain-cache` explains its first load.

#### Scenario: Edits without watching
- **WHEN** `prism run --edits edits main.prism` runs
- **THEN** it prints the usage of `prism run` to stderr, runs nothing and exits with status 1

### Requirement: Program Entry Point
The entry file MUST declare a top-level function `main` that has no type parameters, receiver or parameters, returns `Unit` (written or omitted) and is not `compose`. Without one, `prism run` MUST report `E6002` (`MissingMain`); a `main` breaking these rules, or a top-level `val` named `main`, MUST be reported as `E6003` (`InvalidMain`). A `main` of any other file is an ordinary function.

#### Scenario: Missing main
- **WHEN** `prism run main.prism` runs on a file declaring only `fun helper() {}`
- **THEN** it reports `E6002` and exits with status 1

#### Scenario: Invalid main
- **WHEN** the entry file declares `fun main(n: Int) {}`, or `fun main(): Int = 0`, or `fun <T> main() {}`, or `compose fun main() {}`, or `val main: Int = 1`
- **THEN** `prism run` reports `E6003` at `main` and exits with status 1

#### Scenario: A main of an imported file is ordinary
- **WHEN** `lib/tool.prism` declares `public fun main(n: Int): Int = n`, and `app/main.prism` imports `/lib/tool` and declares `fun main() { println(2) }`
- **THEN** `prism run app/main.prism` prints `2` and exits with status 0

### Requirement: Output Built-Ins Print Their Argument
`print(x)` MUST write the text of `x` to stdout, `println(x)` that text and a newline, and `println()` a newline. An `Int` MUST be written in decimal with a leading `-` when negative, a `Bool` as `true` or `false`, a `Char` as its UTF-8 encoding and a `String` as its UTF-8 text. Any other value MUST be written as the text its `toString()` returns, called once per call, after the argument is evaluated, and dropped after it is written.

#### Scenario: Printing scalars
- **WHEN** `fun main() { print(1); print(true); println('x'); println(-5); println() }` runs
- **THEN** stdout holds `1truex`, a newline, `-5`, a newline, and a newline

#### Scenario: Printing strings
- **WHEN** `fun main() { val s = "Grüße, \u{1F680}"; print(s); println("!"); println(s) }` runs
- **THEN** stdout holds `Grüße, 🚀!`, a newline, `Grüße, 🚀` and a newline

#### Scenario: Printing a value through its toString
- **WHEN** `struct P(val x: Int, val y: Int)`, `fun P.toString(): String = "(" + x.toString() + ", " + y.toString() + ")"` and `fun main() { val p = P(1, 2); println(p); print(p); println("!") }` run with `--check-allocations`
- **THEN** stdout holds `(1, 2)`, a newline, `(1, 2)!` and a newline, and every string `toString()` built is freed

### Requirement: Double Text Form
A `Double` MUST be written as Kotlin writes it (Java's `Double.toString`): as the shortest decimal that reads back as the same value, or, when that decimal has a single significant digit, as the closest decimal of one or two significant digits that reads back as the same value; with at least one digit after the point; in scientific notation (`1.0E7`, `1.0E-4`) when its magnitude is at least 10^7 or below 10^-3; and as `NaN`, `Infinity`, `-Infinity` and `-0.0` for those values.

#### Scenario: Printing doubles
- **WHEN** a program prints `1.0`, `0.1 + 0.2`, `10000000.0`, `0.0001`, `1.0 / 0.0`, `0.0 / 0.0` and `-0.0`, each with `println`
- **THEN** stdout holds the lines `1.0`, `0.30000000000000004`, `1.0E7`, `1.0E-4`, `Infinity`, `NaN` and `-0.0`

#### Scenario: Printing the smallest double
- **WHEN** a program prints `4.9E-324`, the smallest positive `Double`, and `1.0E-323`, its double, each with `println`
- **THEN** stdout holds the lines `4.9E-324` and `9.9E-324`, the closest two-digit decimals, rather than the shortest `5.0E-324` and `1.0E-323`

### Requirement: Output Is Buffered and Flushed
Output MUST appear in the order the program writes it. It MUST be written to stdout at the latest when the program ends, before a runtime error is reported, and at each newline when stdout is a terminal.

#### Scenario: Output before a runtime error
- **WHEN** `fun zero(): Int = 0` is declared and `fun main() { println(1); println(1 / zero()) }` runs
- **THEN** stdout holds `1` and a newline, stderr holds the runtime error, and the exit status is 101

### Requirement: Integer Arithmetic
`Int` MUST be a 64-bit two's complement integer. `+`, `-`, `*`, unary `-` and `/` MUST raise the runtime error "integer overflow" when the exact result does not fit; `/` and `%` by zero MUST raise "division by zero". `/` MUST truncate toward zero and `%` MUST take the sign of the dividend, so the smallest `Int` `% -1` is `0`. Compound assignments MUST behave as their operators.

#### Scenario: Overflowing addition
- **WHEN** `fun main() { val big = 9223372036854775807; println(big + 1); println(2) }` runs
- **THEN** stdout is empty, stderr holds `panic: integer overflow in '+'` and the location of `big + 1`, and the exit status is 101

#### Scenario: Overflow in every form
- **WHEN** a program evaluates `9223372036854775807 * 2`, or `-9223372036854775807 - 2`, or `-m` or `m / -1` for `val m = -9223372036854775807 - 1`, or runs `var n = 9223372036854775807; n += 1`
- **THEN** it raises "integer overflow", naming `*`, `-`, `-`, `/` and `+` respectively, and exits with status 101

#### Scenario: Division by zero
- **WHEN** `fun zero(): Int = 0` is declared and a program evaluates `7 / zero()`, or `7 % zero()`
- **THEN** it raises "division by zero" and exits with status 101

#### Scenario: Truncating division and remainder
- **WHEN** a program prints `-7 / 2`, `-7 % 2`, `7 % -2` and `m % -1` for `val m = -9223372036854775807 - 1`
- **THEN** it prints `-3`, `-1`, `1` and `0`

### Requirement: Double Arithmetic
`Double` arithmetic MUST follow IEEE 754 binary64 with rounding to nearest and MUST never raise a runtime error: division by zero gives an infinity or NaN. `%` MUST be the remainder of truncated division, with the sign of the dividend. Every comparison with NaN MUST be false, except `!=`, which is true.

#### Scenario: IEEE results
- **WHEN** a program prints `-7.5 % 2.0`, `1.0 / 0.0 > 1.0`, and, for `val n = 0.0 / 0.0`, `n == n` and `n != n`
- **THEN** it prints `-1.5`, `true`, `false` and `true`

### Requirement: Runtime Errors End the Program
A runtime error MUST flush the program's output, write `panic: <message>` and then `  at <path>:<line>:<column>` to stderr, where the path is the file's path relative to the workspace root and the position is that of the failing operation, and exit with status 101. No further code of the program MUST run, and no drop MUST run. "out of memory", "stack overflow" raised inside glue, and composition misuse (compose) MUST write no location line.

#### Scenario: Runtime error location
- **WHEN** `app/main.prism` declares `fun zero(): Int = 0` on line 2 and `fun main() { val a = 1; println(a / zero()) }` on line 3, with `a / zero()` starting at column 33
- **THEN** stderr is exactly `panic: division by zero` and `  at app/main.prism:3:33`, each followed by a newline

#### Scenario: A composition misuse has no location
- **WHEN** `import /prism/composer` and `fun main() { val c = startComposition { k -> endGroup(k) } }` run
- **THEN** stderr is exactly `panic: composition misuse: end of a group that was not started` and a newline, with no location line

### Requirement: A Runtime Error Under Watch Ends Only the Run
Under `prism run --watch`, a runtime error MUST be reported as "Runtime Errors End the Program" states, and MUST end the program's run instead of the process: no further code of the run and no drop runs, and the watch session continues (`hot-reload`).

#### Scenario: A runtime error under watch
- **WHEN** `prism run --watch --edits edits app/main.prism` runs the program of "Runtime error location", with an edit script whose only step is `after exit` and fixes `zero` to return `1`
- **THEN** stderr holds `panic: division by zero` and `  at app/main.prism:3:33`, then `watch: the program stopped with a runtime error; waiting for changes`, and the program runs again

### Requirement: Stack Overflow Is a Runtime Error
A program whose calls nest deeper than its stack allows MUST raise the runtime error "stack overflow" at the declaration of the function being entered, or with no location when the stack runs out inside drop, clone or equality glue, which ends the program as any runtime error does. A program MUST get a stack of the same size on every platform.

#### Scenario: Unbounded recursion
- **WHEN** `fun deep(n: Int): Int = 1 + deep(n + 1)` is declared and `fun main() { println(1); println(deep(0)) }` runs
- **THEN** stdout holds `1` and a newline, stderr holds `panic: stack overflow` and the location of the declaration of `deep`, and the exit status is 101

#### Scenario: Dropping a structure deeper than the stack
- **WHEN** `struct Node { val next: Heap<Node>? }` is declared and `fun main() { var list: Heap<Node>? = null; var i = 0; while (i < 1000000) { list = Heap(Node(list)); i += 1 }; println(i) }` runs
- **THEN** stdout holds `1000000` and a newline, stderr holds exactly `panic: stack overflow` and a newline, and the exit status is 101

### Requirement: Top-Level Values Initialize on First Read
A top-level `val`'s initializer MUST run when the program first reads the value, and only then; later reads MUST give the same value without running it again. A value the program never reads MUST never be initialized. Reading a value while its own initializer runs MUST raise the runtime error "top-level value '<name>' read during its own initialization".

#### Scenario: Initialized once on first read
- **WHEN** `fun noisy(): Int { println(0); return 5 }`, `val V: Int = noisy()` and `val W: Int = noisy()` are declared and `fun main() { println(1); println(V); println(V) }` runs
- **THEN** stdout holds the lines `1`, `0`, `5` and `5`

#### Scenario: Reading a value during its own initialization
- **WHEN** `val A: Int = B + 1` and `val B: Int = A + 1` are declared and `fun main() { println(A) }` runs
- **THEN** the program raises "top-level value 'A' read during its own initialization" at the read of `A` in `B`'s initializer and exits with status 101

### Requirement: When Branches Evaluate in Order
A `when` MUST evaluate its subject once, then test its patterns in order, evaluating each pattern only when the earlier ones did not match, and evaluate only the branch of the first pattern that matches, or the `else` branch when none does.

#### Scenario: When pattern matching
- **WHEN** `fun f(x: Int): Int = when (x) { 0 -> 10; in 1..10 -> 20; else -> 30 }` is declared and a program prints `f(0)`, `f(5)` and `f(11)`
- **THEN** it prints `10`, `20` and `30`

#### Scenario: Later patterns are not evaluated
- **WHEN** `fun loud(n: Int): Int { println(n); return n }` is declared and a program runs `when (loud(1)) { loud(1) -> println(7); loud(2) -> println(8); else -> println(9) }`
- **THEN** it prints `1`, `1` and `7`, and `loud(2)` is never called

#### Scenario: Exhaustive when over a Bool
- **WHEN** `fun f(b: Bool): Int = when (b) { true -> 1; false -> 0 }` is declared and a program prints `f(true)` and `f(false)`
- **THEN** it prints `1` and `0`, with no `else` branch

### Requirement: Logical Operators Short-Circuit
`&&` MUST evaluate its right operand only when its left operand is `true`, and `||` only when its left operand is `false`.

#### Scenario: Right operands skipped
- **WHEN** `fun loud(b: Bool): Bool { println(b); return b }` is declared and a program runs `println(loud(false) && loud(true))` and `println(loud(true) || loud(false))`
- **THEN** it prints `false`, `false`, `true` and `true`

### Requirement: Struct Values
Constructing a struct MUST evaluate its field values in source order and build a value holding them; reading a field MUST give the value last stored in it, and assigning a `var` field of a `var` binding, or through a mutable reference, MUST change that field only.

#### Scenario: Constructing and reading a struct
- **WHEN** `struct Point : Copy { var x: Int, var y: Int }` is declared and `fun main() { var p = Point(1, 2); p.y = 5; println(p.x); println(p.y) }` runs
- **THEN** it prints `1` and then `5`

### Requirement: Copies and Moves Are Independent Values
Copying or moving a struct, enum or nullable MUST give a value whose later changes do not affect the source, and changes to the source, where it is still usable, MUST not affect the copy. A move and a copy MUST run no user code.

#### Scenario: Copies are independent
- **WHEN** `struct Point : Copy { var x: Int, var y: Int }` is declared and `fun main() { val a = Point(1, 2); var b = a; b.x = 7; println(a.x); println(b.x) }` runs
- **THEN** it prints `1` and then `7`

#### Scenario: Moving a struct into a function
- **WHEN** `struct Counter { var n: Int }` and `fun take(c: Counter): Int = c.n` are declared and `fun main() { val c = Counter(4); println(take(c)) }` runs
- **THEN** it prints `4`

### Requirement: Aggregate Arguments and Results Are Values
A struct, enum or nullable passed to a function MUST be received as the value it had when the argument was evaluated, and a returned one MUST be the value the function built, even when the result is assigned to the place an argument came from.

#### Scenario: Returning a struct over its own argument
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `fun mirror(p: Point): Point = Point(p.y, p.x)` are declared and `fun main() { var p = Point(1, 2); p = mirror(p); println(p.x); println(p.y) }` runs
- **THEN** it prints `2` and then `1`

#### Scenario: An argument is not changed by a write through another argument
- **WHEN** `struct Point : Copy { var x: Int }` and `fun f(p: Point, r: &mut Point): Int { r.x = 9; return p.x }` are declared and `fun main() { var a = Point(1); println(f(a, &mut a)); println(a.x) }` runs
- **THEN** it prints `1` and then `9`

### Requirement: Borrows Refer to the Borrowed Place
A borrow of a value, of a field or of a nullable's payload MUST refer to that place itself, never to a copy: a write through a mutable borrow MUST change the borrowed place, and an assignment through a reference MUST replace the referent, so its owner sees the new value. Viewing a `&(T?)` as a `(&T)?` MUST give `null` when the nullable is `null` and otherwise a borrow of its payload.

#### Scenario: Writing through a borrow of a field
- **WHEN** `struct Point : Copy { var x: Int, var y: Int }` and `fun bump(r: &mut Int) { *r = r + 1 }` are declared and `fun main() { var p = Point(1, 2); bump(&mut p.x); println(p.x) }` runs
- **THEN** it prints `2`

#### Scenario: Writing through a reference replaces the referent
- **WHEN** `struct Point : Copy { var x: Int, var y: Int }` and `fun set(r: &mut Point, v: Point) { *r = v }` are declared and `fun main() { var a = Point(1, 2); set(&mut a, Point(5, 6)); println(a.x); println(a.y) }` runs
- **THEN** it prints `5` and then `6`

#### Scenario: Viewing a borrowed nullable
- **WHEN** `struct User : Copy { val nick: Int? }`, `fun show(n: (&Int)?): Int { val r = n ?: &0; return r }` and `fun nick(u: &User): Int = show(&u.nick)` are declared and a program prints `nick(&User(4))` and `nick(&User(null))`
- **THEN** it prints `4` and then `0`

### Requirement: When Over Enum Variants and Nullables
A `when` over an enum MUST run the branch of the subject's variant, and a `when` over a nullable MUST match a `null` pattern exactly when the subject is `null` and a value pattern exactly when the subject holds an equal value.

#### Scenario: When over variants
- **WHEN** `enum Color { Red, Green, Blue }` and `fun g(c: Color): Int = when (c) { Red -> 1; Green -> 2; Blue -> 3 }` are declared and a program prints `g(Color.Blue)` and `g(Color.Red)`
- **THEN** it prints `3` and then `1`

#### Scenario: When over a nullable
- **WHEN** `fun f(n: Int?): Int = when (n) { null -> 0; 1 -> 10; else -> 20 }` is declared and a program prints `f(null)`, `f(1)` and `f(2)`
- **THEN** it prints `0`, `10` and `20`

#### Scenario: Exhaustive when over a nullable Bool
- **WHEN** `fun f(b: Bool?): Int = when (b) { true -> 1; false -> 2; null -> 3 }` is declared and a program prints `f(true)`, `f(false)` and `f(null)`
- **THEN** it prints `1`, `2` and `3`, with no `else` branch

### Requirement: Equality of Structs, Enums and Nullables
`==` MUST compare structs field by field, enums by variant and then payload, except a struct or enum with an `equals` operator (`operators`), which it MUST compare by calling it, and nullables as equal when both are `null` or both hold equal values, each field or payload with `==` of its type; `!=` MUST be its negation. Comparing borrows MUST compare what they refer to.

#### Scenario: Structural equality
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `enum Shape : Copy { Circle(Double), Square(Double), Empty }` are declared and a program prints `Point(1, 2) == Point(1, 2)`, `Point(1, 2) == Point(2, 1)`, `Shape.Circle(1.0) == Shape.Circle(2.0)`, `Shape.Circle(1.0) != Shape.Square(1.0)` and `Shape.Empty == Shape.Empty`
- **THEN** it prints `true`, `false`, `false`, `true` and `true`

#### Scenario: Equality of nullables
- **WHEN** a program declares `val a: Int? = null`, `val b: Int? = 1` and `val c: Int? = 1`, and prints `a == null`, `a == b`, `b == c` and `b != null`
- **THEN** it prints `true`, `false`, `true` and `true`

#### Scenario: Fields compare as their types do
- **WHEN** `struct D : Copy { val v: Double }` is declared and a program prints `D(0.0 / 0.0) == D(0.0 / 0.0)` and `D(0.0) == D(-0.0)`
- **THEN** it prints `false` and then `true`

#### Scenario: Comparing values that are not Copy
- **WHEN** `struct Counter { val n: Int }` is declared and `fun same(a: Counter, b: Counter): Bool = a == b` and a program prints `same(Counter(1), Counter(1))`
- **THEN** it prints `true`

### Requirement: Null-Safe Operators Short-Circuit
`a ?: b` MUST evaluate `b` only when `a` is `null`. `a?.m(args)` and `a?.f` MUST give `null` without evaluating `args` when `a` is `null`, and otherwise call `m` or read `f` on the value `a` holds.

#### Scenario: Safe navigation and Elvis operator
- **WHEN** `struct User : Copy { val age: Int }` and `fun age(u: User?): Int = u?.age ?: 0` are declared and a program prints `age(null)` and `age(User(3))`
- **THEN** it prints `0` and then `3`

#### Scenario: A safe call on null evaluates no argument
- **WHEN** `struct User : Copy { val age: Int }`, `fun User.older(by: Int): Int = age + by` and `fun next(): Int { println(9); return 1 }` are declared and `fun main() { val u: User? = null; println(u?.older(next()) ?: 0); val v: User? = User(3); println(v?.older(next()) ?: 0) }` runs
- **THEN** it prints `0`, then `9`, then `4`

### Requirement: Cloning a Value That Owns Nothing
`x.clone()` of a value whose type owns no storage and reaches no clone hook (`ownership`) MUST give a bitwise copy of `x`, with no allocation and no user code run; one that reaches a clone hook is cloned by its hooks ("Clone Hooks Run Wherever Values Are Cloned").

#### Scenario: Copy value clone is a copy
- **WHEN** `struct Point : Copy { var x: Int }` is declared and `fun main() { var p = Point(1); val q = p.clone(); p.x = 2; println(q.x) }` runs
- **THEN** it prints `1`

#### Scenario: Clone of a Clone struct of scalars
- **WHEN** `struct Tally : Clone { var n: Int }` is declared and `fun main() { val a = Tally(3); var b = a.clone(); b.n = 4; println(a.n); println(b.n) }` runs
- **THEN** it prints `3` and then `4`

### Requirement: Ranges Are Values
A `Range` MUST be usable as a value like any `Copy` struct: passed to and returned from functions and stored in top-level values, keeping whether it includes its end.

#### Scenario: Passing ranges to a function
- **WHEN** `fun count(r: Range): Int { var n = 0; for (i in r) n += 1; return n }` is declared and a program prints `count(1..3)` and `count(1..<3)`
- **THEN** it prints `3` and then `2`

### Requirement: Top-Level Values of Aggregate Types
A top-level `val` of a struct, enum or nullable type MUST be initialized on first read like any top-level value, and every read MUST give the value its initializer built.

#### Scenario: Reading an imported top-level struct
- **WHEN** `lib/geo.prism` declares `public struct Point : Copy { val x: Int, val y: Int }` and `public val ORIGIN: Point = Point(0, 7)`, and `app/main.prism` imports `/lib/geo` and declares `fun main() { println(ORIGIN.y); val p = ORIGIN; println(p.x) }`
- **THEN** `prism run app/main.prism` prints `7` and then `0`

### Requirement: Tier 1 Instant Execution
On a release build, `prism run` MUST reach the program's `main` in under 15 ms for the reference program `examples/run/main.prism` on a developer machine, without invoking any external program. A timing benchmark MUST report this time against the target; it MUST NOT fail a test run on the time alone.

#### Scenario: Tier 1 instant script execution
- **WHEN** `prism run --timings examples/run/main.prism` runs on a release build of `prism`
- **THEN** no other process is started, and the reported total time to `main` is under 15 ms

### Requirement: String Values
A string literal MUST evaluate to a string holding its decoded text. `a + b` MUST give a new string holding the text of `a` followed by that of `b`, leaving both operands unchanged, and `s += t` MUST give `s` the value of `s + t`. `a == b` MUST be `true` exactly when both strings hold the same sequence of characters, and `a != b` its negation.

#### Scenario: Concatenation and equality
- **WHEN** `fun main() { val a = "pri"; val b = a + "sm"; println(b); println(a); println(b == "prism"); println(a != "pri") }` runs
- **THEN** it prints `prism`, `pri`, `true` and `false`

#### Scenario: Compound assignment of strings
- **WHEN** `fun main() { var s = "a"; val t = "b"; s += t; s += t; println(s); println(t) }` runs
- **THEN** it prints `abb` and then `b`

#### Scenario: When over strings
- **WHEN** `fun f(s: String): Int = when (s) { "one" -> 1; "two" -> 2; else -> 0 }` is declared and a program prints `f("two")`, `f("one")` and `f("three")`
- **THEN** it prints `2`, `1` and `0`

#### Scenario: Strings in structs and top-level values
- **WHEN** `lib/names.prism` declares `public struct User { val name: String, val nick: String? }` and `public val ADMIN: String = "ro" + "ot"`, and `app/main.prism` imports `/lib/names` and declares `fun main() { val u = User(ADMIN.clone(), null); println(u.name); println(u.nick == null); println(ADMIN) }`
- **THEN** `prism run --check-allocations app/main.prism` prints `root`, `true` and `root`, writes nothing to stderr and exits with status 0

### Requirement: String Length and Characters
`s.length` MUST be the number of characters (Unicode scalar values) of `s`, and `s[i]` the character at index `i`, counting characters from 0. An index outside `0..<s.length` MUST raise the runtime error "string index <i> out of range for length <n>" at the indexing expression.

#### Scenario: Characters beyond ASCII
- **WHEN** `fun main() { val s = "héllo"; println(s.length); println(s[1]); println(s[4]) }` runs
- **THEN** it prints `5`, `é` and `o`

#### Scenario: An index out of range
- **WHEN** `fun main() { val s = "abc"; println(s[0]); println(s[3]) }` runs
- **THEN** stdout holds `a` and a newline, stderr holds `panic: string index 3 out of range for length 3` and the location of `s[3]`, and the exit status is 101

### Requirement: Scalars Convert to Text
`x.toString()` of an `Int`, `Double`, `Bool` or `Char` MUST give a new string holding the text that `print(x)` writes.

#### Scenario: Converting scalars
- **WHEN** `fun main() { val n = -42; println(n.toString() + "!"); println(0.1.toString()); println(1.0E7.toString()); println(true.toString()); println('é'.toString().length) }` runs
- **THEN** it prints `-42!`, `0.1`, `1.0E7`, `true` and `1`

### Requirement: Values Are Freed Where Their Owner's Scope Ends
Each drop that the elaborated MIR runs MUST free, at that point, the memory the dropped value owns, deterministically and without a garbage collector or reference counting, so that every allocation a value owns is freed exactly once: where its owner's scope ends, where it is overwritten by an assignment or a write through a reference, or by the code it was moved to. A top-level value MUST never be dropped while the program runs; only the allocation check drops them, after `main` returns.

#### Scenario: Deterministic scope exit cleanup
- **WHEN** `fun main() { var total = 0; for (i in 0..<1000) { val s = i.toString() + "x"; total += s.length }; println(total) }` runs with `--check-allocations`
- **THEN** it prints `3890`, writes nothing to stderr and exits with status 0

#### Scenario: A conditionally moved value is dropped only if present
- **WHEN** `fun take(s: String): Int = s.length` and `fun f(c: Bool) { val s = 1.toString(); if (c) println(take(s)) }` are declared and `fun main() { f(true); f(false) }` runs with `--check-allocations`
- **THEN** it prints `1`, writes nothing to stderr and exits with status 0

#### Scenario: Writing through a reference drops the old referent
- **WHEN** `fun set(r: &mut String, v: String) { *r = v }` is declared and `fun main() { var s = 1.toString(); set(&mut s, 2.toString()); s = 3.toString(); println(s) }` runs with `--check-allocations`
- **THEN** it prints `3`, writes nothing to stderr and exits with status 0

#### Scenario: Argument temporary dropped at the end of the statement
- **WHEN** `fun make(): String = 5.toString()` and `fun greet(n: &String): Int = n.length` are declared and `fun main() { println(greet(make())); println(greet(make())) }` runs with `--check-allocations`
- **THEN** it prints `1` twice, writes nothing to stderr and exits with status 0

#### Scenario: Top-level values are kept
- **WHEN** `val NAME: String = 4.toString() + "2"` is declared and `fun main() { println(NAME); println(NAME.length) }` runs with `--check-allocations`
- **THEN** it prints `42` and then `2`, writes nothing to stderr and exits with status 0

### Requirement: Cloning Values That Own Storage
`x.clone()` of a value that owns storage MUST give a value that shares no owned memory with it, so that changing or dropping either leaves the other unchanged. When neither its type nor any of its parts has a clone hook (`ownership`), the clone MUST be equal to `x` and MUST run no user code; otherwise the hooks build those parts.

#### Scenario: String clone is independent
- **WHEN** `fun main() { var a = 1.toString(); val b = a.clone(); a = 2.toString(); println(a); println(b) }` runs with `--check-allocations`
- **THEN** it prints `2` and then `1`, writes nothing to stderr and exits with status 0

#### Scenario: Struct clone clones each field
- **WHEN** `struct User : Clone { var name: String, val nick: String? }` is declared and `fun main() { var a = User(1.toString(), 2.toString()); val b = a.clone(); a.name = 3.toString(); println(a.name); println(b.name); println(b.nick == a.nick) }` runs with `--check-allocations`
- **THEN** it prints `3`, `1` and `true`, writes nothing to stderr and exits with status 0

### Requirement: Allocation Checking
With the option `--check-allocations` before the file, `prism run` MUST track every allocation of the program; without it nothing MUST be tracked. When `main` returns, allocations still live that no top-level value owns MUST be reported, after the output is flushed, as `allocation check: allocations still live at exit: <n> (<b> bytes)` on stderr, and the program MUST exit with status 102. A program ending with a runtime error MUST NOT be checked.

#### Scenario: A program that frees what it allocates
- **WHEN** `prism run --check-allocations main.prism` runs on `fun main() { val s = "a" + "b"; println(s) }`
- **THEN** it prints `ab`, writes nothing to stderr and exits with status 0

#### Scenario: A leak is reported
- **WHEN** a program run with `--check-allocations` returns from `main` while one allocation of 5 bytes, owned by no top-level value, is still live
- **THEN** stderr holds `allocation check: allocations still live at exit: 1 (5 bytes)` and the exit status is 102

#### Scenario: A runtime error is not checked
- **WHEN** `prism run --check-allocations main.prism` runs on `fun zero(): Int = 0` and `fun main() { val s = "a" + "b"; println(1 / zero()) }`
- **THEN** stderr holds only the runtime error, and the exit status is 101

### Requirement: Invalid Frees Are Reported
Under `--check-allocations`, a free of memory that is not a live allocation, or that gives another size or alignment than its allocation, MUST flush the output, write `allocation check: freed memory that is not allocated` to stderr and end the program with status 102.

#### Scenario: A double free is reported
- **WHEN** a program run with `--check-allocations` frees the same allocation twice
- **THEN** stderr holds `allocation check: freed memory that is not allocated` and the exit status is 102

### Requirement: Undisposed Compositions Are Reported
Under `--check-allocations`, when `main` returns, compositions of the program thread that were started and not disposed (compose, "Disposing a Composition Releases Everything") MUST be reported, after the output is flushed and before any report of live allocations, as `allocation check: compositions still live at exit: <n>` on stderr, and the program MUST exit with status 102. Without the option they MUST NOT be counted.

#### Scenario: A composition that is never disposed
- **WHEN** `import /prism/composer` and `fun main() { val c = startComposition { k -> println("ok") } }` run with `--check-allocations`
- **THEN** stdout holds `ok`, stderr is exactly `allocation check: compositions still live at exit: 1` and a newline, and the exit status is 102

#### Scenario: An undisposed composition and what it holds
- **WHEN** `fun Greeting(k: Int, name: String) { startRestartGroup(k, 3); println("hello " + name); endRestartGroup(k) { c -> Greeting(c, name.clone()) } }` is declared, and `fun main() { val c = startComposition { k -> Greeting(k, "a" + "b") } }` runs with `--check-allocations` and `import /prism/composer`
- **THEN** stdout is `hello ab`, stderr is the line `allocation check: compositions still live at exit: 1` followed by the line of live allocations, which counts the restart function's environment and its string, and the exit status is 102

### Requirement: Live Runtime Objects Are Reported
Under `--check-allocations`, when `main` returns with no live composition, the program thread's live groups, slots, state cells and nodes MUST be reported after the composition report and before the live allocations, one line per non-zero count in this order: `allocation check: groups still live at exit: <n>`, then `slots`, `state cells` and `nodes` in the same form. Any line MUST make the exit status 102. With a live composition, which holds them, or without the option, none MUST be reported.

#### Scenario: A leaked group, cell and node are reported
- **WHEN** a Rust test leaks one group, one state cell and one node through the runtime's test hooks, with no composition live, and runs the program's finish report
- **THEN** the report is exactly the lines `allocation check: groups still live at exit: 1`, `allocation check: state cells still live at exit: 1` and `allocation check: nodes still live at exit: 1`

#### Scenario: An app that ends cleanly reports nothing
- **WHEN** `fun main() { runApp(width = 200, height = 100) { val c = remember { mutableStateOf(0) }; Text("n = ${c.value}") } }` runs with `import /prism/compose/ui` and `import /prism/compose/material3` under `prism run --check-allocations --headless`
- **THEN** stderr is empty and the exit status is 0

### Requirement: Frame Statistics Option
The option `--frame-stats`, before the file, MUST make every `runApp` of the program measure its frames and print its report to stderr when it returns (`ui`, "Frame Statistics Report"), in headless runs, in windows and under `--watch`. It MUST NOT change the program's stdout or its exit status, and a program that never calls `runApp` MUST print nothing more.

#### Scenario: Frame statistics without an app
- **WHEN** `prism run --frame-stats main.prism` runs on `fun main() { println(1) }`
- **THEN** stdout is `1` and a newline, stderr is empty, and the exit status is 0

#### Scenario: Frame statistics of a headless app
- **WHEN** `prism run --frame-stats --headless --events events main.prism` runs the counter screen of `ui`'s "Counter screen" in `runApp`, with the script `click "+"` and `frame`
- **THEN** stdout is empty, stderr starts with a line `frame stats: frames 1, events 2, passes 2, reloads 0, ` and holds the lines of the report, and the exit status is 0

### Requirement: Running Out of Memory Is a Runtime Error
An allocation the system cannot satisfy, or a string whose length would not fit in memory, MUST raise the runtime error "out of memory", which ends the program as any runtime error does but reports no location.

#### Scenario: An allocation that fails
- **WHEN** the runtime cannot allocate the memory a program asks for
- **THEN** the output is flushed, stderr holds exactly `panic: out of memory` and a newline, and the exit status is 101

### Requirement: Patterns Match Payloads at Run Time
A constructor pattern MUST match exactly when the subject holds its variant, or is its struct, and every sub-pattern matches the part at its position. A binding MUST hold the matched part, a copy of it, or a borrow of it, as its mode says.

#### Scenario: Reading payloads
- **WHEN** `enum Shape { Circle(Int), Rect(Int, Int), Empty }` and `fun area(s: Shape): Int = when (s) { Shape.Circle(val r) -> 3 * r * r; Shape.Rect(val w, val h) -> w * h; Shape.Empty -> 0 }` are declared and a program prints `area(Shape.Circle(2))`, `area(Shape.Rect(3, 4))` and `area(Shape.Empty)`
- **THEN** it prints `12`, `12` and `0`

#### Scenario: Nested and literal sub-patterns
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }`, `enum Event : Copy { Click(Point), Key(Char) }` and `fun f(e: Event): Int = when (e) { Event.Click(Point(0, val y)) -> y; Event.Click(Point(val x, _)) -> x * 10; Event.Key('q') -> -1; Event.Key(_) -> -2 }` are declared and a program prints `f(Event.Click(Point(0, 7)))`, `f(Event.Click(Point(2, 7)))`, `f(Event.Key('q'))` and `f(Event.Key('w'))`
- **THEN** it prints `7`, `20`, `-1` and `-2`

#### Scenario: Writing a payload through a mutable binding
- **WHEN** `enum Counter { Running(Int), Stopped }`, `fun tick(c: &mut Counter) { when (c) { Counter.Running(val n) -> *n += 1; Counter.Stopped -> {} } }` and `fun value(c: &Counter): Int = when (c) { Counter.Running(val n) -> n; Counter.Stopped -> -1 }` are declared and `fun main() { var c = Counter.Running(1); tick(&mut c); tick(&mut c); println(value(&c)) }` runs
- **THEN** it prints `3`

#### Scenario: Binding the whole subject
- **WHEN** `enum Note { Text(String), Empty }`, `fun keep(n: Note): Note = when (n) { Note.Empty -> Note.Empty; val other -> other }` and `fun text(n: &Note): String = when (n) { Note.Text(val s) -> s.clone(); Note.Empty -> "" }` are declared and `fun main() { println(text(&keep(Note.Text("kept")))) }` runs with `prism run --check-allocations`
- **THEN** it prints `kept` and exits with status 0, reporting no leak and no invalid free

### Requirement: Guards Run After Their Pattern Matches
A guard MUST run only when its pattern matched, after the tests of that pattern and before the next pattern is tested. A false guard MUST let testing continue with the next pattern, the subject unchanged.

#### Scenario: Guard order and fallthrough
- **WHEN** `enum Box { Of(Int), Empty }`, `fun big(n: Int): Bool { println(n); return n > 9 }` and `fun f(b: Box): Int = when (b) { Box.Empty -> 0; Box.Of(val n) if big(n) -> 1; Box.Of(_) -> 2 }` are declared and a program prints `f(Box.Empty)`, `f(Box.Of(10))` and `f(Box.Of(3))`
- **THEN** it prints `0`; then `10` and `1`; then `3` and `2`

#### Scenario: Alternatives with a guard
- **WHEN** `enum Shape { Circle(Int), Square(Int) }` and `fun f(s: Shape): Int = when (s) { Shape.Circle(val d), Shape.Square(val d) if d > 1 -> d; else -> 0 }` are declared and a program prints `f(Shape.Circle(5))`, `f(Shape.Square(5))` and `f(Shape.Square(1))`
- **THEN** it prints `5`, `5` and `0`

### Requirement: Taken-Apart Values Release Each Part Once
When a value is taken apart, each moved part MUST be owned by its binding from then on, and each part no binding takes MUST be dropped where the branch or the enclosing block ends, so every allocation the value owned is freed exactly once.

#### Scenario: Taking strings apart
- **WHEN** `enum Note { Pair(String, String), Empty }` and `fun first(n: Note): String = when (n) { Note.Pair(val a, _) -> a; Note.Empty -> "" }` are declared and `fun main() { println(first(Note.Pair("left", "right"))) }` runs with `prism run --check-allocations`
- **THEN** it prints `left` and exits with status 0, reporting no leak and no invalid free

#### Scenario: Destructuring a consuming receiver
- **WHEN** `struct Account { val name: String, val bio: String, val id: Int }` and `consuming fun Account.label(): String { val (name, id) = this; return name }` are declared and `fun main() { val a = Account("ada", "math", 1); println(a.label()) }` runs with `prism run --check-allocations`
- **THEN** it prints `ada` and exits with status 0, reporting no leak and no invalid free

#### Scenario: Borrowing leaves the subject whole
- **WHEN** `enum Note { Text(String), Empty }`, `fun size(s: &String): Int = s.length` and `fun main() { val n = Note.Text("abc"); val k = when (&n) { Note.Text(val s) -> size(s); Note.Empty -> 0 }; println(k); val m = n }` run with `prism run --check-allocations`
- **THEN** it prints `3` and exits with status 0, reporting no leak and no invalid free

### Requirement: Boxes Hold Their Contents
`Heap(v)` MUST give a box holding `v`, and every read through the box (a field, an operator, a copy-out, a borrow of the contents, a pattern) MUST read the value it holds. Each box MUST be freed exactly once: by its drop, by `unbox()`, or when it is taken apart.

#### Scenario: Evaluating an expression tree
- **WHEN** `enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>), Mul(Heap<Expr>, Heap<Expr>) }` and `fun eval(e: &Expr): Int = when (e) { Expr.Lit(val n) -> n; Expr.Add(val l, val r) -> eval(l) + eval(r); Expr.Mul(val l, val r) -> eval(l) * eval(r) }` are declared and `fun main() { val e = Expr.Add(Heap(Expr.Lit(2)), Heap(Expr.Mul(Heap(Expr.Lit(3)), Heap(Expr.Lit(4))))); println(eval(e)) }` runs with `--check-allocations`
- **THEN** it prints `14`, writes nothing to stderr and exits with status 0

#### Scenario: Summing a linked list
- **WHEN** `struct Node { val value: Int, val next: Heap<Node>? }` and `fun Node.sum(): Int = value + (next?.sum() ?: 0)` are declared and `fun main() { var list: Heap<Node>? = null; for (i in 1..4) { list = Heap(Node(i, list)) }; println(list?.sum() ?: 0) }` runs with `--check-allocations`
- **THEN** it prints `10`, writes nothing to stderr and exits with status 0

#### Scenario: Measuring a tree
- **WHEN** `struct Tree { val value: Int, val left: Heap<Tree>?, val right: Heap<Tree>? }`, `fun leaf(v: Int): Heap<Tree> = Heap(Tree(v, null, null))`, `fun Tree.depth(): Int { val l = left?.depth() ?: 0; val r = right?.depth() ?: 0; return 1 + (if (l > r) l else r) }` and `fun valueOf(t: (&Tree)?): Int = t?.value ?: 0` are declared and `fun main() { val t = Tree(1, Heap(Tree(2, leaf(3), null)), leaf(4)); println(t.depth()); println(valueOf(t.left)); println(t.left?.left?.value ?: 0); println(t.right?.left?.value ?: 0) }` runs with `--check-allocations`
- **THEN** it prints `3`, `2`, `3` and `0`, writes nothing to stderr and exits with status 0

#### Scenario: Reading boxed scalars
- **WHEN** `fun show(n: &Int): Int = n * 10` is declared and `fun main() { val h = Heap(4); val n: Int = h; println(h + n); println(show(h)); println(h.toString() + "!"); println(h == Heap(4)) }` runs with `--check-allocations`
- **THEN** it prints `8`, `40`, `4!` and `true`, writes nothing to stderr and exits with status 0

### Requirement: Writing a Box's Contents Keeps the Box
`*h = v` MUST drop the box's old contents and move `v` into the same allocation, so that every reader of the box sees `v`, and MUST neither allocate nor free the box. Assigning a new box to a box place MUST drop the old box.

#### Scenario: Writing boxed strings
- **WHEN** `fun set(r: &mut Heap<String>, v: String) { **r = v }` is declared and `fun main() { var h = Heap(1.toString()); *h = 2.toString(); set(&mut h, 3.toString()); println(h); h = Heap(4.toString()); println(h) }` runs with `--check-allocations`
- **THEN** it prints `3` and then `4`, writes nothing to stderr and exits with status 0

### Requirement: Unboxing Moves the Contents Out
`h.unbox()` MUST give the value the box holds and free the box without dropping that value, and a `consuming fun` called through boxes MUST receive the contents, each box freed. `h?.unbox()` MUST give `null` for a `null` box.

#### Scenario: Unboxing and consuming through a box
- **WHEN** `struct User { val name: String }` and `consuming fun User.take(): String { val (name) = this; return name }` are declared and `fun main() { val u = Heap(User(1.toString())).unbox(); println(u.name); val h = Heap(Heap(User(2.toString()))); println(h.take()); val n: Heap<User>? = null; println(n?.unbox() == null) }` runs with `--check-allocations`
- **THEN** it prints `1`, `2` and `true`, writes nothing to stderr and exits with status 0

### Requirement: Dropping and Cloning Boxes
Dropping a box MUST drop its contents and then free it. Cloning a box MUST give a new box holding a clone of the contents, sharing no memory with the original, so that writing or dropping either leaves the other unchanged.

#### Scenario: Dropping a recursive value
- **WHEN** `enum Doc { Text(String), Pair(Heap<Doc>, Heap<Doc>) }` is declared and `fun main() { var i = 0; while (i < 100) { val d = Doc.Pair(Heap(Doc.Text(i.toString())), Heap(Doc.Pair(Heap(Doc.Text("x")), Heap(Doc.Text(i.toString()))))); i += 1 }; println(i) }` runs with `--check-allocations`
- **THEN** it prints `100`, writes nothing to stderr and exits with status 0

#### Scenario: A cloned box is independent
- **WHEN** `struct Node : Clone { var value: Int, val next: Heap<Node>? }` is declared and `fun main() { var a = Heap(Node(1, Heap(Node(2, null)))); val b = a.clone(); a.value = 5; println(a.value); println(b.value); println(b.next?.value ?: 0) }` runs with `--check-allocations`
- **THEN** it prints `5`, `1` and `2`, writes nothing to stderr and exits with status 0

#### Scenario: A box of a zero-sized value
- **WHEN** `struct Marker : Copy {}` is declared and `fun main() { val h = Heap(Marker()); val g = h.clone(); val m = g.unbox(); println(h == Heap(m)) }` runs with `--check-allocations`
- **THEN** it prints `true`, writes nothing to stderr and exits with status 0

### Requirement: Boxes Compare Their Contents
`a == b` on boxes MUST compare the values they hold structurally, through any depth of boxes, never their addresses, and `!=` MUST be its negation.

#### Scenario: Comparing recursive values
- **WHEN** `enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun sum(a: Int, b: Int): Expr = Expr.Add(Heap(Expr.Lit(a)), Heap(Expr.Lit(b)))` are declared and `fun main() { println(sum(1, 2) == sum(1, 2)); println(sum(1, 2) == sum(2, 1)); println(sum(1, 2) != Expr.Lit(3)) }` runs with `--check-allocations`
- **THEN** it prints `true`, `false` and `true`, writes nothing to stderr and exits with status 0

#### Scenario: A box holding NaN
- **WHEN** `fun main() { val h = Heap(0.0 / 0.0); println(h == h) }` runs
- **THEN** it prints `false`

### Requirement: Taking Apart Through a Box Frees the Box
When a pattern takes a value apart through a box, the box MUST be freed and the contents' parts bound or dropped as for any value taken apart, each exactly once.

#### Scenario: Consuming a linked list
- **WHEN** `struct Node { val value: Int, val next: Heap<Node>? }` and `fun total(list: Heap<Node>?): Int = when (list) { null -> 0; Node(val v, val rest) -> v + total(rest) }` are declared and `fun main() { println(total(Heap(Node(1, Heap(Node(2, Heap(Node(3, null)))))))) }` runs with `--check-allocations`
- **THEN** it prints `6`, writes nothing to stderr and exits with status 0

#### Scenario: Simplifying an expression tree
- **WHEN** `enum Expr { Lit(Int), Name(String), Add(Heap<Expr>, Heap<Expr>) }` and `fun simplify(e: Expr): Expr = when (e) { Expr.Add(Expr.Lit(0), val r) -> r.unbox(); Expr.Add(Expr.Name(val n), Expr.Lit(0)) -> Expr.Name(n); else -> e }` are declared and `fun main() { val a = simplify(Expr.Add(Heap(Expr.Lit(0)), Heap(Expr.Lit(7)))); val b = simplify(Expr.Add(Heap(Expr.Name(1.toString())), Heap(Expr.Lit(0)))); println(a == Expr.Lit(7)); println(b == Expr.Name("1")) }` runs with `--check-allocations`
- **THEN** it prints `true` twice, writes nothing to stderr and exits with status 0

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

### Requirement: Instances Drop and Clone Type-Parameter Values
In an instance, a value of a type parameter's type MUST be dropped, moved and copied as a value of the type argument is: dropping it runs the argument's drop glue and hooks, a move transfers its storage, and a `clone()` under a `Clone` bound builds a value that shares no storage with the original, through the argument's clone hooks if it has any. A value whose argument owns nothing and has no drop hook in it MUST drop without running any code.

#### Scenario: Dropping and cloning strings through generic code
- **WHEN** `struct Pair<A, B>(val a: A, val b: B)`, `fun <T> pick(a: T, b: T, first: Bool): T = if (first) a else b`, `fun <T> discard(x: T) {}` and `fun <T : Clone> both(x: &T): Pair<T, T> = Pair(x.clone(), x.clone())` are declared and `fun main() { println(pick("a", "b", false)); discard("c" + "d"); val p = both(&("e" + "f")); println(p.a + p.b) }` runs under `prism run --check-allocations`
- **THEN** the program prints `b` and `efef`, reports no live allocation and no double free, and exits with status 0

#### Scenario: Generic code over structs that own strings
- **WHEN** `struct User : Clone { val name: String }`, `struct Box<T>(val v: T)` and `fun <T> unwrap(b: Box<T>): T { val (v) = b; return v }` are declared and `fun main() { val u = unwrap(Box(User("ada"))); println(u.name); val c = unwrap(Box(Box(u.clone()))); println(c.v.name) }` runs under `prism run --check-allocations`
- **THEN** the program prints `ada` twice, reports no live allocation and exits with status 0

### Requirement: Drop Hooks Run Before the Value's Parts
Dropping a value whose type has a drop hook (`ownership`) MUST call the hook with a mutable borrow of the value, and then drop the value's parts as its type's drop does: a struct's fields in declaration order, an enum's payloads of its current variant in order. Hooks MUST run where the drops of the elaborated MIR run: locals in reverse order of creation where their scope ends, temporaries at the end of their statement.

#### Scenario: Locals run their hooks in reverse order
- **WHEN** `struct Noisy { val name: String }` and `operator mut fun Noisy.drop() { println("drop " + name) }` are declared and `fun main() { val a = Noisy("a"); val b = Noisy("b"); println("end") }` runs with `--check-allocations`
- **THEN** it prints `end`, `drop b` and `drop a`, and exits with status 0

#### Scenario: A temporary runs its hook at the end of its statement
- **WHEN** with that `Noisy`, `fun main() { println(Noisy("t").name.length); println("next") }` runs with `--check-allocations`
- **THEN** it prints `1`, `drop t` and `next`

#### Scenario: A hook runs before the hooks of its fields
- **WHEN** with that `Noisy`, `struct Pair { val first: Noisy, val second: Noisy }` and `operator mut fun Pair.drop() { println("drop pair of " + first.name) }` are declared and `fun main() { val p = Pair(Noisy("1"), Noisy("2")) }` runs with `--check-allocations`
- **THEN** it prints `drop pair of 1`, `drop 1` and `drop 2`

#### Scenario: An enum's hook runs before its payloads
- **WHEN** with that `Noisy`, `enum Slot { Full(Noisy), Empty }` and `operator mut fun Slot.drop() { println("drop slot") }` are declared and `fun main() { val a = Slot.Full(Noisy("x")); val b = Slot.Empty }` runs with `--check-allocations`
- **THEN** it prints `drop slot`, `drop slot` and `drop x`

#### Scenario: A hook may change its value before its parts are dropped
- **WHEN** with that `Noisy`, `struct Guard { var inner: Noisy }` and `operator mut fun Guard.drop() { inner = Noisy("replaced") }` are declared and `fun main() { val g = Guard(Noisy("first")) }` runs with `--check-allocations`
- **THEN** it prints `drop first` and `drop replaced`, and exits with status 0

### Requirement: Moved Values Run No Drop Hook
A value moved out of its owner MUST NOT run its hook where the owner's scope ends; it MUST run it where its last owner drops it. A value moved on some paths only MUST run its hook at its scope end exactly on the paths where it was not moved. An assignment MUST run the old value's hook before the new value is written, and only when an old value is present.

#### Scenario: A value moved into a function runs its hook there
- **WHEN** with the `Noisy` of "Locals run their hooks in reverse order", `fun consume(n: Noisy) { println("consume " + n.name) }` is declared and `fun main() { val n = Noisy("a"); consume(n); println("after") }` runs with `--check-allocations`
- **THEN** it prints `consume a`, `drop a` and `after`

#### Scenario: A conditionally moved value runs its hook only if present
- **WHEN** with that `Noisy` and `consume`, `fun f(c: Bool) { val n = Noisy("x"); if (c) consume(n); println("end f") }` is declared and `fun main() { f(true); f(false) }` runs with `--check-allocations`
- **THEN** it prints `consume x`, `drop x`, `end f`, `end f` and `drop x`

#### Scenario: Assigning runs the old value's hook
- **WHEN** with that `Noisy` and `consume`, `fun main() { var n = Noisy("a"); n = Noisy("b"); consume(n); n = Noisy("c"); println("end") }` runs with `--check-allocations`
- **THEN** it prints `drop a`, `consume b`, `drop b`, `end` and `drop c`: assigning `c` over the moved `n` runs no hook

#### Scenario: A returned value runs its hook in the caller
- **WHEN** with that `Noisy`, `fun make(): Noisy { val n = Noisy("m"); println("made"); return n }` is declared and `fun main() { val m = make(); println("got") }` runs with `--check-allocations`
- **THEN** it prints `made`, `got` and `drop m`

### Requirement: Drop Hooks Run Wherever Values Are Dropped
A value with a drop hook MUST run it exactly once when it is dropped as a field or payload of a value without a hook, as a nullable's payload, a list element (in index order when the list is dropped, and when an element is overwritten), a box's contents, a capture a closure owns, or a type-parameter value of a generic instance. Unboxing or removing an element MUST move the value without running its hook.

#### Scenario: List elements run their hooks
- **WHEN** with the `Noisy` of "Locals run their hooks in reverse order", `fun main() { var xs = listOf(Noisy("1"), Noisy("2"), Noisy("3")); xs[0] = Noisy("4"); val second = xs.removeAt(1); println("end") }` runs with `--check-allocations`
- **THEN** it prints `drop 1`, `end`, `drop 2`, `drop 4` and `drop 3`

#### Scenario: Box contents and fields run their hooks
- **WHEN** with that `Noisy`, `struct Holder { val n: Noisy, val tag: Int }` is declared and `fun main() { val h = Heap(Noisy("boxed")); val o: Noisy? = Noisy("maybe"); val none: Noisy? = null; val held = Holder(Noisy("held"), 1); val out = Heap(Noisy("out")).unbox(); println("end") }` runs with `--check-allocations`
- **THEN** it prints `end`, `drop out`, `drop held`, `drop maybe` and `drop boxed`

#### Scenario: A closure's owned capture runs its hook with the closure
- **WHEN** with that `Noisy`, `fun keep(): () -> Unit { val n = Noisy("c"); return { println(n.name) } }` is declared and `fun main() { val f = keep(); f(); println("end") }` runs with `--check-allocations`
- **THEN** it prints `c`, `end` and `drop c`

#### Scenario: An instance runs the hook of its type argument
- **WHEN** with that `Noisy`, `fun <T> discard(x: T) {}` and `fun <T> pick(a: T, b: T, first: Bool): T = if (first) a else b` are declared and `fun main() { discard(Noisy("g")); val p = pick(Noisy("a"), Noisy("b"), true); println("end") }` runs with `--check-allocations`
- **THEN** it prints `drop g`, `drop b`, `end` and `drop a`

### Requirement: A Closure Keeps Only What It Captures by Value
A closure MUST keep, and drop with its environment, only the values it captures by value. A binding of which an owning closure copies only `Copy` paths MUST stay owned by its scope, and MUST run its hook where that scope ends, even while the closure lives on.

#### Scenario: A binding whose field a closure copies is dropped by its scope
- **WHEN** `struct Ticket { val id: Int }` and `operator mut fun Ticket.drop() { println("drop ticket") }` are declared, `fun make(t: Ticket): () -> Int = { t.id }` is declared and `fun main() { val f = make(Ticket(7)); println("made"); println(f()) }` runs with `--check-allocations`
- **THEN** it prints `drop ticket`, `made` and `7`: the closure copied `t.id`, so `t` was dropped when `make` returned

### Requirement: Clone Hooks Run Wherever Values Are Cloned
`x.clone()` of a type with a clone hook MUST call the hook with a shared borrow of `x` and give its result. Cloning a value that holds such a type as a field, payload, nullable payload, list element or box contents MUST clone that part by its hook, in the order the parts are cloned, and an instance's `clone()` of a type-parameter value MUST call the type argument's hook.

#### Scenario: Cloning through a hook
- **WHEN** `struct Ticket : Clone { val id: Int }` and `operator fun Ticket.clone(): Ticket { println("clone " + id.toString()); return Ticket(id + 100) }` are declared and `fun main() { val a = Ticket(1); val b = a.clone(); println(b.id); println(a.id) }` runs with `--check-allocations`
- **THEN** it prints `clone 1`, `101` and `1`

#### Scenario: Parts are cloned by their hooks
- **WHEN** with that `Ticket`, `struct Pass : Clone { val ticket: Ticket, val owner: String }` and `fun <T : Clone> copyOf(x: &T): T = x.clone()` are declared and `fun main() { val p = Pass(Ticket(2), "ada").clone(); println(p.ticket.id); val xs = listOf(Ticket(3), Ticket(4)).clone(); println(xs[1].id); val h = Heap(Ticket(5)).clone(); println(h.id); println(copyOf(&Ticket(6)).id) }` runs with `--check-allocations`
- **THEN** it prints `clone 2`, `102`, `clone 3`, `clone 4`, `104`, `clone 5`, `105`, `clone 6` and `106`

### Requirement: Drop Hooks Release Runtime Resources
A type whose value holds a handle to a runtime resource MUST be able to release it in its drop hook and acquire a new one in its clone hook, so that under `--check-allocations` a program using it reports no live resource, whether its values end in locals, lists, boxes, fields or clones.

#### Scenario: A session type disposes its composition
- **WHEN** a file with `import /prism/composer` declares `struct Session : Clone { val id: Int }`, `fun open(): Session = Session(startComposition { k -> })`, `operator mut fun Session.drop() { disposeComposition(id) }` and `operator fun Session.clone(): Session = open()`, and `fun main() { val a = open(); val b = a.clone(); val all = listOf(open(), open()); val boxed = Heap(open()); println("done") }` runs with `--check-allocations`
- **THEN** it prints `done`, writes nothing to stderr and exits with status 0

#### Scenario: Without the hook the session leaks its composition
- **WHEN** the same file declares no drop or clone hook and no `: Clone` marker, and `fun main() { val a = open(); println("done") }` runs with `--check-allocations`
- **THEN** it prints `done`, stderr is exactly `allocation check: compositions still live at exit: 1` and a newline, and the exit status is 102

### Requirement: A Runtime Error in a Hook Ends the Program
A runtime error raised in a hook MUST end the program as any runtime error does (status 101, located in the hook), and no further drop or hook MUST run. Ending the program with `exitProcess` MUST run no hook.

#### Scenario: A hook that divides by zero
- **WHEN** with the `Noisy` of "Locals run their hooks in reverse order", `fun zero(): Int = 0`, `struct Bomb { val n: Int }` and `operator mut fun Bomb.drop() { println(n / zero()) }` are declared and `fun main() { val a = Noisy("a"); val b = Bomb(1); println("end") }` runs
- **THEN** stdout is `end`, stderr holds `panic: division by zero` located at `n / zero()` in the hook, and the exit status is 101: `drop a` is never printed

#### Scenario: Exiting runs no hook
- **WHEN** with that `Noisy`, `fun main() { val n = Noisy("n"); println("bye"); exitProcess(3) }` runs with `import /prism/system`
- **THEN** stdout is `bye` and the exit status is 3

### Requirement: The Allocation Check Drops Top-Level Values
Under `--check-allocations`, when `main` returns, the check MUST drop each initialized top-level value in reverse order of initialization, running its hooks, before it reports, and drop in turn any value initialized meanwhile. Reading a top-level value it dropped MUST raise the runtime error `top-level value '<name>' read after the allocation check dropped it`. Without the option, no top-level value is dropped and no hook of one runs.

#### Scenario: A top-level value's hook runs under the check
- **WHEN** with the `Noisy` of "Locals run their hooks in reverse order", `val GLOBAL: Noisy = Noisy("global")` and `fun main() { println(GLOBAL.name) }` run with `--check-allocations`
- **THEN** it prints `global` and `drop global`, and exits with status 0

#### Scenario: Without the check a top-level value runs no hook
- **WHEN** the same program runs without `--check-allocations`
- **THEN** it prints only `global` and exits with status 0

#### Scenario: A hook reading a top-level value the check dropped
- **WHEN** with that `Noisy`, `struct Reader { val n: Int }`, `operator mut fun Reader.drop() { println(A.name) }`, `val B: Reader = Reader(1)`, `val A: Noisy = Noisy("a")` and `fun main() { println(B.n); println(A.name) }` run with `--check-allocations`
- **THEN** stdout holds `1`, `a` and `drop a`, stderr holds `panic: top-level value 'A' read after the allocation check dropped it` located at `A.name` in the hook, and the exit status is 101

### Requirement: Nullable Type Arguments Flatten at Run Time
An instance whose type argument for `T` is nullable MUST treat `T?` as that argument: a `T` wrapped into a `T?` is the same value, `null` included, a `T?` compared with `null` tests the argument's own tag, and `?:` on a `T` or a `T?` takes its fallback exactly when the value is `null`.

#### Scenario: Wrapping a nullable argument keeps null
- **WHEN** `fun <T> wrap(a: T): T? = a` and `fun <T> isNull(a: T?): Bool = a == null` are declared and `fun main() { println(isNull(wrap<Int?>(null))); println(isNull(wrap<Int?>(1))); println(isNull<Int>(null)); println(wrap<String?>("s") ?: "none") }` runs
- **THEN** the program prints `true`, `false`, `true` and `s`

### Requirement: Generic Default Values Run Per Instance
A default value of a generic function MUST be evaluated by an instance of its generator for the call's type arguments, with the earlier arguments the call passed.

#### Scenario: A default value of a generic function
- **WHEN** `fun <T : Copy> pad(x: T, y: T = x): T = y` is declared and `fun main() { println(pad(1)); println(pad(true, false)); println(pad('c')) }` runs
- **THEN** the program prints `1`, `false` and `c`

### Requirement: Generic Recursive Data Runs
Generic structs and enums that reach themselves through `Heap<T>` with the same type arguments MUST run through generic functions and extensions, with every box freed exactly once.

#### Scenario: A generic linked list
- **WHEN** `enum Chain<T> { Cons(T, Heap<Chain<T>>), Nil }`, `fun <T> Chain<T>.size(): Int = when (this) { Chain.Cons(_, val rest) -> 1 + rest.size(), Chain.Nil -> 0 }` and `fun <T> push(list: Chain<T>, x: T): Chain<T> = Chain.Cons(x, Heap(list))` are declared and `fun main() { val empty: Chain<String> = Chain.Nil; val names = push(push(empty, "a"), "b"); val none: Chain<Int> = Chain.Nil; val numbers = push(none, 1); println(names.size()); println(numbers.size()) }` runs under `prism run --check-allocations`
- **THEN** the program prints `2` and `1`, reports no live allocation and exits with status 0

### Requirement: Runtime Errors in an Instance Name the Generic's Source
A runtime error raised by an instance MUST be reported at the location in the generic's own file that raised it, whichever file called the instance.

#### Scenario: Division by zero in an imported generic
- **WHEN** `lib/math.prism` declares `public fun <T> ratio(x: T, a: Int, b: Int): Int = a / b` on line 2, with `a / b` starting at column 51, and `app/main.prism` imports `/lib/math` and declares `fun main() { println(ratio("x", 1, 0)) }`
- **THEN** stderr is exactly `panic: division by zero` and `  at lib/math.prism:2:51`, each followed by a newline, and the exit status is 101

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

### Requirement: Closures Use the Paths They Capture
A closure that borrows a captured path MUST read and write that place where it lives each time it is called, a closure that copies a captured path MUST read the value the path held when the closure was built, and one that captures a state path weakly MUST reach the state the path held then. None MUST take the path's binding, which its owner frees exactly once.

#### Scenario: A closure writes one field while another is written
- **WHEN** `struct P { var x: Int, var y: Int }` is declared and `fun main() { var p = P(1, 2); val bump = { p.x += 10 }; p.y += 1; bump(); bump(); println(p.x); println(p.y) }` runs with `--check-allocations`
- **THEN** stdout is `21` and then `3`, and the exit status is 0

#### Scenario: A copied path keeps the value it was built with
- **WHEN** `struct P { val x: Int }` and `fun make(p: &P): () -> Int = { p.x * 10 }` are declared and `fun main() { var p = P(1); val f = make(p); p = P(2); println(f()); println(p.x) }` runs with `--check-allocations`
- **THEN** stdout is `10` and then `2`, and the exit status is 0

#### Scenario: A closure copying a field leaves its binding to its owner
- **WHEN** `struct User { val id: Int, val name: String }` and `struct Button { val onClick: () -> Int }` are declared and `fun main() { val u = User(7, "a" + "b"); val b = Button({ u.id * 2 }); println(b.onClick()); println(u.name) }` runs with `--check-allocations`
- **THEN** stdout is `14` and then `ab`, no live allocation is reported, and the exit status is 0

#### Scenario: A handler writes a state field of a borrowed item
- **WHEN** `struct Todo { val id: Int, val done: MutableState<Bool> }`, `fun deliver(v: Bool, f: attached (Bool) -> Unit) { f(v) }` and `fun toggle(todo: &Todo) { deliver(true, { todo.done.value = it }) }` are declared and `fun main() { val t = Todo(1, mutableStateOf(false)); toggle(t); println(t.done.value) }` runs with `--check-allocations`
- **THEN** stdout is `true`, and the exit status is 0: the handler holds a weak handle of `todo.done`, and `t` frees the state once

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

### Requirement: Closures That Own Their Captures Keep Them
A closure that owns its captures MUST keep the values it captured when it was built for as long as it lives, and each call MUST read those values, however many times it is called, wherever it is stored and after the function that built it has returned.

#### Scenario: A returned closure keeps what it captured
- **WHEN** `fun greeter(name: String): (String) -> String = { greeting -> greeting + ", " + name }` and `fun main() { val hello = greeter("Ada"); println(hello("Hello")); println(hello("Bye")) }` run with `--check-allocations`
- **THEN** it prints `Hello, Ada` and then `Bye, Ada`, reports no live allocation and exits with status 0

#### Scenario: A handler stored in a struct
- **WHEN** `struct Button { val label: String, val onClick: () -> Unit }`, `fun button(label: String, onClick: escaping () -> Unit): Button = Button(label, onClick)`, `fun click(b: &Button) { b.onClick() }` and `fun main() { val name = "Ada"; val b = button("Greet") { println("Hi " + name) }; click(b); click(b); println(b.label) }` run with `--check-allocations`
- **THEN** it prints `Hi Ada` twice and then `Greet`, reports no live allocation and exits with status 0

#### Scenario: A closure owning another closure
- **WHEN** `fun make(s: String): () -> Int = { s.length }`, `fun twice(g: escaping () -> Int): () -> Int = { g() * 2 }` and `fun main() { val h = twice(make("abc")); println(h()); println(h()) }` run with `--check-allocations`
- **THEN** it prints `6` twice, reports no live allocation and exits with status 0

#### Scenario: An owning closure in a default value
- **WHEN** `fun pick(a: Int, f: () -> Int = { a * 2 }): Int = f()` and `fun main() { println(pick(4)); println(pick(4) { 1 }) }` run with `--check-allocations`
- **THEN** it prints `8` and then `1`, reports no live allocation and exits with status 0

#### Scenario: Closures in generic and boxed values
- **WHEN** `struct Box<T>(val v: T)`, `fun <T> keep(x: T): T = x` and `fun main() { val s = "abc"; val b = Box(keep({ s.length })); println(b.v()); val t = "de"; val h = Heap({ t.length }); println(1) }` run with `--check-allocations`
- **THEN** it prints `3` and then `1`, reports no live allocation and exits with status 0

### Requirement: Owned Environments Are Freed Exactly Once
An owned environment MUST be freed exactly once, when the function value that owns it is dropped: at the end of its owner's scope, when the variable holding it is assigned another value, or with the struct, enum, box or nullable holding it, and its owned captures MUST be dropped with it.

#### Scenario: Reassigning a variable frees the old environment
- **WHEN** `fun make(s: String): () -> Int = { s.length }` and `fun main() { var h = make("a" + "b"); println(h()); h = make("cde"); println(h()) }` run with `--check-allocations`
- **THEN** it prints `2` and then `3`, reports no live allocation and exits with status 0

#### Scenario: A closure kept past its binding's block
- **WHEN** `fun main() { var h: () -> Int = { 0 }; var i = 0; while (i < 3) { val s = "a" + i.toString(); h = { s.length + 10 }; i += 1 }; println(h()) }` runs with `--check-allocations`
- **THEN** it prints `12`, reports no live allocation and exits with status 0

#### Scenario: A dropped struct frees its handler
- **WHEN** `struct Item { val name: String, val onSelect: () -> Int }` and `fun item(name: String): Item { val n = name.clone(); return Item(name, { n.length }) }` are declared and `fun main() { val a = item("x" + "yz"); println(a.onSelect()); val b = item("w"); println(b.name) }` run with `--check-allocations`
- **THEN** it prints `3` and then `w`, reports no live allocation and exits with status 0

### Requirement: Lists Hold Their Elements in Order
`listOf(e1, ..., en)` MUST evaluate its arguments left to right and give a list holding them in that order. `xs.size` MUST be the number of elements, `xs.indices` the range `0..<xs.size`, `isEmpty()` whether the size is 0 and `isNotEmpty()` its negation. `xs[i]` MUST read or write the element at index `i`, counting from 0, and an assignment to it MUST drop the old element.

#### Scenario: Reading a list
- **WHEN** `fun main() { val xs = listOf(5, 6, 7); println(xs.size); println(xs[2]); println(xs.isNotEmpty()); for (i in xs.indices) print(i); println() }` runs
- **THEN** it prints `3`, `7`, `true` and `012`

### Requirement: Mutating Members Keep the Other Elements in Order
`add(e)` MUST append `e`; `insert(i, e)` MUST place `e` at index `i`, moving the elements from `i` on one index up; `removeAt(i)` MUST give the element at `i`, moving the later elements one index down; `removeLast()` MUST give the last element; `reverse()` MUST reverse the order of the elements; and `clear()` MUST drop every element, leaving the list empty. Each MUST keep the relative order of the elements it does not move.

#### Scenario: Building and changing a list
- **WHEN** `fun show(xs: &List<Int>) { for (x in xs) print(x); println() }` is declared and `fun main() { var xs = listOf(1, 2, 3); xs.add(4); xs.insert(0, 0); show(xs); println(xs.removeAt(2)); println(xs.removeLast()); xs[0] = 9; xs[1] += 10; show(xs); xs.reverse(); show(xs); println(xs.size); xs.clear(); println(xs.isEmpty()) }` runs with `--check-allocations`
- **THEN** it prints `01234`, `2`, `4`, `9113`, `3119`, `3` and `true`, writes nothing to stderr and exits with status 0

#### Scenario: Lists of strings
- **WHEN** `fun main() { var names = listOf("a" + "da", "bob".clone()); names.add(3.toString()); names[1] = "eve".clone(); val first = names.removeAt(0); println(first); for (n in names) println(n); println(names[0].length) }` runs with `--check-allocations`
- **THEN** it prints `ada`, `eve`, `3` and `3`, writes nothing to stderr and exits with status 0

#### Scenario: Lists in structs and nested lists
- **WHEN** `struct Todo { val title: String, var done: Bool }` and `struct Model { var todos: List<Todo> }` are declared and `fun main() { var m = Model(listOf()); m.todos.add(Todo("write".clone(), false)); m.todos.add(Todo("test".clone(), false)); m.todos[1].done = true; for (t in m.todos) if (t.done) println(t.title); var grid = listOf(listOf(1), listOf<Int>()); grid[1].add(2); grid[0][0] = 3; println(grid[0][0] + grid[1][0]) }` runs with `--check-allocations`
- **THEN** it prints `test` and `5`, writes nothing to stderr and exits with status 0

### Requirement: List Indices Are Checked
An element access, `removeAt(i)` or `insert(i, e)` whose index is outside `0..<size` (for `insert`, outside `0..size`) MUST end the program with the runtime error "list index <i> out of range for size <n>", located at the access or the call, after the operands of the access or the call are evaluated and before the list changes. `removeLast()` on an empty list MUST end it with the runtime error "removeLast() on an empty list".

#### Scenario: A list index out of range
- **WHEN** `fun main() { val xs = listOf(1, 2, 3); println(xs[0]); println(xs[3]) }` runs
- **THEN** stdout holds `1` and a newline, stderr holds `panic: list index 3 out of range for size 3` and the location of `xs[3]`, and the exit status is 101

#### Scenario: Negative indices and bad removals
- **WHEN** `fun main() { var xs = listOf(1); xs.removeAt(-1) }`, `fun main() { var xs = listOf(1); xs.insert(2, 5) }` or `fun main() { var xs = listOf<Int>(); xs.removeLast() }` runs
- **THEN** stderr holds `panic: list index -1 out of range for size 1`, `panic: list index 2 out of range for size 1` or `panic: removeLast() on an empty list` respectively, with the call's location, and the exit status is 101

### Requirement: Iterating a List Visits Each Element in Order
A `for` loop over a list MUST run its body once per element, in index order, with the variable referring to that element; `break` MUST leave the loop and `continue` go to the next element. Writes through a variable of a loop over `&mut xs` MUST change the elements of `xs`. A temporary list a loop iterates MUST be dropped after the loop ends.

#### Scenario: Summing, skipping and stopping
- **WHEN** `fun main() { val xs = listOf(1, 2, 3, 4, 5); var sum = 0; for (x in xs) { if (x == 2) continue; if (x == 5) break; sum += x }; println(sum) }` runs
- **THEN** it prints `8`

#### Scenario: Updating elements in a loop
- **WHEN** `fun main() { var xs = listOf(1, 2, 3); for (x in &mut xs) { *x *= 10 }; for (x in xs) print(x); println() }` runs with `--check-allocations`
- **THEN** it prints `102030`, writes nothing to stderr and exits with status 0

#### Scenario: Iterating a temporary list of strings
- **WHEN** `fun make(): List<String> = listOf(1.toString(), 2.toString())` is declared and `fun main() { for (s in make()) println(s) }` runs with `--check-allocations`
- **THEN** it prints `1` and `2`, writes nothing to stderr and exits with status 0

### Requirement: Lists Compare and Search by Element Equality
Two lists MUST be equal when they have the same size and their elements at each index are equal by `==` of the element type, and `!=` MUST be its negation. `indexOf(e)` MUST give the smallest index whose element equals `e`, or -1 when none does, and `contains(e)` whether there is one.

#### Scenario: Comparing and searching
- **WHEN** `struct P : Copy { val x: Int }` is declared and `fun main() { val a = listOf("a".clone(), "b".clone()); val b = listOf("a", "b"); println(a == b); println(a != listOf("a")); println(listOf(P(1), P(2)).indexOf(P(2))); println(a.contains("c")); println(listOf(listOf(1)) == listOf(listOf(1))) }` runs with `--check-allocations`
- **THEN** it prints `true`, `true`, `1`, `false` and `true`, writes nothing to stderr and exits with status 0

### Requirement: Lists Free Their Elements Exactly Once
Dropping a list MUST drop each of its elements once and free its buffer. Cloning a list MUST give a list of clones of its elements that shares no memory with the original, so that changing or dropping either leaves the other unchanged. Elements removed by `removeAt` or `removeLast` MUST be owned by the caller, and elements overwritten or cleared MUST be dropped when they are.

#### Scenario: Cloning a list is deep
- **WHEN** `fun main() { var a = listOf(listOf(1.toString())); val b = a.clone(); a[0][0] = 2.toString(); a[0].add(3.toString()); println(b[0][0]); println(b[0].size); println(a[0].size) }` runs with `--check-allocations`
- **THEN** it prints `1`, `1` and `2`, writes nothing to stderr and exits with status 0

#### Scenario: A tree of nodes with child lists
- **WHEN** `struct Node { val name: String, var children: List<Node> }` and `fun Node.count(): Int { var n = 1; for (c in children) n += c.count(); return n }` are declared and `fun main() { var root = Node("root".clone(), listOf()); root.children.add(Node("a".clone(), listOf(Node("b".clone(), listOf())))); root.children.add(Node("c".clone(), listOf())); println(root.count()) }` runs with `--check-allocations`
- **THEN** it prints `4`, writes nothing to stderr and exits with status 0

#### Scenario: Lists in generic code and top-level values
- **WHEN** `val primes = listOf(2, 3, 5)`, `fun <T> lastOf(xs: &List<T>): &T = &xs[xs.size - 1]` and `fun <T> pushAll(into: &mut List<T>, from: List<T>) { var source = from; source.reverse(); while (source.isNotEmpty()) into.add(source.removeLast()) }` are declared and `fun main() { var names = listOf("a".clone()); pushAll(&mut names, listOf("b".clone(), "c".clone())); println(lastOf(names)); println(lastOf(primes)); println(names.size) }` runs with `--check-allocations`
- **THEN** it prints `c`, `5` and `3`, writes nothing to stderr and exits with status 0

#### Scenario: A list of handlers
- **WHEN** `fun main() { var handlers = listOf<() -> Unit>(); handlers.add { println("one") }; handlers.add { println("two") }; for (h in handlers) h(); handlers.removeAt(0); handlers[0]() }` runs with `--check-allocations`
- **THEN** it prints `one`, `two` and `two`, writes nothing to stderr and exits with status 0

#### Scenario: Lists of enums and boxes
- **WHEN** `enum Shape { Circle(Int), Label(String) }` is declared and `fun main() { var shapes = listOf(Shape.Circle(1), Shape.Label("a".clone())); shapes.add(Shape.Label(2.toString())); var boxes = listOf(Heap(1), Heap(2)); boxes[0] = Heap(3); var total = 0; for (b in boxes) total += b; for (s in shapes) when (s) { Shape.Circle(val r) -> total += r; Shape.Label(val t) -> total += t.length }; println(total); println(shapes == listOf(Shape.Circle(1), Shape.Label("a"), Shape.Label("2"))) }` runs with `--check-allocations`
- **THEN** it prints `8` and `true`, writes nothing to stderr and exits with status 0

#### Scenario: Handlers that own their captures
- **WHEN** `fun makeHandler(label: String): () -> Unit = { println(label) }` is declared and `fun main() { var handlers = listOf<() -> Unit>(); for (i in 0..<3) handlers.add(makeHandler(i.toString())); val greeting = "hi".clone(); handlers.add { println(greeting) }; for (h in handlers) h(); handlers.clear(); println(handlers.size) }` runs with `--check-allocations`
- **THEN** it prints `0`, `1`, `2`, `hi` and `0`, writes nothing to stderr and exits with status 0, every closure's environment freed by `clear()`

### Requirement: Adding to a List Takes Amortized Constant Time
On a release build, `prism run` of a program that adds a million `Int`s to an empty list and then sums them MUST take under 20 ms of wall time, startup included, on a developer machine. A timing benchmark MUST report this time against the target; it MUST NOT fail a test run on the time alone.

#### Scenario: A million adds
- **WHEN** the ignored benchmark runs `prism run` on its program of a million `add`s on a release build
- **THEN** it reports the median and 90th percentile wall time over repeated runs against the 20 ms target, and fails on the time only when `PRISM_ASSERT_TIMING=1` is set

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

### Requirement: A Run From Cached Code Behaves as a Cold Run
A program whose units and instances are loaded from the build cache MUST behave exactly as the same program compiled afresh: the same output, the same exit status, the same runtime errors at the same source locations, and the same result of the allocation check.

#### Scenario: A runtime error in a cached instance
- **WHEN** `lib/math.prism` declares `public fun <T> divide(x: T, n: Int): Int = 10 / n`, `app/main.prism` imports `/lib/math` and declares `fun main() { println(divide("a", 0)) }`, and `prism run app/main.prism` runs twice with the cache enabled
- **THEN** both runs print the same `panic: division by zero` diagnostic located in `lib/math.prism` at `10 / n` to stderr and exit with status 101

#### Scenario: Every executable fixture runs warm as cold
- **WHEN** each executable fixture is run with `prism run --check-allocations` on an empty cache and then again on the cache that run filled
- **THEN** the second run matches the same expected output and status, and writes no new entry

### Requirement: Runtime Errors in Library Code
A runtime error raised by code of a library module MUST name its location by the module path followed by `.prism` (`prism/collections.prism`) and the line and column of the failing operation in the library's source. Runtime errors raised by an intrinsic MUST be located at the call in the calling file.

#### Scenario: An overflow inside a library helper
- **WHEN** `fun main() { val xs = listOf(9223372036854775807, 1); println(xs.sumOf { x -> x }) }` runs
- **THEN** stderr holds the integer overflow runtime error, located `  at prism/collections.prism:<line>:<column>` at the addition in `sumOf`'s body, and the exit status is 101

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

### Requirement: State Errors Are Runtime Errors at the Access
The runtime errors of state, `stale state handle`, `state accessed while borrowed`, `state written during composition` and `state created during composition outside remember`, MUST end the program as every runtime error does (execution, "Runtime Errors End the Program"), with the location of the access, write or creation that failed.

#### Scenario: Location of a stale access
- **WHEN** `app/main.prism` reads a stale handle with `println(kept.value)` on line 9, `kept.value` starting at column 13
- **THEN** stderr is exactly `panic: stale state handle` and `  at app/main.prism:9:13`, each followed by a newline, and the exit status is 101

### Requirement: Headless Runs
`prism run --headless[=<width>x<height>] [--events <script>] <file.prism>` MUST run the program with no window: `runApp` runs headless, in a window of the given size when one is given and of its own `width` and `height` otherwise. Without `--events`, `runApp` MUST compose and lay out its content and return. The options MUST come before the file, and `--events` without `--headless` MUST be a usage error with status 1.

#### Scenario: A headless run of the given size
- **WHEN** `prism run --headless=320x200 --events script main.prism` runs on `fun main() { runApp(width = 800, height = 600) { } }` with `import /prism/compose/ui`, and `script` holds `dump`
- **THEN** stdout is `window 320x200` and a newline, and the exit status is 0

#### Scenario: Events without headless mode
- **WHEN** `prism run --events script main.prism` runs
- **THEN** stderr holds the usage of `prism run`, nothing runs, and the exit status is 1

### Requirement: Event Scripts Are Parsed Before the Program Runs
`prism run` MUST read and parse the event script before it compiles or runs the program. A script it cannot read, or a line it cannot parse, MUST be reported on stderr as `Error: <script>:<line>: <message>`, with status 1 and without running the program.

#### Scenario: An unknown script command
- **WHEN** `prism run --headless --events events.txt main.prism` runs a program that prints before calling `runApp`, and `events.txt` holds `clik "+"` on its first line
- **THEN** stderr is exactly `Error: events.txt:1: unknown command 'clik'` and a newline, stdout is empty, and the exit status is 1

### Requirement: Screenshots of an App
`prism run --screenshot <file.png> <file.prism>`, in a build with the `window` feature, MUST write a PNG image of the app's final frame to `<file.png>` each time `runApp` returns, after the app's last event and before its composition is dropped. The image MUST be the final display list rasterized as a window draws it, at scale 1 in headless runs and at the window's scale factor otherwise, as 8-bit RGBA without premultiplied alpha. The option MUST come before the file.

#### Scenario: A headless screenshot
- **WHEN** `prism run --headless=40x30 --screenshot shot.png main.prism` runs on `fun main() { runApp { Box(modifier = Modifier.size(10.dp).background(Color.Red)) { } } }` with `import /prism/compose/ui`, `import /prism/compose/foundation` and `import /prism/compose/layout`
- **THEN** the exit status is 0, and `shot.png` is a 40 × 30 image whose pixels at columns 0 to 9 and rows 0 to 9 are `#FFFF0000` and whose other pixels are white

### Requirement: Screenshot Errors
`--screenshot` without a file name MUST be a usage error with status 1, and nothing MUST run. When the file cannot be written, the program MUST end with the runtime error `display error: cannot write the screenshot <file.png> (<reason>)`, without a location line and with status 101.

#### Scenario: A screenshot without a file name
- **WHEN** `prism run --screenshot` runs with nothing after the option
- **THEN** stderr holds the usage of `prism run`, nothing runs, and the exit status is 1

#### Scenario: A screenshot that cannot be written
- **WHEN** `prism run --headless --screenshot missing/shot.png main.prism` runs an app, and the directory `missing` does not exist
- **THEN** stderr ends with `panic: display error: cannot write the screenshot missing/shot.png (` followed by the platform's reason, `)` and a newline, and the exit status is 101

### Requirement: States Owned by Data Run
A state object held by a struct, a list element or a local MUST be created, read, written and borrowed in place like any value, and its state MUST be retired exactly once, when its owner drops it, and freed exactly once.

#### Scenario: Items holding their own states
- **WHEN** `struct Todo { val id: Int, val done: MutableState<Bool> }` is declared and `main` builds `val todos = mutableStateOf(listOf(Todo(1, mutableStateOf(false)), Todo(2, mutableStateOf(true))))`, runs `todos.value[0].done.value = true`, `val removed = todos.value.removeAt(1)`, `println(removed.done.value)`, `var left = 0`, `for (t in todos.value) { if (!t.done.value) { left += 1 } }` and `println(left)`, with `--check-allocations`
- **THEN** stdout is `true` and `0`, one per line, and the exit status is 0

### Requirement: Retired Cells Are Freed Before the Allocation Check
Under `--check-allocations`, when `main` returns, every retired cell MUST be freed, its value dropped, after the top-level values are released and before compositions still live are reported and the allocation check reports. Without the check, cells still retired at exit MUST NOT be freed.

#### Scenario: A state holding a composition
- **WHEN** `compose fun Empty() { }` and `fun main() { val s = mutableStateOf(composition { Empty() }) }` run with `--check-allocations`
- **THEN** the program exits with status 0: freeing the retired cell disposes the composition before live compositions are counted

#### Scenario: A top-level state
- **WHEN** `val Count: MutableState<Int> = mutableStateOf(1)` and `fun main() { Count.value += 1; println(Count.value) }` run with `--check-allocations`
- **THEN** stdout is `2`, and the exit status is 0

### Requirement: Operator Functions Run as Calls
A program using arithmetic, unary, ordering, membership or `invoke` operators on types with operator functions MUST run each operator as a call of its function, with the result the function returns, and free every value it builds exactly once.

#### Scenario: Vector arithmetic runs
- **WHEN** a program declares `struct Vector2 : Copy { val x: Int, val y: Int }` with `operator` functions `plus`, `minus`, `times(k: Int)` and `unaryMinus` computing componentwise, and `fun Vector2.toString(): String = "($x, $y)"`, and prints `Vector2(1, 2) + Vector2(3, 4)`, `-Vector2(1, 2) * 3` and `Vector2(5, 5) - Vector2(1, 2)`
- **THEN** `prism run --check-allocations` prints `(4, 6)`, `(-3, -6)` and `(4, 3)` and exits with status 0

#### Scenario: Ordering, membership and invoke run
- **WHEN** a program declares a `Version` with `operator fun Version.compareTo(other: &Version): Int`, a `Tags` holding a `List<String>` with `operator fun Tags.contains(name: &String): Bool`, and a `Counter` with `operator mut fun Counter.invoke(step: Int = 1): Int`, and prints `Version(1, 2) < Version(1, 10)`, `Version(2, 0) <= Version(1, 9)`, `"b" in Tags(listOf("a", "b"))`, `"c" !in Tags(listOf("a"))`, `3 in 0..<3` and, for `var c = Counter(0)`, `c()` then `c(step = 5)`
- **THEN** `prism run --check-allocations` prints `true`, `false`, `true`, `true`, `false`, `1` and `6` and exits with status 0

#### Scenario: Compound assignment through an operator function runs
- **WHEN** a program declares a `struct Text { val s: String }` with `operator consuming fun Text.plus(other: &Text): Text = Text(s + other.s)` and runs `var t = Text("a"); for (part in listOf("b", "c")) { t += Text(part.clone()) }; println(t.s)`
- **THEN** `prism run --check-allocations` prints `abc`, frees each intermediate `Text` once and exits with status 0

### Requirement: Operator Operands Run Left to Right
The operands of an operator use MUST be evaluated left to right, whichever operand is the function's receiver, and each exactly once, also for the indices of a compound assignment through `get` and `set`.

#### Scenario: Evaluation order of operands
- **WHEN** a program declares `fun noisy(label: String, n: Int): Int { println(label); return n }`, a `Tags` with a `contains` operator, a `fun tags(): Tags` printing `tags`, a `Matrix` with `get` and `set` operators, and evaluates `noisy("left", 1) in tags()`, then `m[noisy("row", 0), noisy("column", 1)] += 2.0` on a `var m` holding one row of two cells
- **THEN** `prism run --check-allocations` prints `left` before `tags`, then `row` and `column` once each, and the cell holds its old value plus `2.0`

### Requirement: Index Operators Run Their Accessors
Reading, borrowing and writing `a[...]` on a type with index operators MUST run `get`, the `mut` `get` or `set` as `operators` says, reading and writing the element the accessor reaches, and dropping a replaced element exactly once.

#### Scenario: A matrix read and written by index
- **WHEN** a program declares `struct Matrix { val columns: Int, var cells: List<Double> }` with `operator fun Matrix.get(row: Int, column: Int): Double` and `operator mut fun Matrix.set(row: Int, column: Int, value: Double)` over `cells[row * columns + column]`, and runs `var m = Matrix(2, listOf(0.0, 0.0, 0.0, 0.0)); m[1, 0] = 2.5; m[1, 0] += 1.0; m[0, 1] = m[1, 0] * 2.0; println(m[1, 0]); println(m[0, 1]); println(m[0, 0])`
- **THEN** `prism run --check-allocations` prints `3.5`, `7.0` and `0.0` and exits with status 0

#### Scenario: Elements changed in place through the mutable get
- **WHEN** a program declares `struct Item { var name: String, var count: Int }` and `struct Bag { var items: List<Item> }` with a shared and a `mut` `get` borrowing `items[index]`, and runs `var b = Bag(listOf(Item("a", 0), Item("b", 0))); b[0].count += 2; b[1].name = "c"; b[0] = Item("d", 5); val first: &Item = &b[0]; println(first.name); println(b[0].count); println(b[1].name)`
- **THEN** `prism run --check-allocations` prints `d`, `5` and `c`, frees the replaced `Item("a", 2)` and the string `"b"` once each, and exits with status 0

### Requirement: Cursor Loops Run the Protocol
A `for` loop over a type with the cursor protocol MUST call `iterator` once, then `hasNext` before each iteration and `next` once per iteration, running the body with the element `next` returned, honour `break` and `continue`, and drop the cursor, the elements `next` produced by value, and a temporary iterable exactly once.

#### Scenario: Iterating a custom collection runs
- **WHEN** a program declares a `Bag` of `Item(name: String)` with the cursor protocol over an `Int` index (shared `next` returning `&Item`, `mut` `next` returning `&mut Item`), and runs `var b = Bag(listOf(Item("a"), Item("b"), Item("c"))); for (item in &mut b) { item.name += "!" }; for (item in b) { if (item.name == "b!") continue; println(item.name) }`
- **THEN** `prism run --check-allocations` prints `a!` and `c!` and exits with status 0

#### Scenario: A cursor producing owned values runs
- **WHEN** a program declares `struct Countdown : Copy { val from: Int }` whose protocol's `next` returns `n.toString()` for the cursor `n` and counts down to 1, and runs `for (s in Countdown(3)) { if (s == "1") break; println(s + "...") }`
- **THEN** `prism run --check-allocations` prints `3...` and `2...`, frees each string `next` returned once, the one the `break` left included, and exits with status 0

#### Scenario: Iterating a temporary collection runs
- **WHEN** a program declares the `Bag` protocol above and `fun make(): Bag = Bag(listOf(Item("x"), Item("y")))`, and runs `for (item in make()) println(item.name)`
- **THEN** `prism run --check-allocations` prints `x` and `y`, frees the temporary `Bag` after the loop, and exits with status 0

### Requirement: Equality Operators Run Wherever Values Are Compared
A program comparing values of a type with an `equals` operator MUST get that function's answer from `==` and `!=` on the values, on structs and enums holding them, on nullables and boxes of them, and from list equality, `contains`, `indexOf` and `in`.

#### Scenario: Equality through an equals operator runs
- **WHEN** a program declares `struct User { val id: Int, val name: String }` with `operator fun User.equals(other: &User): Bool = id == other.id` and `struct Team { val lead: User, val size: Int }`, and prints `User(1, "a") == User(1, "b")`, `User(1, "a") != User(2, "a")`, `Team(User(1, "a"), 3) == Team(User(1, "z"), 3)`, `Heap(User(2, "a")) == Heap(User(2, "b"))`, a `User?` holding `User(1, "a")` compared with `User(1, "c")`, `listOf(User(1, "a")) == listOf(User(1, "b"))`, `User(3, "q") in listOf(User(3, "r"))` and `listOf(User(1, "a"), User(2, "b")).indexOf(User(2, "x"))`
- **THEN** `prism run --check-allocations` prints `true`, `true`, `true`, `true`, `true`, `true`, `true` and `1`, and exits with status 0

## Design Rationale

Decisions from `codegen-core` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`Int` overflow traps in every tier** (D10). `+`, `-` and `*` use Cranelift's overflow-reporting forms and branch to a cold block calling `prism_rt_panic`; unary `-` and `/` check the smallest `Int`; `/` and `%` check for zero first; `%` by `-1` gives `0` without dividing. Each check is generated by the backend at the operator's span, and the `for` latch never overflows. This is Swift's rule, and Rust's in debug builds: a silent wrap turns an arithmetic bug into wrong data, and an `Int` that differed between Tier 1 and Tier 2 would make the release build a different program. Explicit wrapping operations can come later. No MIR `Assert` terminator: a Prism panic neither unwinds nor is evaluated at compile time. Rejected: wrapping (Kotlin, Java), Rust's tier-dependent split, trapping on `% -1`, hardware traps from `sdiv`.
- **`Double` is IEEE 754** (D11), with ordered comparisons except `!=`, and `%` as a runtime call (`prism_rt_double_remainder`, C's `fmod`), since Cranelift has no remainder instruction.
- **Runtime errors abort with status 101** (D12). `prism_rt_panic(kind, detail, file, line, column)` flushes the output, writes `panic: <message>` and `  at <file>:<line>:<column>` (the module path plus `.prism`, so workspace-relative and deterministic) and exits with 101, Rust's panic status, distinct from 1 (did not compile) and from signal statuses. There is no unwinding, so no landing pads, cleanup blocks or unwind tables; a `prism test` runner can run tests in subprocesses. No backtrace yet. Rejected: unwinding now, signal handlers, status 1 or 134.
- **The entry point is `fun main()` of the entry file** (D13): no type parameters, receiver or parameters, a `Unit` result, not `compose`. Missing is `E6002`, invalid (including a `val main`) `E6003`, both code-generation errors because `prism check` checks libraries. Rejected: `main(): Int` as the exit status (wait for a process library), `main(args)` (needs strings and lists), a `main` in any file.
- **`prism run [--timings] <file> [-- <argument>...]`** (D14). Arguments after `--` are recorded by the runtime for a future built-in, so scripts written now keep working; `--timings` prints the front end, code generation, linking and total time to `main`. Rejected: rejecting arguments until readable, arguments without `--`, an environment variable for timings.
- **Output built-ins print as Kotlin does** (D15, review). `print` and `println` call one runtime function per representation. A `Double` uses Rust's shortest round-trip digits laid out with Java's plain and scientific thresholds, and, where those digits are a single one, the closest decimal of one or two digits, as Java's `Double.toString` does (`4.9E-324`), which only differs for subnormals. Rejected: Rust's `1e10` and `inf`, `{}` (prints `1` for `1.0`).
- **Output is buffered by the runtime** (D16): written at 8 KiB, at the end, before a runtime error, and at each newline on a terminal. Rejected: a write per `print`, Rust's `LineWriter` (line-buffered into pipes too), C's `stdio`.
- **The runtime library** (D17) exports `RUNTIME_FUNCTIONS` (name, address, signature over a small `RuntimeType`), from which the codegen declares its imports and the JIT resolves them; its state is process-wide behind a `Mutex`, bracketed by `start` and `finish`. Rejected: formatting in generated code, libc's `printf`.
- **Top-level `val`s initialize lazily, through an accessor** (D18). Each has `#storage`, a `#state` byte (uninitialized, initializing, initialized), `#init` from its MIR body, and a generated `#get` returning the storage's address, or null when called during its own initialization, which the reader turns into the runtime error at the read's location. Rejected: eager initialization in dependency order (per-file units cannot see every initializer, and it runs initializers the program never needs), constant evaluation only (rejects initializers the type checker accepts), Kotlin's per-file initialization (a hidden coupling). A thread-safe once comes with threads.
- **The program runs on its own thread with an 8 MiB stack** (D19), so its stack does not depend on how `prism` was started or on the platform's main-thread size.
- **Stack overflow is a runtime error, detected on entry** (D27, from the review). Every compiled body compares the stack pointer with the runtime static `prism_rt_stack_limit`, which `Program::run` sets 256 KiB (`STACK_RESERVE`) above the end of the program thread's stack, and raises "stack overflow" at the body's declaration below it, so the output is flushed and the status is 101 rather than an abort from the guard page that loses buffered output. The limit is zero, disabling the check, outside a run. Rejected: a guard-page signal handler (platform-specific `sigaction`, a `libc` dependency, flushing from a signal handler), Cranelift's trapping `stack_limit` check (no message).
- **`prism run` reuses Phase 2 and bypasses the build cache** (D20): a cached verdict carries no MIR, and caching generated code is a follow-up. Errors stop at the first file with errors, as `prism check` does.
- **Executable fixtures** (D21). A fixture with `// run: <status>` is run as a `prism run` subprocess from its directory and compared with `stdout.snap` and `stderr.snap`, blessed with `PRISM_BLESS=1`; `E6xxx` codes in run stderr count for the catalog check, and every `execution` case is executable. Rejected: inline `// stdout:` lines, running in process (a panic exits the process).
- **The 15 ms target is reported, not asserted** (D22). An ignored release benchmark (`crates/prism-cli/tests/run_timing.rs`) runs `prism run --timings examples/run/main.prism` and prints the median and 90th percentile per stage, failing only with `PRISM_ASSERT_TIMING=1`; shared CI runners and debug builds would make an assertion flaky. Measured at implementation: a 1.8 ms median to `main` on Apple silicon.

Decisions from `codegen-aggregates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Copy and move are the same at run time** (D12): both copy the value's bytes; `Copy` only decides at check time whether the source stays usable, and no value has drop glue yet, so a move runs no code and leaves the source untouched. (With the glue of `runtime-strings-and-drops`, a move is still a copy of the bytes: the elaborated MIR drops only the value's new owner.)
- **Arguments and results are values** (D10): an argument is the value it had when evaluated, even if the callee writes through another argument to the same place, and a result assigned over its own argument (`p = mirror(p)`) is the value the callee built.
- **Structural equality** (D13): `==` over structs, enums and nullables compares fields, then variant and payloads, then presence and payload, each as its type does, so a struct holding `NaN` is not equal to itself, as Rust's derived equality; borrows compare their referents, never addresses.
- **`print` and `println` stay scalar** (D16): an aggregate or nullable argument is still `E4004`, because a text form for structs (Kotlin's data-class `toString`, or a user `toString`) belongs with a `Display`-like design once traits or overloading exist. Programs print fields, or `n ?: 0`.
- **`Range` is an ordinary `Copy` struct** (D1, D9): `<builtin>::Range(start, end, endInclusive)` of the MIR's type table, so it can be passed, returned, borrowed and stored at top level; a `for` loop reads it once before iterating.

Decisions from `runtime-strings-and-drops` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`length` and indexing count characters** (D4). `Char` is a Unicode scalar value and `s[i]` already had type `Char`, so `s.length` is the number of scalar values and `s[i]` the character at that index, as a Kotlin programmer expects of `length` and `[]`, with scalar values in place of Kotlin's UTF-16 code units (`"🚀".length` is 1). `length` reads the stored count; `s[i]` is O(1) on ASCII text and a scan otherwise, so a loop over `s[i]` on non-ASCII text is quadratic until a character iterator exists. Rejected: bytes (meaningless with a scalar `Char`), UTF-16 code units (a second representation, splits emoji), grapheme clusters (Unicode tables in the runtime, O(n) counts), iteration only (the language already has `s[i]`), a stored index of character offsets.
- **Which values are dropped, and when; top-level values never are** (D12). The elaborated MIR decides every drop, so a value's owned memory is freed where its owner's scope ends, where an assignment or a write through a reference overwrites it, at the end of the statement for a temporary, or by the code it was moved into. A top-level `val` lives until the program ends, like a Rust `static`: borrows of it are valid everywhere, and running glue at exit would cost time for memory the process gives back and, once destructors run user code, bring C++'s static destruction order problems. Rejected: dropping initialized top-level values at exit in reverse order.
- **Allocation checking with `prism run --check-allocations`** (D13). The runtime then keeps a map from address to size and alignment; without the option, allocation and free test one relaxed atomic flag. An invalid free (unknown address, double free, free of static text, another size or alignment) flushes the output, writes its message and exits with 102. When `main` returns, `finish` first releases the initialized top-level values that own memory (each accessor registers its storage and drop glue with `prism_rt_register_top_level_value` after its initializer returns; this runs no user code), then reports the allocations still live with 102. 102 is distinct from 0, 1 and 101, so a harness tells a compiler bug from a program error without reading stderr. The fixture runner runs every executable case with the option, so every program fixture asserts that it leaks nothing and frees nothing twice; this is also Phase 4's "no allocation outlives its owner" verification. Codegen tests read `prism_runtime::allocation_statistics()` to assert that literals and `Bool` text allocate nothing, and runtime tests run themselves as child processes to check the failures that end the process. Rejected: an environment variable (leaks into child processes), counting without the map (a double free and a leak cancel out), checking at every drop, always-on checking, status 101, counting top-level values as leaks.
- **Panics do not unwind, so values are not dropped on a panic** (D14). Confirmed from `codegen-core`: a runtime error flushes, reports and exits with 101 with no landing pads, and the operating system reclaims the memory, so `--check-allocations` does not check a program that panics. Unwinding for an in-process `prism test` would add cleanup edges to the MIR's calls, which elaboration and glue already support in shape.
- **`s += t` stays a concatenation** (D15). An in-place append needs a MIR form taking `&mut s` after `t` is evaluated, which turns the accepted `val r = &s; s += r` into a borrow conflict, or a post-borrow-checking rewrite of `tmp = &s + &t; drop s; s = move tmp` across blocks and drop flags. Neither is needed while UI code builds short strings, and Kotlin's `+=` also builds a new string; the capacity field leaves room for the append. A loop of `s += t` is quadratic, though fast in practice (100 000 one-character appends run in about 0.1 s in a debug build).
- **Runtime errors of strings and allocation** (D17). "string index <i> out of range for length <n>", negative indexes included, is reported at the indexing expression, whose location the runtime function receives; "out of memory" is reported without a location line, since an allocation can fail deep in the runtime, and passing a location to every allocating function would cost an argument everywhere for an error programs rarely see. Rejected: aborting as Rust does (the output would be lost), Kotlin's `index: 3, length: 3`.

Decisions from `payload-patterns` (planned autonomously on 2026-10-18, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Parts no binding takes are dropped where the branch ends** (D8). They are moved with the others into anonymous locals of the branch (of the enclosing block for a declaration), so no value is ever partly dropped and drop elaboration keeps working on whole locals. Once destructors exist they run after the branch body, where Rust runs them. Rejected: dropping unbound parts at once, which would run destructors before the body.

Decisions from `heap-boxes` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **Deep structures are a stack overflow, not a crash** (D14 in `code-generation`): dropping, cloning or comparing a structure deeper than the stack raises "stack overflow" with no location line, like "out of memory", and the program's output is kept.

Decisions from `string-templates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **Templates give the text `print` writes and free their temporaries by the end of the statement** (D11, D12 of `string-templates`). Scalars format exactly as `print` does (`1.0E7`, `NaN`, `-0.0`), parts run left to right, each once, and only a `toString()` part allocates a temporary of its own. Every template scenario is an executable fixture under `--check-allocations`.

Decisions from `monomorphization` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **An instance runs as the non-generic body of its substituted types would** (D6 in `mir`). A drop of a type-parameter value runs the argument's glue, or nothing when the argument owns nothing; a `clone()` under `T : Clone` runs the argument's clone glue; a `T : Copy` value is copied bitwise; and a `T?` with a nullable argument is that argument, so `null` stays `null` through any number of wraps and `?:` tests the argument's own tag (D7 in `generics`). No instance-specific run-time rule exists, so every tier inherits these from the substituted MIR.
- **Runtime errors name the generic's source** (D14 in `code-generation`): an instance's unit carries the generic's file name, so `panic: division by zero` in an imported generic is reported at the generic's file and position, whoever called it.

Decisions from `closures` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **Closures use the captured variables themselves** (D5): a borrowing closure's environment holds the captured places' addresses, so its writes are seen by the enclosing body and it sees the values the variables hold when it runs, as in Kotlin. Assigning a whole captured variable drops its old value through the capture, as any assignment does.
- **Function names and variant constructors behave as closures that call them** (D7), converting their arguments to the declared parameter types, so `each(show)` for `show(n: Int?)` runs as `each { n -> show(n) }`.
- **A function value is called through its references and boxes** (found in review): a payload bound from a borrowed enum, a boxed function value and a `&(() -> Int)` parameter are called in place. A callee that is not a name or path, such as `(opt ?: { 0 })()`, is a temporary that takes the value its `?:` or branch gives.
- **Closures in default values and top-level values** (D12): a default value's closures belong to its generator; a capture-free closure or a function name may initialize a top-level `val`, which is never dropped, which is fine since its environment is null.

Decisions from `escaping-closures` (planned autonomously on 2026-10-18, revised for compile speed on 2026-10-19, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Owned environments live as long as their function value** (D8, D17). Each is freed exactly once, when its owner is dropped, reassigned, or dropped with the struct, enum, box or nullable holding it, which every executable fixture checks under `--check-allocations` (the zero-cost memory safety invariant). A closure returned as a default value escapes, so `fun pick(a: Int, f: () -> Int = { a * 2 })` copies `a` and runs; top-level values are never captured, so a closure initializing a top-level `val` has a null environment.

Decisions from `collections` (planned autonomously on 2026-10-18, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Index errors happen after the operands and before any change** (D8). An element access, `insert` or `removeAt` checks its index after its operands are evaluated, so `xs[f()] = g()` runs `f` and `g` first, as Kotlin's `xs.set(f(), g())` does; runtime errors do not unwind, so no partially changed list is ever observed. `removeLast()` of an empty list has its own message, after Kotlin's `NoSuchElementException`.
- **An element's index is evaluated before the assigned value** (D21). The MIR evaluated an assignment's value before its target, which no earlier target could observe; for an element the target's index now comes first, matching Kotlin and the left-to-right order of calls.
- **A reported benchmark for adds** (D19). An ignored release test (`crates/prism-cli/tests/list_timing.rs`) reports the wall time of `prism run` on a million adds and their sum against 20 ms, failing only with `PRISM_ASSERT_TIMING=1`, like the startup benchmark: 4.2 ms median, 4.8 ms 90th percentile, startup included (Apple arm64, 2026-10-18). Asserting the time in `cargo test` would be flaky on shared machines.

Decisions from `hot-reload` (planned autonomously on 2026-10-17, implemented and reviewed on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **A runtime error under `--watch` stops the run, not the process** (D13). The error is reported as for `prism run`; then `panic.rs::stop` clears the stack limit, tells the session and parks the program thread forever, so no further code and no drop runs, as this spec requires, and the session waits for the next change, which runs `main` in a fresh program. A parked thread holds no runtime lock and keeps 8 MiB of reserved address space. Rejected: exiting the session (one bad edit would end it); unwinding (Prism has none).
- **Cache options under `--watch`** (D19). `--no-cache` keeps the whole session out of the build cache, with keys computed for the process alone; `--explain-cache` explains the first load only, since the `reload: applied` line already names what each reload defines.

Decisions from `compose-runtime` (planned autonomously on 2026-10-18, implemented and reviewed on 2026-10-18 and 2026-10-19, and logged in `docs/decisions-for-review.md`):

- **Composition misuse has no location** (D12). It is one runtime error kind, `composition misuse: <detail>`, raised by an entry point of the compose runtime, which has no caller location; passing one would cost a word per call for an error the generated code never produces.
- **Undisposed compositions fail the allocation check** (D13). A composition whose restart functions have inline environments holds no allocation of the program, so the leak report alone would miss it. `prism-runtime` calls a finish report, which `Program::run` installs from `prism-compose`, before its leak line; either line makes the status 102.

Decisions from `state-handles` (planned autonomously on 2026-10-18, reconciled with the archived compose changes and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **State errors are located runtime errors** (D14). `stale state handle`, `state accessed while borrowed`, `state written during composition` and `state created during composition outside remember` are `PanicKind`s after `CompositionMisuse`, raised at the access through a recorded kind and a cold block with its location, since unlike composition misuse ordinary programs reach them.
- **Program cells are freed first under the check** (D12). The finish report frees the program's cells, repeatedly, since a cell may hold a composition whose disposal frees cells, then reports live compositions, then allocations; without the check they are never freed, as top-level values are not.

Decisions from `ui-toolkit` (planned on 2026-10-18 and revised on 2026-10-19 by user direction to follow Jetpack Compose's APIs and modules, implemented and reviewed on 2026-10-19, and logged in `docs/decisions-for-review.md`):

- **Headless runs and event scripts** (D16). `--headless[=<width>x<height>]` runs `runApp` without a window, the size overriding `runApp`'s, and `--events <script>` drives it; `--events` alone is a usage error, since a program wanting a window would otherwise silently do nothing. The script is parsed before the program compiles, so a syntax error is `Error: <script>:<line>: <message>` with status 1 and nothing runs, rather than surfacing after the program's earlier output. Fixtures gain `// events:` and `// headless:` headers.

Decisions from `desktop-window` (planned autonomously on 2026-10-18, implemented and reviewed on 2026-10-19, and logged in `docs/decisions-for-review.md`):

- **Screenshots of the final frame** (D15). `--screenshot <file.png>` sets `ui-toolkit`'s final-frame sink: when an app ends, after its last event and before its composition is dropped, the final display list is rasterized on the program thread by the window's rasterizer, at scale 1 headless and at the window's scale factor otherwise, and written as 8-bit RGBA; every `runApp` overwrites the file. A headless screenshot is laid out with the fixed-advance metric but drawn with Inter's glyphs, so its text is narrower than its boxes, which is acceptable for a debugging aid. The option needs the `window` feature, which holds the rasterizer, and is unknown without it. Fixtures do not compare screenshots; a CLI test checks one's size and exact pixels. Rejected: a script command (it would change `ui-toolkit`'s grammar and put files into fixtures), reading back the presented surface (not portable, and rasterizing the same list gives the same pixels), always scale 1 (blurry on the display people use).

Decisions from `reactive-app-verification` (planned autonomously on 2026-10-18 and 2026-10-19, implemented and reviewed on 2026-10-19, and logged in `docs/decisions-for-review.md`). The report `--frame-stats` prints is `ui`'s.

- **Live runtime objects in the allocation check** (D6). Groups, slots and state cells live in `prism-compose`'s per-thread arenas and nodes in `prism-ui`'s, as Rust memory the allocation map never sees, so a release path that forgot an arena would pass the check. The finish report the program thread installs therefore runs, after the program's cells are released and the composition report, `live_objects_report` (groups and cells from their arenas, slots from the live groups and the release queue) and `live_nodes_report` (the current tree's nodes and those left in trees a later app replaced), one line per non-zero count. When a composition is live, what it holds is expected to be live, so only its line is printed. Counting costs nothing before exit: one read per arena then. No correct program prints these lines, so Rust tests leak through test hooks, and every UI fixture and session asserts their absence. Rejected: counting with a live composition (four more lines, no information), counting only nodes, routing arena records through the program allocator.
- **`--frame-stats` and a counting allocator** (D7). The `prism` binary's `#[global_allocator]` is `prism_runtime::allocation_counter::CountingAllocator`, the system allocator plus, once `--frame-stats` enables it before the program thread starts, per-thread allocation and free counts in `const` thread-locals that never allocate (a reallocation counts one of each). Library tests and other embedders keep their allocator. Off, an allocation costs one relaxed atomic load: `run_timing.rs` stayed within its noise, and at review a build without it ran the 1,000-item todo app's 100 toggles in 240.1 ms against 240.7 ms with it (medians of 25). Under `--check-allocations` the map's own growth is counted too, so benchmarks run without that option. Rejected: counting program allocations through the allocation map only (needs the check, whose map dominates timings, and misses the runtime's allocations), `--timings` printing frames.

Decisions from `operator-conventions` (planned autonomously on 2026-10-20 for part L1 of the review's agreed item A4, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`; the conventions themselves are in `operators`):

- **Operator uses run as the calls they spell** (D4, D6). Operands run left to right, each index of a compound assignment through `set` once; `==` of a type with `equals` runs that function wherever values of the type are compared, user code included in list searches, state writes and compose's comparisons.

Decisions from `drop-and-clone-hooks` (part L2 of the review's agreed item A4, planned autonomously on 2026-10-20, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`; the user decided in A4 that there is no `unsafe` layer for now, so hooks serve types wrapping runtime handles, not raw memory):

- **Hooks run where drops and clones already run** (D7, D8): a moved-from value runs no hook, a value moved on some paths runs it on the others, an assignment runs the old value's, and a hook runs wherever glue runs: fields, payloads, nullables, list elements (index order, and the old element of an indexed write), box contents (not on `unbox()`), owned closure captures, type-parameter values of instances, released slots and freed state cells. A closure keeps only what it captures by value, so a binding whose `Copy` field an owning closure copies runs its hook at its own scope end (the closure keeps a possibly stale handle, a logic error the runtime's handle checks report, as for `val id = t.id`). Rejected: truncating copied paths at a hooked value, which would reject `{ t.id }` over a reference.
- **Top-level values are never dropped by the program; the allocation check drops them with their hooks** (D13): it already released their memory in reverse order of initialization, and running hooks keeps resources held by top-level values from being reported live. The check marks each value dropped in its accessor's state byte before dropping it (`TOP_LEVEL_VALUE_DROPPED`), and the accessor then returns 1 (0 while initializing), so a reader tests `address < 2` in one compare and a hook reading a dropped value raises `top-level value '<name>' read after the allocation check dropped it`, located, instead of reading freed memory; a value a hook initializes is registered and dropped in turn, until none is left. The output of hooks run by the check is flushed before its report. Rejected: never running them (resources reported as leaks), rejecting top-level values whose type reaches a hook, dropping top-level values at every exit (C++'s static destruction order).
- **A runtime error in a hook ends the program, and exiting runs no hook** (D14): there is no unwinding, so the remaining values are not dropped. Rejected: running the remaining drops after a panic.
- **A hook meets the borrow flags of states** (found in review): assigning a state's value drops the old value while the cell is borrowed, so a hook of that value reading the same state stops with `state accessed while borrowed`, and a clone hook writing the state being cloned likewise; neither can reach freed memory.
