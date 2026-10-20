# Loops Specification

## Purpose
Defines Prism's loop statements (`while`, `do`/`while`, `for`/`in`) and the `break` and `continue` jumps: their grammar, scoping and typing, and which loop a jump belongs to.

## Requirements

### Requirement: While Loops
The grammar MUST accept `while (condition) body` as a statement. The parentheses are mandatory, as for `if`. The condition MUST typecheck as `Bool` after peeling `&` and `&mut` layers, as `if` conditions do, otherwise `E4002` is reported. The loop has type `Unit`.

#### Scenario: While loop with a block body
- **WHEN** `fun f(n: Int) { var i = 0; while (i < n) { i = i + 1 } }` is checked
- **THEN** the program typechecks without errors

#### Scenario: Non-Bool while condition rejected
- **WHEN** `fun f(n: Int) { while (n) { } }` is checked
- **THEN** the type checker emits `E4002`

#### Scenario: Borrowed Bool condition
- **WHEN** `fun f(c: &Bool) { while (c) { } }` is checked
- **THEN** the program typechecks without errors

#### Scenario: While condition without parentheses rejected
- **WHEN** parsing `fun f(c: Bool) { while c { } }`
- **THEN** the parser reports `E2001`

### Requirement: Do-While Loops
The grammar MUST accept `do body while (condition)` as a statement whose body is a braced block. The condition MUST be checked like a `while` condition, in a scope that still contains the locals declared at the top level of the body.

#### Scenario: Condition sees the body's locals
- **WHEN** `fun next(): Int = 0` and `fun f() { do { val x = next() } while (x > 0) }` are checked
- **THEN** the program typechecks without errors, and `x` in the condition refers to the body's local

#### Scenario: Body's locals do not leak past the loop
- **WHEN** `fun f() { do { val x = 1 } while (x > 0); val y = x }` is checked
- **THEN** the type checker emits `E4001` for the `x` in `val y = x`

### Requirement: For Loops Over Ranges
`for (name in iterable) body` MUST be a statement. Its iterable, read through `&`, `&mut` and `Heap<T>` layers, MUST be a `Range`, a list (`collections`) or a type with the cursor protocol (`operators`), else `E4002` naming them, and is evaluated once, before the first iteration. The loop variable MUST be a fresh immutable binding per iteration, in a scope enclosing the body, which may shadow it: an `Int` over a range, an element borrow over a list, `next`'s result over a cursor.

#### Scenario: Counting loop over a half-open range
- **WHEN** `fun f(n: Int): Int { var sum = 0; for (i in 0..<n) { sum = sum + i }; return sum }` is checked
- **THEN** the program typechecks without errors and `i` has type `Int`

#### Scenario: Non-range iterable rejected
- **WHEN** `fun f(s: String) { for (c in s) { } }` is checked
- **THEN** the type checker emits `E4002`

#### Scenario: Loop variable is immutable
- **WHEN** `fun f() { for (i in 0..10) { i = 0 } }` is checked
- **THEN** the type checker emits `E4003`

#### Scenario: Body may shadow the loop variable
- **WHEN** `fun f() { for (i in 0..10) { val i = "s" } }` is checked
- **THEN** the program typechecks without errors

#### Scenario: A list is iterable
- **WHEN** `fun f(xs: List<Int>, ys: &List<Int>) { for (x in xs) {}; for (y in ys) {}; for (i in xs.indices) {} }` is checked
- **THEN** the program typechecks without errors

### Requirement: Loop Bodies
A loop body MUST be a braced block, or for `while` and `for` a single unbraced statement that is an expression or an assignment, as for `if` branches. A local declaration as an unbraced loop body MUST be rejected with `E2001`. Locals declared in a loop body MUST NOT be visible after the loop.

#### Scenario: Unbraced assignment body
- **WHEN** `fun f(n: Int) { var i = 0; while (i < n) i = i + 1 }` is checked
- **THEN** the program typechecks without errors

#### Scenario: Unbraced declaration body rejected
- **WHEN** parsing `fun f(c: Bool) {\nwhile (c) val x = 1\n}`
- **THEN** the parser reports `E2001`

### Requirement: Loops Are Unit Statements
A loop MUST be a statement and MUST NOT appear in expression position, where the parser reports `E2001`. A block or function body that ends with a loop has no value. A non-`Unit` function MUST still end in a value or a `return` after a loop, even one whose condition is `true`.

#### Scenario: Loop as an initializer rejected
- **WHEN** parsing `fun f(c: Bool) { val x = while (c) { } }`
- **THEN** the parser reports `E2001`

#### Scenario: Function ending in a loop needs a value
- **WHEN** `fun f(): Int { while (true) { return 1 } }` is checked
- **THEN** the type checker emits `E4005`, while adding `return 0` after the loop typechecks

### Requirement: Break and Continue
`break` and `continue` MUST be expressions of type `Nothing` that take no operand and belong to the innermost enclosing loop. Like `return`, they may appear wherever an expression is allowed, such as the right side of `?:` or an `if` branch.

#### Scenario: Break on the right of Elvis
- **WHEN** `fun f(xs: Int?) { while (true) { val x = xs ?: break; val y: Int = x } }` is checked
- **THEN** the program typechecks without errors and `x` has type `Int`

#### Scenario: Continue in an if branch
- **WHEN** `fun f() { for (i in 0..10) { if (i == 3) continue; g(i) } }` is checked with `fun g(x: Int) { }`
- **THEN** the program typechecks without errors

#### Scenario: Break in a nested loop
- **WHEN** a `break` appears in the body of a `while` that is itself inside a `for` body
- **THEN** the program typechecks without errors, and the `break` belongs to the inner `while`

