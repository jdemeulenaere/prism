# Execution Specification Delta

## ADDED Requirements

### Requirement: Struct Values
Constructing a struct MUST evaluate its field values in source order and build a value holding them; reading a field MUST give the value last stored in it, and assigning a `var` field of a `var` binding, or through a mutable reference, MUST change that field only.

#### Scenario: Constructing and reading a struct
- **WHEN** `struct Point : Copy { var x: Int, var y: Int }` is declared and `fun main() { var p = Point(1, 2); p.y = 5; println(p.x); println(p.y) }` runs
- **THEN** it prints `1` and then `5`

### Requirement: Copies and Moves Are Independent Values
Copying or moving a struct, enum or nullable MUST give a value whose later changes do not affect the source, and changes to the source, where it is still usable, MUST not affect the copy. A move and a copy MUST run no user code.

#### Scenario: Copies are independent
- **WHEN** `struct Point : Copy { var x: Int, var y: Int }` is declared and `fun main() { val a = Point(1, 2); var b = a; b.x = 7; println(a.x); println(b.x) }` runs
- **THEN** it prints `1` and then `7`

#### Scenario: Moving a struct into a function
- **WHEN** `struct Counter { var n: Int }` and `fun take(c: Counter): Int = c.n` are declared and `fun main() { val c = Counter(4); println(take(c)) }` runs
- **THEN** it prints `4`

### Requirement: Aggregate Arguments and Results Are Values
A struct, enum or nullable passed to a function MUST be received as the value it had when the argument was evaluated, and a returned one MUST be the value the function built, even when the result is assigned to the place an argument came from.

#### Scenario: Returning a struct over its own argument
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `fun mirror(p: Point): Point = Point(p.y, p.x)` are declared and `fun main() { var p = Point(1, 2); p = mirror(p); println(p.x); println(p.y) }` runs
- **THEN** it prints `2` and then `1`

#### Scenario: An argument is not changed by a write through another argument
- **WHEN** `struct Point : Copy { var x: Int }` and `fun f(p: Point, r: &mut Point): Int { r.x = 9; return p.x }` are declared and `fun main() { var a = Point(1); println(f(a, &mut a)); println(a.x) }` runs
- **THEN** it prints `1` and then `9`

### Requirement: Borrows Refer to the Borrowed Place
A borrow of a value, of a field or of a nullable's payload MUST refer to that place itself, never to a copy: a write through a mutable borrow MUST change the borrowed place, and an assignment through a reference MUST replace the referent, so its owner sees the new value. Viewing a `&(T?)` as a `(&T)?` MUST give `null` when the nullable is `null` and otherwise a borrow of its payload.

#### Scenario: Writing through a borrow of a field
- **WHEN** `struct Point : Copy { var x: Int, var y: Int }` and `fun bump(r: &mut Int) { *r = r + 1 }` are declared and `fun main() { var p = Point(1, 2); bump(&mut p.x); println(p.x) }` runs
- **THEN** it prints `2`

#### Scenario: Writing through a reference replaces the referent
- **WHEN** `struct Point : Copy { var x: Int, var y: Int }` and `fun set(r: &mut Point, v: Point) { *r = v }` are declared and `fun main() { var a = Point(1, 2); set(&mut a, Point(5, 6)); println(a.x); println(a.y) }` runs
- **THEN** it prints `5` and then `6`

#### Scenario: Viewing a borrowed nullable
- **WHEN** `struct User : Copy { val nick: Int? }`, `fun show(n: (&Int)?): Int { val r = n ?: &0; return r }` and `fun nick(u: &User): Int = show(&u.nick)` are declared and a program prints `nick(&User(4))` and `nick(&User(null))`
- **THEN** it prints `4` and then `0`

### Requirement: When Over Enum Variants and Nullables
A `when` over an enum MUST run the branch of the subject's variant, and a `when` over a nullable MUST match a `null` pattern exactly when the subject is `null` and a value pattern exactly when the subject holds an equal value.

#### Scenario: When over variants
- **WHEN** `enum Color { Red, Green, Blue }` and `fun g(c: Color): Int = when (c) { Red -> 1; Green -> 2; Blue -> 3 }` are declared and a program prints `g(Color.Blue)` and `g(Color.Red)`
- **THEN** it prints `3` and then `1`

#### Scenario: When over a nullable
- **WHEN** `fun f(n: Int?): Int = when (n) { null -> 0; 1 -> 10; else -> 20 }` is declared and a program prints `f(null)`, `f(1)` and `f(2)`
- **THEN** it prints `0`, `10` and `20`

