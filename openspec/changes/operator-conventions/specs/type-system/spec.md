## MODIFIED Requirements

### Requirement: Static Type Inference and Assignment Compatibility
The type checker MUST infer types for unannotated `val` and `var` bindings, enforce strict numeric typing without implicit coercion, and verify assignment compatibility for references, nullable types, and struct fields. The floating-point type is `Double`, which a type annotation MAY also write `Float`; every diagnostic, dump and ABI MUST name it `Double`.

#### Scenario: Primitive type inference
- **WHEN** `val x = 42`, `val c = 'a'`, or `val b = true` is declared without an explicit type annotation
- **THEN** the type checker infers `Int`, `Char`, and `Bool` respectively

#### Scenario: Mismatched type assignment rejected
- **WHEN** a variable declared as `Int` is assigned an expression of type `String`
- **THEN** the type checker rejects the assignment with a type mismatch error (`E4002`)

#### Scenario: Implicit numeric widening prohibited
- **WHEN** an arithmetic operation mixes numeric types (e.g. `val x: Double = 1 + 2.0`)
- **THEN** the type checker rejects the operation with a type mismatch error (`E4002`)

#### Scenario: Diagnostics name the floating-point type Double
- **WHEN** `fun f(x: Float, y: Double) { x(1); y(2) }` is checked
- **THEN** the type checker emits `E4004` twice, each saying `type 'Double' cannot be called as a function`: the alias `Float` is the same type, and no message calls it `Float`

#### Scenario: Ordering comparison needs Int, Double or Char
- **WHEN** `<`, `<=`, `>` or `>=` compares two `Int`s, two `Double`s or two `Char`s, it has type `Bool`; when it compares two `Bool`s (`false < true`), two `String`s, values of different types (`1 < 2.0`), or any other type, and no `compareTo` operator function of the left operand's type applies (`operators`)
- **THEN** the type checker rejects it with `E4002` through `OperatorMismatch`, whose message names the operator and both operand types (`operator '<' cannot be applied to 'Bool' and 'Bool'`) and whose help names the accepted types, rather than a mismatch between two identical types

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
- **WHEN** indexing into a `List` or a `String`, `target[index]`
- **THEN** `index` must evaluate to `Int` and the expression produces the element type (or emits `E4002` if incompatible); indexing any other type calls its `get` operator function, whose parameters type the indices (`operators`)

### Requirement: Compound Assignment Typing
`t op= v` MUST typecheck as `t = t op v` with `t` written once. The target MUST be writable under the rules for `=` (`E4003` otherwise). The operator MUST apply to the target's type and the value's type: built in for `Int` with `Int` and `Double` with `Double` for every compound operator and `String` with `String` for `+=` only, else through an operator function (`operators`). Otherwise the type checker MUST report `E4002`, with help naming the operand types the operator accepts.

#### Scenario: Compound assignment of numbers and strings
- **WHEN** a body runs `var n = 1; n += 2; n -= 1; n *= 3; n /= 2; n %= 2`, `var d = 1.5; d += 0.5; d %= 1.0` and `var s = "a"; s += "b"`
- **THEN** the body typechecks successfully

#### Scenario: Compound assignment through fields and references
- **WHEN** `struct Counter { var count: Int }` and `mut fun Counter.bump() { count += 1; this.count *= 2 }` are declared, and `fun f(r: &mut Int, c: &mut Counter, k: &Int) { *r += k; c.count -= 1 }` is checked
- **THEN** the file typechecks successfully, and the value `k` is read through its reference

#### Scenario: Compound assignment to an immutable target rejected
- **WHEN** `struct P { val x: Int; var y: Int }` is declared and a body of `fun f(r: &Int, q: P)` runs `val n = 1; n += 1`, `var p = P(1, 2); p.x += 1`, `q.y += 1` or `*r += 1`
- **THEN** the type checker emits `E4003` for each, as it does for `=`

#### Scenario: Compound operator not applicable rejected
- **WHEN** a body runs `var n = 1; n += "a"`, `var s = "a"; s -= "b"`, `var m: Int? = 1; m += 1`, `var x = 1; x += 1.0` or `var b = true; b += true`
- **THEN** the type checker emits `E4002` for each, with help saying which operand types the operator accepts

#### Scenario: Compound assignment does not consume its value
- **WHEN** `fun take(s: String) {}` is declared and `fun f(t: String) { var s = "a"; s += t; take(t) }` is checked
- **THEN** the file passes `prism check`, because `+=` reads `t` in place
