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
The grammar MUST accept `for (name in iterable) body` as a statement. The iterable MUST have the built-in `Range` type after peeling `&` and `&mut` layers, as conditions do, otherwise `E4002` is reported. It is evaluated once, before the first iteration. The loop variable MUST be a fresh immutable `Int` binding in each iteration, in a scope enclosing the body, so the body may shadow it.

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

## Design Rationale
Decisions from `statements-and-loops` (agreed with the user in the borrow-checker roadmap exploration):

- **Kotlin's loop forms, as statements.** `while`, `do`/`while` and `for`/`in` are statements of type `Unit`, and `break` carries no value. Rust's `loop` expression with a break value was not adopted, since Kotlin users don't expect it.
- **`for` iterates only the built-in `Range` for now.** The iterable is evaluated once and read through references like a condition, and the loop variable is a fresh immutable `Int` in a scope level above the body, as parameters sit above a function body, so the body may shadow it. General iteration needs generics and traits.
- **The `do`/`while` condition sees the body's locals,** as in Kotlin. The `do` body must be braced.
- **Loops are not diverging.** `while (true)` with no `break` is still `Unit`, so a non-`Unit` function needs a value or `return` after it. Treating a literal-`true` loop as `Nothing` would only relax this, so it can be added later without breaking code.
- **`break` and `continue` are `Nothing`-typed expressions,** so `xs ?: break` works as in Kotlin. The type checker tracks loop depth per body, and a closure body starts at zero, so a jump can't leave a closure (`E4015`). Doing this in the checker keeps the parser free of context stacks and puts the rule next to the closure-boundary logic for compose contexts.
- **Labels (`break@outer`) are deferred.** They're additive, and Kotlin's syntax for them is fixed.
