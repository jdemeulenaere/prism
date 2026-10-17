# Type System Specification Delta

## MODIFIED Requirements

### Requirement: Static Type Inference and Assignment Compatibility
The type checker MUST infer types for unannotated `val` and `var` bindings, enforce strict numeric typing without implicit coercion, and verify assignment compatibility for references, nullable types, and struct fields.

#### Scenario: Primitive type inference
- **WHEN** `val x = 42`, `val c = 'a'`, or `val b = true` is declared without an explicit type annotation
- **THEN** the type checker infers `Int`, `Char`, and `Bool` respectively

#### Scenario: Mismatched type assignment rejected
- **WHEN** a variable declared as `Int` is assigned an expression of type `String`
- **THEN** the type checker rejects the assignment with a type mismatch error (`E4002`)

#### Scenario: Implicit numeric widening prohibited
- **WHEN** an arithmetic operation mixes numeric types (e.g. `val x: Float = 1 + 2.0`)
- **THEN** the type checker rejects the operation with a type mismatch error (`E4002`)

#### Scenario: Reassignment to immutable val rejected
- **WHEN** code attempts to reassign to a variable declared with `val`
- **THEN** the type checker emits an immutability violation error (`E4003`)

#### Scenario: Struct field mutability validation
- **WHEN** code attempts to assign to a struct field declared with `val`, or mutates a field on an immutable `val` struct instance
- **THEN** the type checker emits an immutability violation error (`E4003`)

#### Scenario: Mutability of nested field places
- **WHEN** code assigns to a field path such as `o.user.name = "x"` or takes `&mut o.user.name`
- **THEN** every hop must be mutable: the root is a `var` binding or is reached through `&mut`, every field on the path is `var`, and no hop goes through a shared `&` borrow; otherwise the type checker emits `E4003`

#### Scenario: Reference and borrow compatibility
- **WHEN** passing an argument to a parameter expecting `&T` or `&mut T`
- **THEN** a `&T` parameter accepts a matching borrow (with `&mut T` coercible to `&T`) or a `T`, which is borrowed implicitly; a `&mut T` parameter requires a `&mut` borrow, written explicitly unless the argument already is one; and taking `&mut` on an immutable `val` binding is rejected

#### Scenario: Borrowed constructor values match annotated references
- **WHEN** a value built by a constructor (`val u = User("a")` or `val c = Color.Red`) is borrowed and passed to a parameter `&User`, `&mut User` or `&Color`
- **THEN** the borrow matches: `&T` accepts `&U` and `&mut T` accepts `&mut U` only when `U` and `T` are the same type, since a reference cannot change the representation of the value it points at

#### Scenario: Error types absorb borrows and safe navigation
- **WHEN** the operand of `&`, `&mut` or a safe navigation `?.` already has an error
- **THEN** the result has the error type, and no follow-on mismatch such as `found '&mut <error>'` is reported

#### Scenario: Nullable type compatibility
- **WHEN** `null` is assigned to a variable of type `T?`
- **THEN** the assignment succeeds; whereas assigning `null` to non-nullable `T` emits `E4002`

#### Scenario: Safe navigation and Elvis operator evaluation
- **WHEN** accessing a field via safe navigation `opt?.field` on `T?` or coalescing via `opt ?: default`
- **THEN** `opt?.field` evaluates to `FieldType?` and `opt ?: default` evaluates to `T` when `default` is of type `T`

#### Scenario: Elvis right side is always checked
- **WHEN** `left ?: right` is evaluated and `left` is not nullable
- **THEN** `right` is still typechecked (an unresolved name emits `E4001`, an incompatible type emits `E4002`), and the expression has the type of `left`

#### Scenario: Nullable types never nest
- **WHEN** a safe navigation reads a nullable field through a nullable value (`b?.a` where `a: A?`), so the chain `b?.a?.name` continues
- **THEN** the result is `A?`, not `A??`, and the chain resolves `name` against `A` to give `String?`

#### Scenario: Indexing expression evaluation
- **WHEN** indexing into a collection or array `target[index]`
- **THEN** `index` must evaluate to `Int` and the expression produces the element type (or emits `E4002` if incompatible)

### Requirement: Operators Read Through References
Operator operands (arithmetic, comparison, equality, logical, unary and range bounds), and `if` and subjectless-`when` conditions, MUST be typed after peeling every `&` and `&mut` layer, so a borrowed value can be read by operators. Positions that expect a value of type `T` (arguments, initializers, returns, assignment values) accept a `&T` only by copying a `Copy` value out of it, as the `references` spec defines.

#### Scenario: Arithmetic on a borrowed integer
- **WHEN** `fun f(a: &Int): Int = a + 1` is checked
- **THEN** the function typechecks successfully

#### Scenario: Equality between a borrow and a value
- **WHEN** `fun f(a: &Int, b: Int): Bool = a == b` is checked
- **THEN** the function typechecks successfully

#### Scenario: Borrowed boolean as a condition
- **WHEN** `fun f(c: &Bool): Int = if (c) 1 else 2` is checked
- **THEN** the function typechecks successfully

#### Scenario: Borrow passed where a value is expected
- **WHEN** `fun g(s: String): Int` is declared and `fun f(a: &String): Int = g(a)` is checked
- **THEN** the type checker rejects the argument with `E4017`, as a move out of the borrow `a`