#### Scenario: Break takes no value
- **WHEN** parsing `fun f() { while (true) { break 1 } }`
- **THEN** the parser reports `E2001`, as for two expressions on one line without a separator

### Requirement: Jumps Outside a Loop Rejected
A `break` or `continue` that has no enclosing loop in the same function or closure body MUST be rejected with `BreakOutsideLoop` (`E4015`), naming the keyword. A closure body starts with no enclosing loop, so a jump cannot leave a closure that sits inside a loop.

#### Scenario: Break in a function body
- **WHEN** `fun f() { break }` is checked
- **THEN** the type checker emits `E4015`

#### Scenario: Continue inside a closure inside a loop
- **WHEN** `fun run(f: () -> Unit) { }` and `fun g() { while (true) { run { continue } } }` are checked
- **THEN** the type checker emits `E4015` for `continue`

#### Scenario: Loop inside a closure
- **WHEN** `fun run(f: () -> Unit) { }` and `fun g() { run { while (true) { break } } }` are checked
- **THEN** the program typechecks without errors

### Requirement: Loop and Range Evaluation Semantics
A `while` loop MUST evaluate its condition before each iteration and a `do`/`while` loop after each iteration. A `for` loop MUST evaluate its range once and bind the loop variable to each `Int` in it in ascending order, without overflowing at the end of the `Int` range. `break` MUST leave the innermost enclosing loop and `continue` MUST go to its next condition check or element. `a..b` MUST include `b`, and `a..<b` MUST exclude it.

#### Scenario: Half-open range iteration
- **WHEN** `for (i in 0..<3)` runs, and `for (i in 3..<3)` runs
- **THEN** the first loop's body runs with `i` equal to 0, 1 and 2, and the second loop's body never runs

#### Scenario: Closed range iteration
- **WHEN** `for (i in 1..3)` runs
- **THEN** its body runs with `i` equal to 1, 2 and 3

#### Scenario: Range ending at the largest Int
- **WHEN** `for (i in 9223372036854775806..9223372036854775807)` runs
- **THEN** its body runs twice, and no integer overflow is raised

#### Scenario: Do-while body runs once
- **WHEN** `do { n = n + 1 } while (false)` runs with `n` equal to 0
- **THEN** `n` is 1

#### Scenario: While condition checked first
- **WHEN** `while (n < 3) { n = n + 1 }` runs with `n` equal to 5
- **THEN** its body never runs and `n` is still 5

#### Scenario: Continue skips to the next element
- **WHEN** `for (i in 0..<5) { if (i == 2) continue; sum = sum + i }` runs with `sum` equal to 0
- **THEN** `sum` is 8

#### Scenario: Break leaves only the innermost loop
- **WHEN** a `break` runs inside a `while` loop nested in a `for` loop
- **THEN** the `while` loop ends and the `for` loop continues with its next element

#### Scenario: Range bounds evaluated once
- **WHEN** `for (i in 0..<next())` runs, where `next()` returns 3
- **THEN** `next()` is called once, before the first iteration

## Design Rationale
Decisions from `statements-and-loops` (agreed with the user in the borrow-checker roadmap exploration):

- **Kotlin's loop forms, as statements.** `while`, `do`/`while` and `for`/`in` are statements of type `Unit`, and `break` carries no value. Rust's `loop` expression with a break value was not adopted, since Kotlin users don't expect it.
- **`for` iterates only the built-in `Range` for now.** The iterable is evaluated once and read through references like a condition, and the loop variable is a fresh immutable `Int` in a scope level above the body, as parameters sit above a function body, so the body may shadow it. General iteration needs generics and traits.
- **The `do`/`while` condition sees the body's locals,** as in Kotlin. The `do` body must be braced.
- **Loops are not diverging.** `while (true)` with no `break` is still `Unit`, so a non-`Unit` function needs a value or `return` after it. Treating a literal-`true` loop as `Nothing` would only relax this, so it can be added later without breaking code.
- **`break` and `continue` are `Nothing`-typed expressions,** so `xs ?: break` works as in Kotlin. The type checker tracks loop depth per body, and a closure body starts at zero, so a jump can't leave a closure (`E4015`). Doing this in the checker keeps the parser free of context stacks and puts the rule next to the closure-boundary logic for compose contexts.
- **Labels (`break@outer`) are deferred.** They're additive, and Kotlin's syntax for them is fixed.

Decisions from `codegen-core` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Loop evaluation follows the MIR shapes** (from `docs/architecture.md` §4). A `for` loop evaluates its range once into a hidden `Range` local and tests its latch against the last element before incrementing, so a range ending at the largest `Int` never overflows; `continue` in a `do`/`while` goes to its condition. These are runtime semantics shared by every tier.

Decisions from `collections` (planned autonomously on 2026-10-18, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`for` iterates lists too, without a protocol** (D9, D10; supersedes "`for` iterates only the built-in `Range` for now" above). The iterable is a `Range` or a `List`, read through `&`, `&mut` and `Heap<T>` layers; over a list the variable is a `&T` (or `&mut T`) per element, and the list is borrowed for the whole loop. A user-visible iteration protocol waits for traits (`collections`, "Design Rationale").

Decisions from `operator-conventions` (planned autonomously on 2026-10-20 for part L1 of the review's agreed item A4, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`; the conventions themselves are in `operators`):

- **Other types iterate through a cursor protocol** (D11; supersedes "without a protocol" above). A `for` over any type other than `Range` and `List` calls its `iterator`, `hasNext` and `next` operators, the collection itself serving the protocol with a cursor holding no reference, since an iterator object borrowing its collection would be a struct holding a borrow. The loop variable is `next`'s result.
