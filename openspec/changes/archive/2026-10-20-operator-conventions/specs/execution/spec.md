## MODIFIED Requirements

### Requirement: Equality of Structs, Enums and Nullables
`==` MUST compare structs field by field, enums by variant and then payload, except a struct or enum with an `equals` operator (`operators`), which it MUST compare by calling it, and nullables as equal when both are `null` or both hold equal values, each field or payload with `==` of its type; `!=` MUST be its negation. Comparing borrows MUST compare what they refer to.

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

## ADDED Requirements

### Requirement: Operator Functions Run as Calls
A program using arithmetic, unary, ordering, membership or `invoke` operators on types with operator functions MUST run each operator as a call of its function, with the result the function returns, and free every value it builds exactly once.

#### Scenario: Vector arithmetic runs
- **WHEN** a program declares `struct Vector2 : Copy { val x: Int, val y: Int }` with `operator` functions `plus`, `minus`, `times(k: Int)` and `unaryMinus` computing componentwise, and `fun Vector2.toString(): String = "($x, $y)"`, and prints `Vector2(1, 2) + Vector2(3, 4)`, `-Vector2(1, 2) * 3` and `Vector2(5, 5) - Vector2(1, 2)`
- **THEN** `prism run --check-allocations` prints `(4, 6)`, `(-3, -6)` and `(4, 3)` and exits with status 0

#### Scenario: Ordering, membership and invoke run
- **WHEN** a program declares a `Version` with `operator fun Version.compareTo(other: &Version): Int`, a `Tags` holding a `List<String>` with `operator fun Tags.contains(name: &String): Bool`, and a `Counter` with `operator mut fun Counter.invoke(step: Int = 1): Int`, and prints `Version(1, 2) < Version(1, 10)`, `Version(2, 0) <= Version(1, 9)`, `"b" in Tags(listOf("a", "b"))`, `"c" !in Tags(listOf("a"))`, `3 in 0..<3` and, for `var c = Counter(0)`, `c()` then `c(step = 5)`
- **THEN** `prism run --check-allocations` prints `true`, `false`, `true`, `true`, `false`, `1` and `6` and exits with status 0

#### Scenario: Compound assignment through an operator function runs
- **WHEN** a program declares a `struct Text { val s: String }` with `operator consuming fun Text.plus(other: &Text): Text = Text(s + other.s)` and runs `var t = Text("a"); for (part in listOf("b", "c")) { t += Text(part.clone()) }; println(t.s)`
- **THEN** `prism run --check-allocations` prints `abc`, frees each intermediate `Text` once and exits with status 0

### Requirement: Operator Operands Run Left to Right
The operands of an operator use MUST be evaluated left to right, whichever operand is the function's receiver, and each exactly once, also for the indices of a compound assignment through `get` and `set`.

#### Scenario: Evaluation order of operands
- **WHEN** a program declares `fun noisy(label: String, n: Int): Int { println(label); return n }`, a `Tags` with a `contains` operator, a `fun tags(): Tags` printing `tags`, a `Matrix` with `get` and `set` operators, and evaluates `noisy("left", 1) in tags()`, then `m[noisy("row", 0), noisy("column", 1)] += 2.0` on a `var m` holding one row of two cells
- **THEN** `prism run --check-allocations` prints `left` before `tags`, then `row` and `column` once each, and the cell holds its old value plus `2.0`

### Requirement: Index Operators Run Their Accessors
Reading, borrowing and writing `a[...]` on a type with index operators MUST run `get`, the `mut` `get` or `set` as `operators` says, reading and writing the element the accessor reaches, and dropping a replaced element exactly once.

