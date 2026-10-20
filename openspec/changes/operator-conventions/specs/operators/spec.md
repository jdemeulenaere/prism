## ADDED Requirements

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
An operator use MUST find its function as a call of that name on the receiver finds an extension: members first, then the file's own extensions, then its imports' (`extension-functions`), so an operator function of another module is used only where that module is imported. Several imported matches MUST be `E4012`. None MUST be `E4002`, with help naming the function to declare (`operator fun Vector2.plus(other: Vector2)`).

#### Scenario: An imported operator function
- **WHEN** `geometry/vector.prism` exports `public struct Vector2 : Copy { val x: Int, val y: Int }` and `public operator fun Vector2.plus(other: Vector2): Vector2`, and `app/main.prism` imports `/geometry/vector` and declares `fun f(a: Vector2, b: Vector2): Vector2 = a + b`
- **THEN** `prism check app/main.prism` succeeds, while the same body in a file that receives `Vector2` values from another module without importing `/geometry/vector` reports `E4002` with help naming `operator fun Vector2.plus`

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
`equals` MUST be declared as `operator fun T.equals(other: &T): Bool`, in the file declaring `T`, a non-generic struct or enum, with `T`'s visibility, at most once per type; anything else is `E4035`. It MUST be part of `T`'s definition: `==` on `T` uses it in every file, whatever that file imports, and a built-in type cannot have one.

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
- **THEN** the file typechecks, `b[0].name` is borrowed where it lives and `first` is the borrow `get` returned

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
The loop variable MUST be a fresh immutable binding of `next`'s result type in each iteration: a `&E` borrows an element, a value is owned and dropped at the end of the iteration. `for (x in &mut y)` MUST call a `mut` `next` returning `&mut E`, a second `next` of the receiver; with none it MUST be `E4003`, naming it. The cursor MUST be dropped when the loop ends.

#### Scenario: Iterating mutably through a mutable next
- **WHEN** the `Bag` protocol above also declares `operator mut fun Bag.next(cursor: &mut Int): &mut Item { val i: Int = cursor; *cursor = i + 1; return &mut items[i] }` with `struct Item { var name: String }`, and `fun f(b: &mut Bag) { for (item in &mut b) { item.name = "x" } }` and `fun g(b: &mut Bag) { for (item in b) { item.name = "x" } }` are checked
- **THEN** the type checker accepts `f`, where `item` has type `&mut Item`, and emits `E4003` for `g`, whose `item` is a shared borrow, with help suggesting `for (item in &mut b)`

#### Scenario: Values produced by next
- **WHEN** `struct Countdown : Copy { val from: Int }`, `operator fun Countdown.iterator(): Int = from`, `operator fun Countdown.hasNext(cursor: &Int): Bool = cursor > 0` and `operator fun Countdown.next(cursor: &mut Int): String { val n: Int = cursor; *cursor = n - 1; return n.toString() }` are declared and `fun f() { for (s in Countdown(3)) println(s) }` is checked
- **THEN** the file typechecks, `s` has type `String`, and each `s` is dropped at the end of its iteration

### Requirement: Compound Assignment Through Operator Functions
`a op= b`, where the built-in operator does not apply to `a`'s and `b`'s types, MUST be `a = a op b` with the target place evaluated once: the operator function receives the target as its receiver (borrowed, or moved for a consuming receiver), and its result, of the target's type (`E4002` otherwise), is assigned to the target. The target MUST be writable (`E4003`).

#### Scenario: Accumulating vectors
- **WHEN** `struct Vector2 : Copy { val x: Int, val y: Int }` and `operator fun Vector2.plus(other: Vector2): Vector2 = Vector2(x + other.x, y + other.y)` are declared and `fun f(vs: &List<Vector2>): Vector2 { var total = Vector2(0, 0); for (v in vs) { total += v }; return total }` and `fun g(a: Vector2) { a += a }` are checked
- **THEN** the type checker accepts `f` and emits `E4003` for `g`

#### Scenario: A consuming operator in a compound assignment
- **WHEN** `struct Text { val s: String }` and `operator consuming fun Text.plus(other: &Text): Text = Text(s + other.s)` are declared and `fun f(t: Text): Text { var a = Text("x"); a += t; return a }` is checked
- **THEN** the file passes `prism check`: `a` is moved into `plus` and the result assigned back to it

### Requirement: Operator Uses Cost a Lookup Each
Typing an operator use MUST cost at most a constant number of extension table lookups (one per function a convention calls, two for an index place), with no search over overloads or candidate types, so checking a file stays linear in its operator uses.

#### Scenario: Many operator uses check in linear time
- **WHEN** `compile_timing`'s `operator-uses` shape generates files of 256, 1,024 and 4,096 functions, each using `+`, `<`, `in`, `==` on a type with `equals`, an index place and a cursor loop on user types, and runs under `PRISM_ASSERT_TIMING=1`
- **THEN** checking time grows by at most 4.6 times per fourfold growth
