# Operators Specification

## Purpose
Defines Kotlin-style operator conventions: the `operator` modifier, which functions arithmetic, unary, ordering, equality, membership, indexing, call and `for` syntax reach on types other than the built-in ones, how such a use resolves (built-in operators first, then the receiver's operator function through the file and its imports), the shape each convention requires (`E4035`), equality operators as part of their type, indexed places through shared and mutable accessors, `set`, `invoke`, the cursor protocol of `for` loops, compound assignment through operator functions, and the cost of an operator use.

## Requirements

### Requirement: Operator Functions Are Marked
Operator syntax MUST call only extensions declared with the `operator` modifier. An extension named like a convention without it MUST stay an ordinary extension that no operator reaches, and an operator function MUST stay callable by name (`a.plus(b)`). A use that finds only an unmarked function MUST be rejected as when none exists, with help saying to declare it `operator`.

#### Scenario: A marked function serves its operator
- **WHEN** `struct Vector2 : Copy { val x: Int, val y: Int }` and `operator fun Vector2.plus(other: Vector2): Vector2 = Vector2(x + other.x, y + other.y)` are declared and `fun f(a: Vector2, b: Vector2): Vector2 = a + b` and `fun g(a: Vector2, b: Vector2): Vector2 = a.plus(b)` are checked
- **THEN** the file typechecks, and both bodies call `plus`

#### Scenario: An unmarked function is not an operator
- **WHEN** `struct Vector2 : Copy { val x: Int, val y: Int }` and `fun Vector2.plus(other: Vector2): Vector2 = other` are declared and `fun f(a: Vector2, b: Vector2): Vector2 = a + b` is checked
- **THEN** the type checker emits `E4002` at `a + b`, with help saying that `plus` must be declared `operator fun Vector2.plus` to serve `+`

### Requirement: Arithmetic and Unary Operators Call Their Functions
`a + b`, `a - b`, `a * b`, `a / b` and `a % b` MUST call `a.plus(b)`, `a.minus(b)`, `a.times(b)`, `a.div(b)` and `a.rem(b)`, and `-a` and `!a` MUST call `a.unaryMinus()` and `a.not()`, when the built-in operator does not apply ("Built-In Operators Come First"). The expression MUST have the function's result type.

#### Scenario: Vector arithmetic
- **WHEN** `struct Vector2 : Copy { val x: Int, val y: Int }` is declared with `operator` functions `plus(other: Vector2): Vector2`, `minus(other: Vector2): Vector2`, `times(k: Int): Vector2` and `unaryMinus(): Vector2` on `Vector2`, and `fun f(a: Vector2, b: Vector2): Vector2 = -(a + b * 2) - a` is checked
- **THEN** the file typechecks, with the usual precedence: `minus` of `unaryMinus` of `plus` of `a` and `times` of `b` and `2`, then `a`

#### Scenario: A result of another type
- **WHEN** `struct Dp : Copy { val value: Double }`, `struct Ratio : Copy { val value: Double }` and `operator fun Dp.div(other: Dp): Ratio = Ratio(value / other.value)` are declared and `fun f(a: Dp, b: Dp): Ratio = a / b` is checked
- **THEN** the file typechecks, and `a / b` has type `Ratio`

#### Scenario: Negation through not
- **WHEN** `struct Flags : Copy { val bits: Int }` and `operator fun Flags.not(): Flags = Flags(-1 - bits)` are declared and `fun f(a: Flags): Flags = !a` is checked
- **THEN** the file typechecks, and `!a` calls `not`

### Requirement: Built-In Operators Come First
An operator MUST use its built-in meaning whenever the built-in operator accepts its operand types after their `&`, `&mut` and `Heap<T>` layers, whatever operator functions are visible. Otherwise it MUST call the operator function of its receiver's type: the left operand of a binary operator, the right of `in`, the indexed, called or iterated value. A built-in receiver type MAY have operator functions for other operand types.

#### Scenario: An operator function on a built-in receiver
- **WHEN** `struct Dp : Copy { val value: Double }` and `operator fun Int.times(other: Dp): Dp = Dp(other.value * 2.0)` are declared and `fun f(d: Dp): Dp = 2 * d` and `fun g(): Int = 2 * 3` are checked
- **THEN** the file typechecks: `2 * d` calls `times`, and `2 * 3` is the built-in multiplication

#### Scenario: Mismatched scalars stay mismatches
- **WHEN** `fun f(): Double = 1 + 2.0` is checked in a file declaring no operator functions
- **THEN** the type checker emits `E4002`, as before

### Requirement: Operator Functions a Built-In Operator Covers Are Rejected
An operator function the built-in operator always serves could never be called and MUST be rejected with `E4010`, saying that the built-in operator takes precedence: an arithmetic, unary or ordering function on `Int`, `Double`, `Char`, `Bool` or `String` whose operand types the built-in operator accepts, and `get`, `set`, `iterator`, `hasNext`, `next` or `contains` on `List` or `Range`, or `get` or `set` on `String`.

#### Scenario: Redefining a built-in operator rejected
- **WHEN** a file declares `operator fun Int.plus(other: Int): Int = 0`, `operator fun Double.compareTo(other: &Double): Int = 0`, `operator fun Bool.not(): Bool = true` and `operator fun <T> List<T>.get(index: Int): &T = &this[0]`
- **THEN** the type checker emits `E4010` for each, saying that the built-in operator always takes precedence

#### Scenario: Operand types the built-in operator rejects
- **WHEN** a file declares `operator fun String.times(count: Int): String = this.clone()` and `operator fun String.compareTo(other: &String): Int = length - other.length`
- **THEN** the file typechecks, and `"a" * 3` and `"a" < "bc"` call them

### Requirement: Operator Functions Resolve Like Extensions
An operator use MUST find its function as a call of that name on the receiver finds an extension: members, then the file's own extensions, then its imports' (`extension-functions`), so another module's operator function is used only where that module is imported. Several imported matches MUST be `E4012`. None MUST be `E4002`, with help naming the function to declare (`operator fun Vector2.plus(other: Vector2)`), and first the import of the receiver type's module if missing.

#### Scenario: An imported operator function
- **WHEN** `geometry/vector.prism` exports `public struct Vector2 : Copy { val x: Int, val y: Int }` and `public operator fun Vector2.plus(other: Vector2): Vector2`, and `app/main.prism` imports `/geometry/vector` and declares `fun f(a: Vector2, b: Vector2): Vector2 = a + b`
- **THEN** `prism check app/main.prism` succeeds, while the same body in a file that receives `Vector2` values from another module without importing `/geometry/vector` reports `E4002` with help saying to add `import /geometry/vector` or naming `operator fun Vector2.plus`

#### Scenario: No operator function
- **WHEN** `struct Point : Copy { val x: Int }` is declared and `fun f(a: Point, b: Point): Point = a * b` is checked
- **THEN** the type checker emits `E4002` at `a * b`, whose message names `*`, `Point` and `Point`, and whose help suggests `operator fun Point.times(other: Point)`

### Requirement: An Operator Use Is a Call
An operator use MUST be typed, borrowed and evaluated as the call it spells: the receiver passed by the function's receiver mode as a method receiver is, each other operand as an argument of its parameter (borrowed for `&T`, moved or copied otherwise), and the result tied to its inputs by the elision rules. Operands MUST be evaluated left to right. A nullable receiver MUST be `E4002`, with help suggesting `?.` and the call.

#### Scenario: Operands borrowed or moved as parameters say
- **WHEN** `struct Name { val text: String }`, `operator fun Name.plus(other: &Name): Name = Name(text + other.text)` and `fun take(n: Name) {}` are declared and `fun f(a: Name, b: Name) { val c = a + b; take(a); take(b); take(c) }` is checked
- **THEN** the file passes `prism check`: `a` is borrowed as the receiver and `b` as a `&Name` argument, so neither is moved by `+`

#### Scenario: A consuming operator moves its receiver
- **WHEN** `struct Name { val text: String }`, `operator consuming fun Name.plus(other: &Name): Name = Name(text + other.text)` and `fun take(n: Name) {}` are declared and `fun f(a: Name, b: Name) { val c = a + b; take(a) }` is checked
- **THEN** the borrow checker emits `E5001` at `a` in `take(a)`, labelling `a + b` as the move

#### Scenario: A nullable operand
- **WHEN** `struct Vector2 : Copy { val x: Int }` and `operator fun Vector2.plus(other: Vector2): Vector2 = other` are declared and `fun f(a: Vector2?, b: Vector2): Vector2? = a + b` is checked
- **THEN** the type checker emits `E4002`, with help suggesting `a?.plus(b)`

### Requirement: Operator Signatures Are Checked
An `operator` function MUST be an extension named by a convention of this spec, without `compose`, a context parameter or default values (except `invoke`), with the parameters, receiver modes and result its convention states. Any other `operator` declaration MUST be rejected with `E4035`, whose message names the convention and the rule broken.

#### Scenario: Wrong shapes rejected
- **WHEN** a file declares `struct P : Copy { val x: Int }` and `operator fun P.plus(): P = this`, `operator fun P.compareTo(other: &P): Bool = true`, `operator fun P.size(): Int = 0`, `operator fun double(p: P): P = p`, `operator mut fun P.minus(other: P): P = other` and `operator fun P.times(k: Int = 2): P = this`
- **THEN** the type checker emits `E4035` for each: `plus` takes one parameter, `compareTo` returns `Int`, `size` is no operator convention, an operator function needs a receiver, arithmetic receivers are shared or consuming, and only `invoke` may have default values

### Requirement: The Shapes of the Conventions
Each convention MUST have this shape: `plus`, `minus`, `times`, `div`, `rem` one parameter, `unaryMinus` and `not` none, all with a shared or consuming receiver; `compareTo` a shared receiver, one parameter, result `Int`; `contains` a shared receiver, one parameter, result `Bool`; `equals` as "Equality Operators Belong to Their Type" says. Indexing, `invoke` and the cursor protocol have their own requirements.

#### Scenario: Accepted shapes
- **WHEN** a file declares `struct Money : Copy { val cents: Int }`, `struct Bag { val items: List<Money> }`, `operator fun Money.plus(other: Money): Money`, `operator consuming fun Bag.plus(other: Bag): Bag`, `operator fun Money.unaryMinus(): Money`, `operator fun Money.compareTo(other: &Money): Int` and `operator fun Bag.contains(m: &Money): Bool`, each with a body
- **THEN** the file typechecks without `E4035`

### Requirement: Ordering Operators Call compareTo
`a < b`, `a <= b`, `a > b` and `a >= b` MUST call `a.compareTo(b)` when the built-in comparison does not apply, and compare its `Int` result with `0` by the same operator. Comparisons MUST stay non-associative, and `==` and `!=` MUST never call `compareTo`.

#### Scenario: Comparing versions
- **WHEN** `struct Version : Copy { val major: Int, val minor: Int }` and `operator fun Version.compareTo(other: &Version): Int = if (major != other.major) major - other.major else minor - other.minor` are declared and `fun f(a: Version, b: Version): Bool = a < b || a >= b` is checked
- **THEN** the file typechecks, each comparison calling `compareTo` and comparing its result with `0`

### Requirement: Membership Operators Call contains
`a in b` MUST call `b.contains(a)` and `a !in b` MUST be its negation, with type `Bool`, `a` evaluated before `b`. On a `Range` both MUST be built in, testing `start <= a` and `a` below or at the end as the range says; on a `List`, `contains` is its intrinsic member (`collections`).

#### Scenario: Membership of a custom set, a list and a range
- **WHEN** `struct Tags { val names: List<String> }` and `operator fun Tags.contains(name: &String): Bool = names.contains(name)` are declared and `fun f(t: &Tags, xs: &List<Int>, n: Int): Bool = "a" in t && n !in xs && n in 0..<10` is checked
- **THEN** the file typechecks: the first operand calls `contains` of `Tags`, the second negates the `contains` member of `List`, and the third is the built-in range test

### Requirement: Equality Operators Belong to Their Type
`equals` MUST be declared as `operator fun T.equals(other: &T): Bool`, in the file declaring `T`, a non-generic struct or enum, with `T`'s visibility; anything else is `E4035`, and a second one a duplicate declaration (`E4010`). It MUST be part of `T`'s definition: `==` on `T` uses it in every file, whatever that file imports, and a built-in type cannot have one.

#### Scenario: Equality operators outside their type's file rejected
- **WHEN** `models/user.prism` exports `public struct User { val id: Int, val name: String }`, and `app/main.prism` imports `/models/user` and declares `operator fun User.equals(other: &User): Bool = id == other.id` and `operator fun Int.equals(other: &Int): Bool = true`
- **THEN** `prism check app/main.prism` emits `E4035` for each, saying that a type's `equals` is declared in the type's own file

#### Scenario: Equality operator shapes rejected
- **WHEN** a file declares `public struct P { val x: Int }`, `struct Box<T> { val v: T }`, `operator fun P.equals(other: &P): Bool = true`, `operator fun <T> Box<T>.equals(other: &Box<T>): Bool = true` and `struct Q { val x: Int }` with `operator fun Q.equals(other: Q): Bool = true`
- **THEN** the type checker emits `E4035` for `P`'s (private for a public type), `Box`'s (a generic type) and `Q`'s (the parameter is `&Q`)

### Requirement: A Type's Equality Operator Replaces Structural Equality
Wherever `==` or `!=` compares two values of a type with an `equals` operator, it MUST call that function instead of comparing fields or payloads: directly, as a field or payload of another type, inside nullables and boxes, as elements of lists (`==`, `contains`, `indexOf`), in `when` value patterns, and where compose compares kept arguments or written state values. `!=` MUST negate it.

#### Scenario: Equality by identity field
- **WHEN** `struct User { val id: Int, val name: String }` and `operator fun User.equals(other: &User): Bool = id == other.id` are declared and `fun f(a: &User, b: &User): Bool = a == b` is checked
- **THEN** the file typechecks, and comparing `User(1, "a")` with `User(1, "b")` gives `true` (`execution`)

### Requirement: Indexing Calls get
`a[i1, ..., in]` on a type other than `List` and `String` MUST call `a.get(i1, ..., in)`, which takes one or more parameters. A `get` returning `&E` MUST make `a[...]` a place of type `E` behind that borrow: read by copying a `Copy` value out, borrowed where it lives (`&a[i]` is the borrow `get` returned), and a method receiver. A `get` returning a value MUST make `a[...]` that value, a temporary.

#### Scenario: Reading elements through a borrowing get
- **WHEN** `struct Item { val name: String, var count: Int }`, `struct Bag { var items: List<Item> }`, `operator fun Bag.get(index: Int): &Item = &items[index]` and `fun show(s: &String) {}` are declared and `fun f(b: &Bag): Int { show(b[0].name); val first: &Item = &b[0]; return b[1].count + first.count }` is checked
- **THEN** the file typechecks, `b[0].name` is borrowed where it lives and `first` borrows the element `get`'s borrow points at

#### Scenario: A computed element
- **WHEN** `struct Matrix { val columns: Int, var cells: List<Double> }` and `operator fun Matrix.get(row: Int, column: Int): Double = cells[row * columns + column]` are declared and `fun f(m: &Matrix): Double = m[1, 2] + m[0, 0]` is checked
- **THEN** the file typechecks, each index calling `get` with two arguments

#### Scenario: An element borrow outlives the accessor's temporary
- **WHEN** `struct Bag { var items: List<String> }` and `operator fun Bag.get(index: Int): &String = &items[index]` are declared and `fun first(b: &Bag): &String = &b[0]` and `fun f(b: Bag) { val s = &b[0]; println(s) }` are checked
- **THEN** the file passes `prism check`: each borrow derives from `b` through `get`, tied to its receiver by elision rule 2

#### Scenario: Holding an element across a change rejected
- **WHEN** `struct Bag { var items: List<String> }`, `operator fun Bag.get(index: Int): &String = &items[index]` and `mut fun Bag.add(s: String) { items.add(s) }` are declared and `fun f() { var b = Bag(listOf("a")); val s = &b[0]; b.add("c"); println(s) }` is checked
- **THEN** the borrow checker emits `E5003` at `b.add("c")`, labelling the borrow `&b[0]`

### Requirement: Mutable Index Positions Call the Mutable get
A type MAY declare two `get` functions of one receiver type constructor and context, a shared one and a `mut` one returning `&mut E`, the only extensions besides `next` that may differ by receiver mode alone. Where a `List` element would need a mutable list (a field write, `&mut a[i]`, a `mut fun` receiver, an assignment without `set`), `a[...]` MUST call the `mut` one; with none, it MUST be `E4003`, naming it.

#### Scenario: Writing through the mutable get
- **WHEN** `struct Item { val name: String, var count: Int }`, `struct Bag { var items: List<Item> }`, `operator fun Bag.get(index: Int): &Item = &items[index]`, `operator mut fun Bag.get(index: Int): &mut Item = &mut items[index]` and `mut fun Item.reset() { count = 0 }` are declared and `fun f(b: &mut Bag) { b[0].count += 1; b[1].reset(); val m: &mut Item = &mut b[0]; m.count = 2; b[0] = Item("z", 0) }` is checked
- **THEN** the file typechecks: each statement calls the `mut` `get`, and the assignment drops the old element and moves the new one in

#### Scenario: No mutable get
- **WHEN** `struct Bag { var items: List<Item> }` and only `operator fun Bag.get(index: Int): &Item = &items[index]` are declared and `fun f(b: &mut Bag) { b[0].count = 1 }` is checked
- **THEN** the type checker emits `E4003`, with help suggesting `operator mut fun Bag.get(index: Int): &mut Item`

#### Scenario: Mutable get shapes
- **WHEN** a file declares `operator mut fun Bag.get(index: Int): &Item` and `operator consuming fun Bag.get(index: Int): Item`
- **THEN** the type checker emits `E4035` for each: a `mut` `get` returns `&mut E`, and `get` has a shared or `mut` receiver

### Requirement: Writing an Indexed Element Calls set
`a[...] = v` MUST call `a.set(..., v)` when `set` is declared, a `mut fun` whose last parameter takes the value, and otherwise write through the `mut` `get`. `a[...] op= v` MUST call `set` with `get`'s element (copied out of a borrow) and `v` combined by `op`, each index evaluated once, or else update through the `mut` `get` in place.

#### Scenario: A matrix written through set
- **WHEN** `struct Matrix { val columns: Int, var cells: List<Double> }`, `operator fun Matrix.get(row: Int, column: Int): Double = cells[row * columns + column]` and `operator mut fun Matrix.set(row: Int, column: Int, value: Double) { cells[row * columns + column] = value }` are declared and `fun f(m: &mut Matrix, k: Int) { m[0, 1] = 2.0; m[k, k] += 1.5 }` is checked
- **THEN** the file typechecks: the first statement calls `set`, and the second evaluates `k` once for each index, calls `get`, adds `1.5` and calls `set`

#### Scenario: set needs a mutable receiver
- **WHEN** `operator fun Matrix.set(row: Int, column: Int, value: Double) {}` is declared
- **THEN** the type checker emits `E4035`, saying that `set` has a `mut` receiver

### Requirement: Calling a Value Calls invoke
A call whose callee is a value whose type, after its `&`, `&mut` and `Heap<T>` layers, is not a function type (a local, a parameter, a field, a top-level `val` or any expression) MUST call that type's `invoke` with the call's arguments, named arguments and defaults included. `invoke` MAY have any receiver mode and parameters.

#### Scenario: A callable counter
- **WHEN** `struct Counter { var count: Int }` and `operator mut fun Counter.invoke(step: Int = 1): Int { count += step; return count }` are declared and `fun f(): Int { var c = Counter(0); c(); c(step = 2); return c() }` and `fun g(c: &Counter): Int = c()` are checked
- **THEN** the type checker accepts `f`, each call borrowing `c` mutably, and emits `E4003` for `g`, whose receiver is a shared borrow

### Requirement: For Loops Over a Cursor
`for (x in y)` over a type other than `Range` and `List` MUST use the cursor protocol: `iterator()` (shared) returns a cursor of a type `K` holding no reference outside function types; the loop runs while `hasNext(cursor: &K): Bool` (shared) is true, binding `next(cursor: &mut K)`'s result to `x` each time. `y` MUST be evaluated once and borrowed for the whole loop, as a list is.

#### Scenario: Iterating a custom collection
- **WHEN** `struct Item { val name: String }`, `struct Bag { var items: List<Item> }`, `operator fun Bag.iterator(): Int = 0`, `operator fun Bag.hasNext(cursor: &Int): Bool = cursor < items.size` and `operator fun Bag.next(cursor: &mut Int): &Item { val i: Int = cursor; *cursor = i + 1; return &items[i] }` are declared and `fun f(b: &Bag) { for (item in b) { if (item.name == "stop") break; println(item.name) } }` is checked
- **THEN** the file typechecks, `item` has type `&Item`, and `b` is borrowed for the loop

#### Scenario: Changing a collection while iterating it rejected
- **WHEN** the same `Bag` protocol and `mut fun Bag.add(item: Item) { items.add(item) }` are declared and `fun f() { var b = Bag(listOf()); for (item in b) { b.add(Item("x")) } }` is checked
- **THEN** the borrow checker emits `E5003` at `b.add(...)`, labelling the loop's borrow of `b`

#### Scenario: Cursor protocol shapes rejected
- **WHEN** a file declares `operator fun Bag.iterator(): &Item`, `operator fun Bag.hasNext(cursor: Int): Bool`, `operator mut fun Bag.iterator(): Int` and `operator fun Bag.next(cursor: &Int): &Item`
- **THEN** the type checker emits `E4035` for each

#### Scenario: A missing part of the protocol
- **WHEN** `struct Bag { var items: List<Int> }` and `operator fun Bag.iterator(): Int = 0` are declared and `fun f(b: &Bag) { for (x in b) {} }` is checked
- **THEN** the type checker emits `E4002` at `b`, saying that iterating a `Bag` needs `operator fun Bag.hasNext(cursor: &Int): Bool` and `operator fun Bag.next(cursor: &mut Int)`

### Requirement: Loop Variables Are What next Returns
The loop variable MUST be a fresh immutable binding of `next`'s result type in each iteration: a `&E` borrows an element, a value is owned and dropped at the end of the iteration. Iterating a `&mut` iterable (`for (x in &mut y)`, or `y` of a `&mut` type, as for lists) MUST call a `mut` `next` returning `&mut E`, a second `next` of the receiver; with none, a written `&mut y` MUST be `E4003`, naming it, and a `y` of a `&mut` type iterates shared. The cursor MUST be dropped when the loop ends.

#### Scenario: Iterating mutably through a mutable next
- **WHEN** the `Bag` protocol above also declares `operator mut fun Bag.next(cursor: &mut Int): &mut Item { val i: Int = cursor; *cursor = i + 1; return &mut items[i] }` with `struct Item { var name: String }`, and `fun f(b: &mut Bag) { for (item in b) { item.name = "x" } }`, `fun g() { var b = Bag(listOf()); for (item in b) { item.name = "x" } }` and `fun h() { var b = Bag(listOf()); for (item in &mut b) { item.name = "x" } }` are checked
- **THEN** the type checker accepts `f` and `h`, which iterate a `&mut Bag` as a list loop iterates a `&mut List<T>`, so `item` has type `&mut Item`, and emits `E4003` for `g`, whose `item` is a shared borrow, with help suggesting `for (item in &mut b)`

#### Scenario: Values produced by next
- **WHEN** `struct Countdown : Copy { val from: Int }`, `operator fun Countdown.iterator(): Int = from`, `operator fun Countdown.hasNext(cursor: &Int): Bool = cursor > 0` and `operator fun Countdown.next(cursor: &mut Int): String { val n: Int = cursor; *cursor = n - 1; return n.toString() }` are declared and `fun f() { for (s in Countdown(3)) println(s) }` is checked
- **THEN** the file typechecks, `s` has type `String`, and each `s` is dropped at the end of its iteration

### Requirement: Compound Assignment Through Operator Functions
`a op= b`, where the built-in operator does not apply to `a`'s and `b`'s types, MUST be `a = a op b` with the target place evaluated once: the operator function receives the target as its receiver (borrowed, or moved for a consuming receiver), and its result, of the target's type (`E4002` otherwise), is assigned to the target. The target MUST be writable (`E4003`), and for a consuming receiver a whole binding, which a move may take (`E4017` otherwise).

#### Scenario: Accumulating vectors
- **WHEN** `struct Vector2 : Copy { val x: Int, val y: Int }` and `operator fun Vector2.plus(other: Vector2): Vector2 = Vector2(x + other.x, y + other.y)` are declared and `fun f(vs: &List<Vector2>): Vector2 { var total = Vector2(0, 0); for (v in vs) { total += v }; return total }` and `fun g(a: Vector2) { a += a }` are checked
- **THEN** the type checker accepts `f` and emits `E4003` for `g`

#### Scenario: A consuming operator in a compound assignment
- **WHEN** `struct Text { val s: String }` and `operator consuming fun Text.plus(other: &Text): Text = Text(s + other.s)` are declared and `fun f(t: Text): Text { var a = Text("x"); a += t; return a }` is checked
- **THEN** the file passes `prism check`: `a` is moved into `plus` and the result assigned back to it

#### Scenario: A consuming operator on a list element rejected
- **WHEN** the same `Text` and `plus` are declared and `fun f(xs: &mut List<Text>, i: Int, t: &Text) { xs[i] += t }` is checked
- **THEN** the type checker emits `E4017`: the consuming `plus` would move the element out of the list

### Requirement: Operator Uses Cost a Lookup Each
Typing an operator use MUST cost at most a constant number of extension table lookups (one per function a convention calls, two for an index place), with no search over overloads or candidate types, so checking a file stays linear in its operator uses.

#### Scenario: Many operator uses check in linear time
- **WHEN** `compile_timing`'s `operator-uses` shape generates files of 256, 1,024 and 4,096 functions, each using `+`, `<`, `in`, `==` on a type with `equals`, an index place and a cursor loop on user types, and runs under `PRISM_ASSERT_TIMING=1`
- **THEN** checking time grows by at most 4.6 times per fourfold growth


## Design Rationale
Decisions from `operator-conventions` (part L1 of agreed item A4 of the post–Milestone 2 review, user direction 2026-10-20; the rest planned autonomously on 2026-10-20, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Kotlin's conventions, resolved by name and marked `operator`** (D1). An operator use is a call of a function with a conventional name on the receiver's type (`a + b` is `a.plus(b)`), so library types written in Prism get Kotlin's operator syntax without a compiler intrinsic. The modifier is required, as in Kotlin, so a function that happens to be named `get` or `contains` never becomes an operator, and the signature checks know which functions claim a convention. Operator functions are ordinary extensions, callable by name. Rejected: traits or interfaces (a conformance system, coherence rules and bounds, deferred until traits exist; a later trait can be satisfied by the same functions) and no modifier.
- **Which conventions** (D2). `plus`, `minus`, `times`, `div`, `rem`, `unaryMinus`, `not`, `compareTo`, `equals`, `contains`, `get`, `set`, `invoke` and `iterator`/`hasNext`/`next`: the ROADMAP's list and what the library rewrite needs (`Dp` arithmetic and ordering, `Int.times(Dp)`, flag sets, membership, indexing and iterating list-like types, equality by identity). Deferred, each in `ROADMAP.md`: `rangeTo`/`rangeUntil` (ranges stay `Int`s), `plusAssign` and the other assignment operators, `unaryPlus`, `inc`/`dec`, `componentN`, `getValue`/`setValue`, `hashCode`, overloads by parameter type, generic `equals`, composable operators and operators with a context parameter, and `in` patterns over containers.
- **Built-in operators first, then the receiver's operator function through the file and its imports** (D3). Built-in operators act as members: whenever the built-in operator accepts the operand types after their layers it applies, so `1 + 2` never costs a lookup and an import cannot change it; an operator function it would always shadow is `E4010` at its declaration, while functions for other operand types (`operator fun Int.times(other: Dp)`) are allowed. Otherwise the receiver's function is looked up as a method call's is (members, the file's own extensions, its imports'), so an operator function is used only where its module is imported, as the explicit-library direction implies; a missing one names the declaration to add, after the import of the receiver type's module when the file lacks it (review fix). One function per receiver, name and context, as for every extension: overloading by parameter type would need overload resolution per use. Rejected: operator functions first, operators found without imports (a prelude of operators), looking up the right operand's type too (Python's `__radd__`).
- **An operator use is exactly the call it spells** (D4). The receiver is passed by the function's receiver mode, each other operand as an argument of its parameter, the result tied to the inputs by the elision rules; operands are evaluated left to right, and a place receiver is borrowed after the other operands are evaluated, as a method receiver is. A library type chooses copy, borrow or move per operand through its signature. A nullable receiver is `E4002` with help suggesting `?.` and the call. Rejected: operands always borrowed (no buffer reuse for owning types) and safe operators on nullables.
- **Signatures checked against their convention with one code, `E4035`** (D5). A fixed check per declaration against a table of the conventions' shapes (receiver modes, parameters, result; no `compose`, context parameter or defaults except on `invoke`). A use that cannot be typed is reported at the use with the existing codes, since each declaration may be valid alone. Rejected: Kotlin's looser shapes, reusing `E4004`/`E4010`, a parse error for a missing receiver.
- **`equals` belongs to its type and replaces structural equality everywhere** (D6). Equality is used far from any `==` (glue for fields, nullables, boxes, lists, `when` value patterns, compose's kept arguments, `remember` keys, closure environments and state writes), so a scoped `equals` would make `a == b` and `listOf(a).contains(b)` disagree across files. It is declared in the type's own file, non-generic, with the type's visibility, recorded in the type's ABI record and reachable definition, and called by every `==` of the type in every file without an import. Generic types are deferred (an instance planned from glue no body names), and `==` on a type parameter stays `E4002`. Rejected: a scoped operator, direct `==` only, deferring it, any file of the package (breaks single-file ABI extraction).
- **`compareTo` serves the four ordering operators** (D7), compared with `0` by the built-in comparison; `==` never calls it. **`in` and `!in` are binary operators calling `contains`** (D8), between `?:` and comparisons as in Kotlin, non-associative, with `!in` one token; built in on a `Range`, the intrinsic member on a `List`.
- **Indexing: a borrowing or value `get`, and a shared and a mutable accessor** (D9). A `get` returning `&E` makes `a[i]` a place behind that borrow, as a list element is (fields read in place, `&a[i]`, receivers), tied to `a` by elision rule 2; a value `get` makes a temporary. Writes need `&mut E`, so a type may declare a `mut` `get` beside the shared one (Rust's `Index`/`IndexMut`), kept as `get#mut` in the tables; a mutable position switches the typed call to it when demanded, with no pass over the body. A `mut` `get` that disagrees with the shared one serves no write (`E4003`, naming it as such). Rejected: Kotlin's value `get` only (no field writes or borrows of non-`Copy` elements), a `&mut` `get` only, a separately named accessor, a backward pass choosing the accessor.
- **`set` serves whole-element writes when declared** (D10), so a type can observe every assignment; `a[i] op= v` through `set` evaluates each index once into a temporary (`index#1`) and needs a variable or field path as the indexed value. Field writes and borrows still use the `mut` `get`.
- **`for` uses a cursor protocol on the collection** (D11). An iterator object borrowing its collection would be a struct holding a borrow, which needs lifetime annotations, against the zero-cost memory invariant; so the collection serves `iterator()`, `hasNext(&cursor)` and `next(&mut cursor)` with a cursor holding no reference, and the loop borrows the collection for its whole length as a list loop does. A `&mut` iterable calls a `mut` `next`. Rejected: Kotlin's iterator objects, an index protocol (a special case), a closure protocol (no `break`), generators.
- **`invoke` serves calls of values of non-function types** (D12), after name resolution, with any receiver mode. **Compound assignment through operator functions is `a = a op b`** (D13), the target evaluated once; a consuming receiver needs a whole binding, so `xs[i] += t` with a consuming `plus` is `E4017` naming the function. Rejected: taking the element out and putting the result back (a "take" operand every MIR consumer would learn; a follow-up in `ROADMAP.md`).
- **Typed tree and MIR: marked calls, accessor places and one loop shape** (D14). Operator uses are `Call` nodes marked with the operator spelled; an ordering use compares the call with `0`; `==` stays an equality node; an accessor place is `Deref` of a temporary holding the call, which borrows reborrow; a cursor loop is `ForIteration::Cursor`, lowered to a header calling `hasNext` and a body calling `next`. The borrow checker and the backend need nothing new. Rejected: an operator node in MIR, desugaring in the parser.
- **Cost** (D3, D18). One table lookup per operator use that is not built in (two for an index place), a fixed check per declaration and a flag per type in Phase 1; `compile_timing`'s `operator-uses` shape keeps every stage linear.
- **Review (archive).** A fresh review fixed `equals` being ignored by files importing the type's module (an importer took the flag only from reachable definitions, not from exported struct and enum symbols), and several diagnostics. User code now runs inside equality glue and the compose runtime, as in Compose: an `equals` reading or writing the state being written fails with the located `state accessed while borrowed`.
