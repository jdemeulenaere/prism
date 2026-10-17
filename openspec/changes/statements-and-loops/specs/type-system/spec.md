# Spec Delta

## ADDED Requirements

### Requirement: Half-Open Ranges
A half-open range `a..<b` MUST be typed exactly like a closed range `a..b`: both operands are checked as `Int`, and the result has the built-in `Range` type. An `in a..<b` pattern in a `when` MUST be checked like `in a..b`, with both bounds checked against the subject's type.

#### Scenario: Half-open range has the Range type
- **WHEN** `fun f(n: Int) { val r = 0..<n }` is checked
- **THEN** the program typechecks without errors and `r` has the built-in `Range` type

#### Scenario: Non-Int bound rejected
- **WHEN** `fun f() { val r = 0..<"s" }` is checked
- **THEN** the type checker emits `E4002`

#### Scenario: Half-open range pattern
- **WHEN** `fun f(x: Int): String = when (x) { in 0..<10 -> "small", else -> "large" }` is checked
- **THEN** the program typechecks without errors

## MODIFIED Requirements

### Requirement: Control Flow Type Unification and Return Validation
Control flow expressions (`if`/`else`, `when`), blocks, and function return statements MUST satisfy type unification and exhaustiveness rules.

#### Scenario: If-else branch unification
- **WHEN** an `if (cond) e1 else e2` expression is evaluated
- **THEN** the condition must typecheck as `Bool`, and both branches must unify to a common type

#### Scenario: Branches join with null into a nullable type
- **WHEN** one branch of an `if`/`else` or `when` has type `T` and another is `null` or has type `T?` (e.g. `val x = if (c) 1 else null`)
- **THEN** the branches join to `T?`

#### Scenario: Diverging expression has bottom type Nothing
- **WHEN** an expression diverges (such as a `return` expression) inside an `if` or `when` branch
- **THEN** the diverging branch has bottom type `Nothing`, which unifies with any expected target branch type (e.g. `val x: Int = if (cond) return 0 else 42`)

#### Scenario: Subject when expression branch checking
- **WHEN** evaluating `when (subject) { p1 -> e1, p2 -> e2, else -> e3 }`
- **THEN** pattern expressions `p1` and `p2` must match the type of `subject`, and branch results unify to a common type

#### Scenario: Subjectless when expression condition checking
- **WHEN** evaluating `when { c1 -> e1, c2 -> e2, else -> e3 }`
- **THEN** conditions `c1` and `c2` must each evaluate to `Bool`

#### Scenario: Non-exhaustive when expression rejected
- **WHEN** a `when` expression is used in value position and fails to cover all possible enum variants or omit an `else` branch
- **THEN** the type checker rejects the expression with a non-exhaustive pattern match error (`E4009`)

#### Scenario: Only a variant name covers that variant
- **WHEN** a `when` over an enum has a pattern naming a payload-free variant (`Color.Red`, or bare `Red` when no local binding shadows it)
- **THEN** the pattern covers that variant, while a pattern that only evaluates to a variant value, such as `Shape.Circle(1)` or a field `h.Green`, covers nothing, so a variant with a payload is covered only by `else`

#### Scenario: When statement needs no else branch
- **WHEN** a `when` is used as a statement, either before the last statement of a block or as the last statement of a block whose expected type is `Unit`
- **THEN** it needs no `else` branch, its branches need no common type, and it has type `Unit`

#### Scenario: When over a borrowed enum
- **WHEN** the subject of a `when` has type `&E` or `&mut E` for an enum `E`
- **THEN** its patterns are checked against `E` and its variants count toward exhaustiveness

#### Scenario: Block statement evaluation and Unit typing
- **WHEN** evaluating a block of statements `{ s1; s2 }`
- **THEN** a block without a tail expression (an empty block `{}`, or one ending in a declaration, an assignment or a loop) evaluates to `Unit`, and otherwise the tail expression determines the block type

#### Scenario: Return statement type verification
- **WHEN** a `return expr` statement occurs inside a function declared with return type `T`
- **THEN** `expr` must be compatible with `T`, and returning a value inside a `Unit` function or returning mismatched types emits a return type error (`E4005`)
