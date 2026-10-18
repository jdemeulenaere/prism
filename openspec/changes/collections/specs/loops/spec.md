# Loops Specification Delta

## MODIFIED Requirements

### Requirement: For Loops Over Ranges
The grammar MUST accept `for (name in iterable) body` as a statement. The iterable MUST have the built-in `Range` type after peeling `&`, `&mut` and `Heap<T>` layers, as conditions do, or be a list, which `collections` ("For Loops Over Lists") specifies; otherwise `E4002` is reported, naming `Range` and `List<T>` as the iterable types. It is evaluated once, before the first iteration. Over a range, the loop variable MUST be a fresh immutable `Int` binding in each iteration, in a scope enclosing the body, so the body may shadow it; over a list, it is such a binding of a borrow of the element.

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
