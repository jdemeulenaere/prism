# Type System Delta

## MODIFIED Requirements

### Requirement: Output Built-In Arguments
`print` and `println`, the intrinsics of `/prism/core`, MUST take one argument, and `println` also none, given positionally, of any type with a text form, the rule of string template parts ("Built-In Text Forms of Template Parts", "Other Template Parts Call toString"). The argument MUST be read in place through its `&`, `&mut` and `Heap<T>` layers, never moved. A `Copy` scalar (`Int`, `Double`, `Bool`, `Char`) MUST be passed by value, copied out. A `String` MUST be borrowed implicitly, as an argument of a `&String` parameter is. Any other type MUST be printed through the `toString()` that `x.toString()` resolves, called with a shared borrow of the argument, its result printed and then dropped. An argument without a text form (a nullable, `Unit`, a function value, a type parameter, or a struct or enum with no usable `toString()`) MUST be rejected with `E4026`, with the help a template part gets. Any other count or a named argument MUST be rejected with `E4004`. A call MUST have type `Unit`.

#### Scenario: Printing scalars and strings
- **WHEN** `fun f(r: &Int, s: String) { print(1); println(2.5); println(true); println('c'); println(r); println(s); println(s); println() }` is checked
- **THEN** the file typechecks, and `s` is not moved by either call

#### Scenario: Printing a value through its toString
- **WHEN** `struct P { val x: Int }` and `fun P.toString(): String = "P(" + x.toString() + ")"` are declared, and `fun f(p: P) { println(p); println(p) }` is checked
- **THEN** the file typechecks, each call borrows `p` for its `toString()` call, and `p` is not moved

#### Scenario: Printing an unsupported value
- **WHEN** `struct P { val x: Int }` is declared without a `toString()` and `fun f(n: Int?) { println(P(1)); println(n); println({ 1 }) }` is checked
- **THEN** the type checker emits `E4026` for each of the three arguments, the first with help suggesting `fun P.toString(): String` and the second with help suggesting `?:`

#### Scenario: Wrong argument count or form
- **WHEN** `fun f() { print(); println(1, 2); println(value = 1) }` is checked
- **THEN** the type checker emits `E4004` for each of the three calls

#### Scenario: Printing boxed values
- **WHEN** `fun f(h: Heap<Int>, s: Heap<String>) { println(h); println(s); println(s) }` is checked
- **THEN** the file typechecks, and neither `h` nor `s` is moved

## REMOVED Requirements

### Requirement: Output Built-Ins Are Found Last
**Reason**: `print` and `println` are declarations of the prelude module `/prism/core`, so the rule that a local, parameter, declaration or import of the same name shadows them is the prelude's rule for every name (`standard-library`, "Prelude Names Are Found Last"). Naming one without a call is `E4013` (a generic function used as a value) instead of `E4001`.
**Migration**: None for programs: a declaration named `print` still shadows the intrinsic. `val p = println` reports `E4013` instead of `E4001`.