### Requirement: Equality of Structs, Enums and Nullables
`==` MUST compare structs field by field, enums by variant and then payload, and nullables as equal when both are `null` or both hold equal values, each field or payload with `==` of its type; `!=` MUST be its negation. Comparing borrows MUST compare what they refer to.

#### Scenario: Structural equality
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `enum Shape : Copy { Circle(Double), Square(Double), Empty }` are declared and a program prints `Point(1, 2) == Point(1, 2)`, `Point(1, 2) == Point(2, 1)`, `Shape.Circle(1.0) == Shape.Circle(2.0)`, `Shape.Circle(1.0) != Shape.Square(1.0)` and `Shape.Empty == Shape.Empty`
- **THEN** it prints `true`, `false`, `false`, `true` and `true`

#### Scenario: Equality of nullables
- **WHEN** a program declares `val a: Int? = null`, `val b: Int? = 1` and `val c: Int? = 1`, and prints `a == null`, `a == b`, `b == c` and `b != null`
- **THEN** it prints `true`, `false`, `true` and `true`

#### Scenario: Fields compare as their types do
- **WHEN** `struct D : Copy { val v: Double }` is declared and a program prints `D(0.0 / 0.0) == D(0.0 / 0.0)` and `D(0.0) == D(-0.0)`
- **THEN** it prints `false` and then `true`

#### Scenario: Comparing values that are not Copy
- **WHEN** `struct Counter { val n: Int }` is declared and `fun same(a: Counter, b: Counter): Bool = a == b` and a program prints `same(Counter(1), Counter(1))`
- **THEN** it prints `true`

### Requirement: Null-Safe Operators Short-Circuit
`a ?: b` MUST evaluate `b` only when `a` is `null`. `a?.m(args)` and `a?.f` MUST give `null` without evaluating `args` when `a` is `null`, and otherwise call `m` or read `f` on the value `a` holds.

#### Scenario: Safe navigation and Elvis operator
- **WHEN** `struct User : Copy { val age: Int }` and `fun age(u: User?): Int = u?.age ?: 0` are declared and a program prints `age(null)` and `age(User(3))`
- **THEN** it prints `0` and then `3`

#### Scenario: A safe call on null evaluates no argument
- **WHEN** `struct User : Copy { val age: Int }`, `fun User.older(by: Int): Int = age + by` and `fun next(): Int { println(9); return 1 }` are declared and `fun main() { val u: User? = null; println(u?.older(next()) ?: 0); val v: User? = User(3); println(v?.older(next()) ?: 0) }` runs
- **THEN** it prints `0`, then `9`, then `4`

### Requirement: Cloning a Value That Owns Nothing
`x.clone()` of a value whose type owns no storage MUST give a bitwise copy of `x`, with no allocation and no user code run.

#### Scenario: Copy value clone is a copy
- **WHEN** `struct Point : Copy { var x: Int }` is declared and `fun main() { var p = Point(1); val q = p.clone(); p.x = 2; println(q.x) }` runs
- **THEN** it prints `1`

#### Scenario: Clone of a Clone struct of scalars
- **WHEN** `struct Tally : Clone { var n: Int }` is declared and `fun main() { val a = Tally(3); var b = a.clone(); b.n = 4; println(a.n); println(b.n) }` runs
- **THEN** it prints `3` and then `4`

### Requirement: Ranges Are Values
A `Range` MUST be usable as a value like any `Copy` struct: passed to and returned from functions and stored in top-level values, keeping whether it includes its end.

#### Scenario: Passing ranges to a function
- **WHEN** `fun count(r: Range): Int { var n = 0; for (i in r) n += 1; return n }` is declared and a program prints `count(1..3)` and `count(1..<3)`
- **THEN** it prints `3` and then `2`

### Requirement: Top-Level Values of Aggregate Types
A top-level `val` of a struct, enum or nullable type MUST be initialized on first read like any top-level value, and every read MUST give the value its initializer built.

#### Scenario: Reading an imported top-level struct
- **WHEN** `lib/geo.prism` declares `public struct Point : Copy { val x: Int, val y: Int }` and `public val ORIGIN: Point = Point(0, 7)`, and `app/main.prism` imports `/lib/geo` and declares `fun main() { println(ORIGIN.y); val p = ORIGIN; println(p.x) }`
- **THEN** `prism run app/main.prism` prints `7` and then `0`