#### Scenario: A matrix read and written by index
- **WHEN** a program declares `struct Matrix { val columns: Int, var cells: List<Double> }` with `operator fun Matrix.get(row: Int, column: Int): Double` and `operator mut fun Matrix.set(row: Int, column: Int, value: Double)` over `cells[row * columns + column]`, and runs `var m = Matrix(2, listOf(0.0, 0.0, 0.0, 0.0)); m[1, 0] = 2.5; m[1, 0] += 1.0; m[0, 1] = m[1, 0] * 2.0; println(m[1, 0]); println(m[0, 1]); println(m[0, 0])`
- **THEN** `prism run --check-allocations` prints `3.5`, `7.0` and `0.0` and exits with status 0

#### Scenario: Elements changed in place through the mutable get
- **WHEN** a program declares `struct Item { var name: String, var count: Int }` and `struct Bag { var items: List<Item> }` with a shared and a `mut` `get` borrowing `items[index]`, and runs `var b = Bag(listOf(Item("a", 0), Item("b", 0))); b[0].count += 2; b[1].name = "c"; b[0] = Item("d", 5); val first: &Item = &b[0]; println(first.name); println(b[0].count); println(b[1].name)`
- **THEN** `prism run --check-allocations` prints `d`, `5` and `c`, frees the replaced `Item("a", 2)` and the string `"b"` once each, and exits with status 0

### Requirement: Cursor Loops Run the Protocol
A `for` loop over a type with the cursor protocol MUST call `iterator` once, then `hasNext` before each iteration and `next` once per iteration, running the body with the element `next` returned, honour `break` and `continue`, and drop the cursor, the elements `next` produced by value, and a temporary iterable exactly once.

#### Scenario: Iterating a custom collection runs
- **WHEN** a program declares a `Bag` of `Item(name: String)` with the cursor protocol over an `Int` index (shared `next` returning `&Item`, `mut` `next` returning `&mut Item`), and runs `var b = Bag(listOf(Item("a"), Item("b"), Item("c"))); for (item in &mut b) { item.name += "!" }; for (item in b) { if (item.name == "b!") continue; println(item.name) }`
- **THEN** `prism run --check-allocations` prints `a!` and `c!` and exits with status 0

#### Scenario: A cursor producing owned values runs
- **WHEN** a program declares `struct Countdown : Copy { val from: Int }` whose protocol's `next` returns `n.toString()` for the cursor `n` and counts down to 1, and runs `for (s in Countdown(3)) { if (s == "1") break; println(s + "...") }`
- **THEN** `prism run --check-allocations` prints `3...` and `2...`, frees each string `next` returned once, the one the `break` left included, and exits with status 0

#### Scenario: Iterating a temporary collection runs
- **WHEN** a program declares the `Bag` protocol above and `fun make(): Bag = Bag(listOf(Item("x"), Item("y")))`, and runs `for (item in make()) println(item.name)`
- **THEN** `prism run --check-allocations` prints `x` and `y`, frees the temporary `Bag` after the loop, and exits with status 0

### Requirement: Equality Operators Run Wherever Values Are Compared
A program comparing values of a type with an `equals` operator MUST get that function's answer from `==` and `!=` on the values, on structs and enums holding them, on nullables and boxes of them, and from list equality, `contains`, `indexOf` and `in`.

#### Scenario: Equality through an equals operator runs
- **WHEN** a program declares `struct User { val id: Int, val name: String }` with `operator fun User.equals(other: &User): Bool = id == other.id` and `struct Team { val lead: User, val size: Int }`, and prints `User(1, "a") == User(1, "b")`, `User(1, "a") != User(2, "a")`, `Team(User(1, "a"), 3) == Team(User(1, "z"), 3)`, `Heap(User(2, "a")) == Heap(User(2, "b"))`, a `User?` holding `User(1, "a")` compared with `User(1, "c")`, `listOf(User(1, "a")) == listOf(User(1, "b"))`, `User(3, "q") in listOf(User(3, "r"))` and `listOf(User(1, "a"), User(2, "b")).indexOf(User(2, "x"))`
- **THEN** `prism run --check-allocations` prints `true`, `true`, `true`, `true`, `true`, `true`, `true` and `1`, and exits with status 0
