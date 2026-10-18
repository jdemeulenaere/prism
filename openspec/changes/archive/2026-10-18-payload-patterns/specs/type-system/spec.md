# Type System Specification Delta

## MODIFIED Requirements

### Requirement: Control Flow Type Unification and Return Validation
Control flow expressions (`if`/`else`, `when`), blocks, and function return statements MUST satisfy type unification and exhaustiveness rules.

#### Scenario: If-else branch unification
- **WHEN** an `if (cond) e1 else e2` expression is evaluated
- **THEN** the condition must typecheck as `Bool`, and both branches must unify to a common type

#### Scenario: If without else where a value is expected
- **WHEN** `fun f(c: Bool) { val x: Int = if (c) nope }` is checked
- **THEN** the type checker emits `E4001` once, for `nope`, and `E4002` once, because an `if` without `else` has type `Unit`

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

#### Scenario: Range pattern without a subject rejected
- **WHEN** `fun f(x: Int): Int = when { in 0..<10 -> 1; else -> 2 }` is checked
- **THEN** the type checker emits `E4002`, because without a subject a pattern is a condition, which a range is not

#### Scenario: Non-exhaustive when expression rejected
- **WHEN** a `when` expression is used in value position and fails to cover all possible enum variants or omit an `else` branch
- **THEN** the type checker rejects the expression with a non-exhaustive pattern match error (`E4009`) naming a value no branch covers (see the [`patterns`](../patterns/spec.md) spec, "Uncovered Values Are Named")

#### Scenario: Boolean subject covered by true and false
- **WHEN** a `when` over a `Bool` subject (or a `&Bool`) in value position has a branch for the literal `true` and one for `false`, or a `Bool?` subject has branches for `true`, `false` and `null`, each alone or among several patterns of a branch
- **THEN** it is exhaustive without an `else` branch, as a `when` over an enum naming every variant is, and the typed tree records it as exhaustive; with only one of the literals, or without `null` over a `Bool?`, it still needs `else` (`E4009`)

#### Scenario: Only a variant name covers that variant
- **WHEN** a `when` over an enum has a pattern naming a payload-free variant (`Color.Red`, or bare `Red` when no local binding shadows it), a payload variant alone (`Shape.Circle`), or a constructor pattern whose sub-patterns cover every payload (`Shape.Circle(val r)`, `Shape.Circle(_)`)
- **THEN** the pattern covers that variant, while a constructor pattern with a refutable sub-pattern, such as `Shape.Circle(1)`, covers only the values it matches, and a value that only evaluates to a variant, such as a field `h.Green`, covers nothing

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
