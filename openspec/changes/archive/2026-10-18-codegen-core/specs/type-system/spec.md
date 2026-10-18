# Type System Specification Delta

## ADDED Requirements

### Requirement: Output Built-Ins Are Found Last
A call whose callee is the bare name `print` or `println` MUST call the output built-in of that name only when no local, parameter, top-level declaration or import of that name is in scope; otherwise the name resolves as usual. The built-ins MUST only be called: naming one without a call MUST be rejected with `E4001`.

#### Scenario: A declaration shadows the built-in
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f() { print(1); println(2) }` is checked
- **THEN** the file typechecks, `print(1)` calls `main::print` and `println(2)` calls the output built-in

#### Scenario: A built-in is not a value
- **WHEN** `fun f() { val p = println }` is checked
- **THEN** the type checker emits `E4001` for `println`

### Requirement: Output Built-In Arguments
`print` MUST take exactly one argument and `println` zero or one, given positionally, of type `Int`, `Double`, `Bool`, `Char` or `String`, or a reference to one. A `Copy` argument MUST be passed by value, copied out of a reference, and a `String` MUST be borrowed implicitly, as an argument of a `&String` parameter is, without being moved. Any other count, a named argument or another type MUST be rejected with `E4004`. A call MUST have type `Unit`.

#### Scenario: Printing scalars and strings
- **WHEN** `fun f(r: &Int, s: String) { print(1); println(2.5); println(true); println('c'); println(r); println(s); println(s); println() }` is checked
- **THEN** the file typechecks, and `s` is not moved by either call

#### Scenario: Printing an unsupported value
- **WHEN** `struct P { val x: Int }` is declared and `fun f(n: Int?) { println(P(1)); println(n); println({ 1 }) }` is checked
- **THEN** the type checker emits `E4004` for each of the three arguments, with help listing the printable types

#### Scenario: Wrong argument count or form
- **WHEN** `fun f() { print(); println(1, 2); println(value = 1) }` is checked
- **THEN** the type checker emits `E4004` for each of the three calls
