# Type System Specification Delta

## ADDED Requirements

### Requirement: Heap Construction
A call `Heap(value)` MUST construct a `Heap<T>` holding `value`, where `T` is the call's explicit type argument (`Heap<Int?>(1)`), else the contents' type of the expected `Heap<T>`, else the type of `value`. The argument MUST be checked against `T` in a consuming position. Exactly one positional argument MUST be given, or the call is `E4004`.

#### Scenario: Boxing a value
- **WHEN** `enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }` is declared and `fun f(): Expr = Expr.Add(Heap(Expr.Lit(1)), Heap(Expr.Lit(2)))` and `fun g(): Heap<Int> { val h = Heap(1); return h }` are checked
- **THEN** both typecheck, and `h` has type `Heap<Int>`

#### Scenario: The expected type gives the contents' type
- **WHEN** `fun f(): Heap<Int?> = Heap(1)` and `fun g(): Heap<Int?> = Heap<Int?>(null)` are checked
- **THEN** both typecheck, the `1` being converted to `Int?` before it is boxed

#### Scenario: The boxed value is consumed
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: User): Heap<String> = Heap(u.name)` and `fun g(u: User): Heap<User> = Heap(u)` are checked
- **THEN** the type checker emits `E4017` for `u.name` and accepts `g`

#### Scenario: Wrong arguments of Heap rejected
- **WHEN** `fun f() { val a = Heap(); val b = Heap(1, 2); val c = Heap(value = 1); val d = Heap<Int>("x") }` is checked
- **THEN** the type checker emits `E4004` for each of the first three calls and `E4002` for `"x"`

#### Scenario: Borrows in the type argument rejected
- **WHEN** `fun f(a: Int) { val h = Heap<&Int>(&a) }` is checked
- **THEN** the type checker emits `E4022`

### Requirement: The Name Heap Is Built In
`Heap` in call position MUST name the built-in constructor whatever is in scope. A top-level function, struct, enum or `val` named `Heap` MUST be rejected with `E4010`, and `Heap` named without a call MUST be rejected with `E4001`.

#### Scenario: A declaration named Heap rejected
- **WHEN** a file declares `struct Heap(val x: Int)` or `fun Heap(x: Int): Int = x`
- **THEN** the type checker emits `E4010`, saying that the declaration would hide the built-in `Heap`

#### Scenario: Heap is not a value
- **WHEN** `fun f() { val make = Heap }` is checked
- **THEN** the type checker emits `E4001` for `Heap`

### Requirement: Heap Types Are Invariant
`Heap<U>` MUST be assignable to `Heap<T>` only when `U` and `T` are the same type, because a box cannot change the representation of its contents. A mismatch MUST be reported like any other, with help saying that the contents can be converted when the box is built.

#### Scenario: A box is not a box of a nullable
- **WHEN** `fun g(h: Heap<Int?>) {}` is declared and `fun f(h: Heap<Int>) { g(h) }` and `fun k(h: Heap<Int>): Heap<Int?> = h` are checked
- **THEN** the type checker emits `E4004` for `f` and `E4005` for `k`, each with help suggesting that the box be built as a `Heap<Int?>`

#### Scenario: Branches of different boxes have no join
- **WHEN** `fun f(c: Bool) { val h = if (c) Heap(1) else Heap(null) }` and `fun g(c: Bool): Heap<Int?> = if (c) Heap(1) else Heap(null)` are checked
- **THEN** the type checker emits `E4002` for `f`, and accepts `g`, whose branches are each built as a `Heap<Int?>`

## MODIFIED Requirements

### Requirement: Operators Read Through References
Operator operands (arithmetic, comparison, equality, logical, unary and range bounds), and `if` and subjectless-`when` conditions, MUST be typed after peeling every `&`, `&mut` and `Heap<T>` layer, so a borrowed or boxed value can be read by operators. Positions that expect a value of type `T` (arguments, initializers, returns, assignment values) accept a `&T` or a `Heap<T>` only by copying a `Copy` value out of it, as the `references` spec defines.

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

#### Scenario: Operators on boxed values
- **WHEN** `fun f(h: Heap<Int>, r: &Heap<Int>, c: Heap<Bool>): Int = if (c && h == r) h + r * 2 else -h` is checked
- **THEN** the function typechecks successfully

### Requirement: Output Built-In Arguments
`print` MUST take exactly one argument and `println` zero or one, given positionally, of type `Int`, `Double`, `Bool`, `Char` or `String`, or a reference or `Heap<T>` of one, at any depth. A `Copy` argument MUST be passed by value, copied out of references and boxes, and a `String` MUST be borrowed implicitly, as an argument of a `&String` parameter is, without being moved. Any other count, a named argument or another type MUST be rejected with `E4004`. A call MUST have type `Unit`.

#### Scenario: Printing scalars and strings
- **WHEN** `fun f(r: &Int, s: String) { print(1); println(2.5); println(true); println('c'); println(r); println(s); println(s); println() }` is checked
- **THEN** the file typechecks, and `s` is not moved by either call

#### Scenario: Printing an unsupported value
- **WHEN** `struct P { val x: Int }` is declared and `fun f(n: Int?) { println(P(1)); println(n); println({ 1 }) }` is checked
- **THEN** the type checker emits `E4004` for each of the three arguments, with help listing the printable types

#### Scenario: Wrong argument count or form
- **WHEN** `fun f() { print(); println(1, 2); println(value = 1) }` is checked
- **THEN** the type checker emits `E4004` for each of the three calls

#### Scenario: Printing boxed values
- **WHEN** `fun f(h: Heap<Int>, s: Heap<String>) { println(h); println(s); println(s) }` is checked
- **THEN** the file typechecks, and neither `h` nor `s` is moved

### Requirement: Scalars Have a Built-In toString
An `Int`, `Double`, `Bool` or `Char`, read through any `&`, `&mut` and `Heap<T>` layers, MUST have a built-in member function `toString()` taking no arguments and returning `String`, and `x?.toString()` on a nullable of one of these MUST have type `String?`. Arguments MUST be rejected with `E4004`. On any other type, `toString` MUST be an ordinary name, resolved to a field or an extension like any other member (`E4006` when none exists).

#### Scenario: Converting scalars to strings
- **WHEN** `fun f(n: Int, d: &Double, b: Bool, c: Char, m: Int?): String = n.toString() + d.toString() + b.toString() + c.toString() + (m?.toString() ?: "")` is checked
- **THEN** the file typechecks successfully

#### Scenario: toString takes no arguments
- **WHEN** `fun f(n: Int): String = n.toString(10)` is checked
- **THEN** the type checker emits `E4004`, saying that `toString()` takes no arguments

#### Scenario: toString of a struct is an extension
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }`, `enum Color { Red }`, `fun Point.toString(): String = x.toString() + "," + y.toString()`, `fun f(p: Point): String = p.toString()` and `fun g(c: Color): String = c.toString()` are checked
- **THEN** the type checker accepts `f` and the extension, and emits `E4006` for `toString` on `Color`

#### Scenario: toString through a box
- **WHEN** `fun f(h: Heap<Int>, r: &Heap<Char>): String = h.toString() + r.toString()` is checked
- **THEN** the file typechecks successfully
