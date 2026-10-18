# References Specification Delta

## MODIFIED Requirements

### Requirement: Implicit Shared Borrow at Argument Sites
Where a call's parameter has type `&T`, an argument of type `T` MUST be borrowed implicitly. A place argument (a variable or a `.` field path) MUST be borrowed where it lives, without being consumed. A string's character `s[i]` is a value, not a place. Any other argument MUST be evaluated as a temporary, in a consuming position, that lives until the end of the statement. Default values are arguments. Elsewhere a borrow MUST be written with `&`.

#### Scenario: Implicit shared borrow at argument sites
- **WHEN** `fun greet(name: &String)` and `struct User { val name: String }` are declared and a body of `fun f(u: &User, name: String)` runs `greet(name)`, `greet(u.name)` and `greet("Ada")`
- **THEN** the body typechecks successfully, and neither `name` nor `u.name` is reported as moved

#### Scenario: Borrowing the result of a call
- **WHEN** `fun make(): String` and `fun greet(name: &String)` are declared and `fun f() { greet(make()) }` is checked
- **THEN** the file typechecks successfully

#### Scenario: A branch result is a temporary
- **WHEN** `fun greet(name: &String)` and `struct User { val name: String }` are declared, and `fun f(c: Bool, u: User, v: User) { greet(if (c) u.name else v.name) }` and `fun g(c: Bool) { greet(if (c) "a" else "b") }` are checked
- **THEN** the type checker emits `E4017` for `u.name` and for `v.name`, and accepts `g`

#### Scenario: A default value is borrowed like an argument
- **WHEN** `fun greet(name: &String = "guest")` is declared and `fun f() { greet() }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Calling a function-typed value
- **WHEN** `fun f(g: (&String) -> Unit, s: String) { g(s) }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Borrows outside arguments are written
- **WHEN** `struct User { val name: String }` is declared, and `fun f(name: String) { val r: &String = name }` and `fun g(u: &User): &String = u.name` are checked
- **THEN** the type checker emits `E4002` for `f`, with help suggesting `&name`, and `E4005` for `g`, with help suggesting `&u.name`

#### Scenario: A string character is borrowed as a temporary
- **WHEN** `fun show(c: &Char): Int = 1` is declared and `fun f() { var s = "ab"; val r = &s[0]; s = "cd"; println(show(s[1])); println(r) }` is checked
- **THEN** the file typechecks successfully: `r` and the argument of `show` borrow temporaries holding the characters, not `s`
