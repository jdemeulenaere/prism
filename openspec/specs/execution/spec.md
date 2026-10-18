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
Arguments after `--` MUST be passed to the program and never interpreted by `prism`; no built-in reads them yet. The `--timings` option, before the file, MUST print to stderr the time spent in each stage (front end, code generation, linking) and the total time from the start of `prism` until `main` is called, before `main` runs.

#### Scenario: Arguments after the separator
- **WHEN** `prism run main.prism -- --timings x` runs on `fun main() { println(1) }`
- **THEN** it prints `1` to stdout, nothing to stderr, and exits with status 0

#### Scenario: Reporting timings
- **WHEN** `prism run --timings main.prism` runs on `fun main() { println(1) }`
- **THEN** stderr holds one line per stage and one line with the total time to `main`, each in milliseconds, and stdout holds `1`

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
`print(x)` MUST write the text of `x` to stdout, `println(x)` the text of `x` followed by a newline, and `println()` a newline. An `Int` MUST be written in decimal with a leading `-` when negative, a `Bool` as `true` or `false`, a `Char` as its UTF-8 encoding and a `String` as its text in UTF-8.

#### Scenario: Printing scalars
- **WHEN** `fun main() { print(1); print(true); println('x'); println(-5); println() }` runs
- **THEN** stdout holds `1truex`, a newline, `-5`, a newline, and a newline

#### Scenario: Printing strings
- **WHEN** `fun main() { val s = "Grüße, \u{1F680}"; print(s); println("!"); println(s) }` runs
- **THEN** stdout holds `Grüße, 🚀!`, a newline, `Grüße, 🚀` and a newline

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
A runtime error MUST flush the program's output, write `panic: <message>` and then `  at <path>:<line>:<column>` to stderr, where the path is the file's path relative to the workspace root and the position is that of the failing operation, and exit with status 101. "out of memory", and "stack overflow" raised inside glue, MUST write no location line. No further code of the program MUST run, and no drop MUST run.

#### Scenario: Runtime error location
- **WHEN** `app/main.prism` declares `fun zero(): Int = 0` on line 2 and `fun main() { val a = 1; println(a / zero()) }` on line 3, with `a / zero()` starting at column 33
- **THEN** stderr is exactly `panic: division by zero` and `  at app/main.prism:3:33`, each followed by a newline

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
`==` MUST compare structs field by field, enums by variant and then payload, and nullables as equal when both are `null` or both hold equal values, each field or payload with `==` of its type; `!=` MUST be its negation. Comparing borrows MUST compare what they refer to.

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
`x.clone()` of a value whose type owns no storage MUST give a bitwise copy of `x`, with no allocation and no user code run.

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
Each drop that the elaborated MIR runs MUST free, at that point, the memory the dropped value owns, deterministically and without a garbage collector or reference counting, so that every allocation a value owns is freed exactly once: where its owner's scope ends, where it is overwritten by an assignment or a write through a reference, or by the code it was moved to. A top-level value MUST never be dropped.

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
`x.clone()` of a value that owns storage MUST give a value equal to `x` that shares no owned memory with it, so that changing or dropping either leaves the other unchanged, and MUST run no user code.

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
In an instance, a value of a type parameter's type MUST be dropped, moved and copied as a value of the type argument is: dropping it runs the argument's drop glue, a move transfers its storage, and a `clone()` under a `Clone` bound builds a value that shares no storage with the original. A value whose argument owns nothing MUST drop without running any code.

#### Scenario: Dropping and cloning strings through generic code
- **WHEN** `struct Pair<A, B>(val a: A, val b: B)`, `fun <T> pick(a: T, b: T, first: Bool): T = if (first) a else b`, `fun <T> discard(x: T) {}` and `fun <T : Clone> both(x: &T): Pair<T, T> = Pair(x.clone(), x.clone())` are declared and `fun main() { println(pick("a", "b", false)); discard("c" + "d"); val p = both(&("e" + "f")); println(p.a + p.b) }` runs under `prism run --check-allocations`
- **THEN** the program prints `b` and `efef`, reports no live allocation and no double free, and exits with status 0

#### Scenario: Generic code over structs that own strings
- **WHEN** `struct User : Clone { val name: String }`, `struct Box<T>(val v: T)` and `fun <T> unwrap(b: Box<T>): T { val (v) = b; return v }` are declared and `fun main() { val u = unwrap(Box(User("ada"))); println(u.name); val c = unwrap(Box(Box(u.clone()))); println(c.v.name) }` runs under `prism run --check-allocations`
- **THEN** the program prints `ada` twice, reports no live allocation and exits with status 0

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
- **WHEN** `enum List<T> { Cons(T, Heap<List<T>>), Nil }`, `fun <T> List<T>.size(): Int = when (this) { List.Cons(_, val rest) -> 1 + rest.size(), List.Nil -> 0 }` and `fun <T> push(list: List<T>, x: T): List<T> = List.Cons(x, Heap(list))` are declared and `fun main() { val empty: List<String> = List.Nil; val names = push(push(empty, "a"), "b"); val none: List<Int> = List.Nil; val numbers = push(none, 1); println(names.size()); println(numbers.size()) }` runs under `prism run --check-allocations`
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
