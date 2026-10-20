## MODIFIED Requirements

### Requirement: Scalars Have a Built-In toString
An `Int`, `Double`, `Bool` or `Char`, read through any `&`, `&mut` and `Heap<T>` layers, MUST have the member function `toString()` that `/prism/core` declares in Prism (`public fun Int.toString(): String`), taking no arguments and returning `String`, checked and called as an extension with a shared receiver. `x?.toString()` on a nullable of one of these MUST have type `String?`. Arguments MUST be rejected with `E4004`. On any other type, `toString` MUST be an ordinary name, resolved to a field or an extension like any other member (`E4006` when none exists).

#### Scenario: Converting scalars to strings
- **WHEN** `fun f(n: Int, d: &Double, b: Bool, c: Char, m: Int?): String = n.toString() + d.toString() + b.toString() + c.toString() + (m?.toString() ?: "")` is checked
- **THEN** the file typechecks successfully

#### Scenario: toString takes no arguments
- **WHEN** `fun f(n: Int): String = n.toString(10)` is checked
- **THEN** the type checker emits `E4004` for the argument, `toString()` being declared with no parameter

#### Scenario: toString of a struct is an extension
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }`, `enum Color { Red }`, `fun Point.toString(): String = x.toString() + "," + y.toString()`, `fun f(p: Point): String = p.toString()` and `fun g(c: Color): String = c.toString()` are checked
- **THEN** the type checker accepts `f` and the extension, and emits `E4006` for `toString` on `Color`

#### Scenario: toString through a box
- **WHEN** `fun f(h: Heap<Int>, r: &Heap<Char>): String = h.toString() + r.toString()` is checked
- **THEN** the file typechecks successfully
