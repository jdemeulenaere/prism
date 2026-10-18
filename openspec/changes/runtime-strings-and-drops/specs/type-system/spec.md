# Type System Specification Delta

## ADDED Requirements

### Requirement: String Length
A `String` MUST have a built-in member `length` of type `Int`, read without parentheses through any `&`, `&mut` and `Heap<T>` layers, and `s?.length` on a `String?` MUST have type `Int?`. `length` is not a function: `s.length()` MUST be rejected as calling an `Int` (`E4004`), as a call of a field of type `Int` is.

#### Scenario: Reading the length
- **WHEN** `fun f(a: String, b: &String, c: String?): Int = a.length + b.length + (c?.length ?: 0)` is checked
- **THEN** the file typechecks successfully

#### Scenario: Calling the length
- **WHEN** `fun f(s: String): Int = s.length()` is checked
- **THEN** the type checker emits `E4004`, saying that `Int` cannot be called

### Requirement: String Characters Are Values
`s[i]` on a `String`, read through any `&`, `&mut` and `Heap<T>` layers, MUST take an `Int` index (`E4002` otherwise) and have type `Char`. A string's character MUST be a value, not a place: it is computed from the string, so `s[i] = c` MUST be rejected with `E4003`, with help saying that strings are immutable, and a borrow of `s[i]`, written or implicit, MUST borrow a temporary holding the character.

#### Scenario: Reading a character
- **WHEN** `fun f(s: &String, i: Int): Char = s[i + 1]` is checked
- **THEN** the file typechecks successfully

#### Scenario: Assigning a character rejected
- **WHEN** `fun f() { var s = "ab"; s[0] = 'c' }` is checked
- **THEN** the type checker emits `E4003` at `s[0]`, with help saying that the characters of a string cannot be assigned

#### Scenario: A non-integer index rejected
- **WHEN** `fun f(s: String): Char = s['a']` is checked
- **THEN** the type checker emits `E4002`, expecting `Int`

### Requirement: Scalars Have a Built-In toString
An `Int`, `Double`, `Bool` or `Char`, read through any `&` and `&mut` layers, MUST have a built-in member function `toString()` taking no arguments and returning `String`, and `x?.toString()` on a nullable of one of these MUST have type `String?`. Arguments MUST be rejected with `E4004`. On any other type, `toString` MUST be an ordinary name, resolved to a field or an extension like any other member (`E4006` when none exists).

#### Scenario: Converting scalars to strings
- **WHEN** `fun f(n: Int, d: &Double, b: Bool, c: Char, m: Int?): String = n.toString() + d.toString() + b.toString() + c.toString() + (m?.toString() ?: "")` is checked
- **THEN** the file typechecks successfully

#### Scenario: toString takes no arguments
- **WHEN** `fun f(n: Int): String = n.toString(10)` is checked
- **THEN** the type checker emits `E4004`, saying that `toString()` takes no arguments

#### Scenario: toString of a struct is an extension
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }`, `enum Color { Red }`, `fun Point.toString(): String = x.toString() + "," + y.toString()`, `fun f(p: Point): String = p.toString()` and `fun g(c: Color): String = c.toString()` are checked
- **THEN** the type checker accepts `f` and the extension, and emits `E4006` for `toString` on `Color`
