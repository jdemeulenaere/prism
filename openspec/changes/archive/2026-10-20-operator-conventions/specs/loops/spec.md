## MODIFIED Requirements

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
