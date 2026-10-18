# Collections Specification Delta

## Purpose

Defines Prism's built-in collection, the growable single-owner `List<T>`: its type and markers, how a list is built, its built-in members and their precedence over extensions, its elements as places, how `for` iterates it, how its elements are compared, and how the borrow rules apply to elements and to the lists that loops iterate. Its runtime meaning is in `execution`, its representation in `code-generation`.

## ADDED Requirements

### Requirement: The List Type
`List<T>` MUST be a built-in generic type with exactly one type argument (`E4020` otherwise), which MUST be borrow-free (`E4022`). Two list types MUST be assignable only when their element types are the same type: `List<T>` is invariant. A struct or enum that reaches itself through a `List` MUST NOT be `E4023`: a list is an indirection, like `Heap<T>`. A `List<T>` in an exported signature MUST be recorded in the ABI as the built-in list type with its element type.

#### Scenario: Lists in signatures and fields
- **WHEN** `struct Todo { val title: String, var done: Bool }`, `struct Model { var todos: List<Todo> }` and `public fun count(m: &Model): Int = m.todos.size` are checked
- **THEN** the file typechecks successfully, and the ABI records the parameter type `&Model` and the field type `List<Todo>`

#### Scenario: Lists are invariant
- **WHEN** `fun take(xs: List<Int?>) {}` is declared and `fun f(xs: List<Int>) { take(xs) }` is checked
- **THEN** the type checker emits `E4004`, with help saying that list types are invariant and that `listOf<Int?>(...)` builds a list of the wider type

#### Scenario: A recursive type through a list
- **WHEN** `struct Node { val name: String, val children: List<Node> }` is checked
- **THEN** the file typechecks successfully, without `E4023`

#### Scenario: Borrows and wrong arity in the element type rejected
- **WHEN** `fun f(a: List<&Int>) {}` and `fun g(b: List<Int, Int>) {}` are checked
- **THEN** the type checker emits `E4022` for `f` and `E4020` for `g`

### Requirement: The Name List Is Built In
The name `List` MUST be built in, like `Heap`: a top-level `struct`, `enum`, `fun` or `val`, or a type parameter, named `List` MUST be rejected with `E4010`, and `List` used as a value or called MUST be rejected with `E4001`, with help naming `listOf`.

#### Scenario: The name List is built in
- **WHEN** a file declares `struct List { val n: Int }`, or declares `fun f() { val l = List }`
- **THEN** the type checker emits `E4010` for the struct, and `E4001` for `List` in `f`, with help suggesting `listOf()`

### Requirement: Lists Are Built With listOf
A call `listOf(e1, ..., en)` with zero or more positional arguments MUST be typed as a call of a generic function `fun <T> listOf(e1: T, ..., en: T): List<T>` with exactly `n` parameters: an explicit type argument (`listOf<Int?>(1)`), the expected type and the arguments, joined, give `T` as for any generic call, each argument is consumed as a `T`, a conflicting argument is `E4004`, and an unsolved `T` (`val xs = listOf()`) is `E4013`. Named arguments are `E4004`.

#### Scenario: Element types from arguments, joins and expected types
- **WHEN** `fun f(): List<Int?> { val a = listOf(1, 2, 3); val b = listOf(1, null); val c: List<String> = listOf(); val d = listOf<Double>(); return b }` is checked
- **THEN** the file typechecks successfully: `a` is a `List<Int>`, `b` a `List<Int?>`, `c` a `List<String>` and `d` a `List<Double>`

#### Scenario: Arguments are moved into the list
- **WHEN** `fun f(s: String): List<String> { val xs = listOf(s, "b"); println(s); return xs }` is checked
- **THEN** the borrow checker emits `E5001` at `println(s)`, because `listOf` consumed `s`

#### Scenario: Conflicting or missing element types rejected
- **WHEN** `fun f() { val a = listOf(1, "a") }` and `fun g() { val b = listOf() }` are checked
- **THEN** the type checker emits `E4004` at `"a"` and `E4013` for `b`, naming the type parameter `T` and suggesting `listOf<...>()` or a type annotation

### Requirement: listOf Is Found Last
`listOf` MUST be called only (`E4001` when named without a call), and a local, parameter, top-level declaration or import named `listOf` MUST shadow it, as declarations shadow the output built-ins.

#### Scenario: A declaration shadows listOf
- **WHEN** `fun listOf(n: Int): Int = n` is declared and `fun f(): Int = listOf(3)` is checked
- **THEN** the file typechecks successfully, and the call is a call of `main::listOf`

### Requirement: List Markers
`List<T>` MUST never be `Copy`, and MUST be `Clone` exactly when `T` is `Clone`, as a generic struct declared `: Clone` with one field of type `T` is. The built-in `clone()` of a `List<T>` MUST give a list of clones of its elements, and calling it when `T` is not `Clone` MUST be rejected with `E4018`. A `: Copy` struct or enum with a list member MUST be rejected with `E4016`, and a `: Clone` one whose list's element type is not `Clone` likewise.

#### Scenario: Cloning lists
- **WHEN** `struct Todo : Clone { val title: String }` and `struct Handle { val id: Int }` are declared and `fun f(a: List<Todo>, b: List<Handle>) { val c = a.clone(); val d = b.clone() }` is checked
- **THEN** the type checker accepts `c` and emits `E4018` for `b.clone()`, naming `Handle`

#### Scenario: Markers of types holding lists
- **WHEN** `struct A : Copy { val xs: List<Int> }` and `struct B : Clone { val xs: List<String> }` are checked
- **THEN** the type checker emits `E4016` for `A` and accepts `B`

### Requirement: List Members Are Typed as Extensions
A `List<T>`, read through any `&`, `&mut` and `Heap<T>` layers, MUST have built-in member functions, each typed as an extension on `List<T>` with a receiver mode and a signature, so that receivers, arguments (`E4004`) and safe calls (`xs?.add(1)`) are checked as for an extension. The reading members are `fun isEmpty(): Bool`, `fun isNotEmpty(): Bool`, `fun contains(element: &T): Bool` and `fun indexOf(element: &T): Int`.

#### Scenario: Wrong element types rejected
- **WHEN** `fun f() { var xs = listOf(1); xs.add("a"); xs.insert("b", 1) }` is checked
- **THEN** the type checker emits `E4004` for `"a"` and for the index `"b"`, while the element `1` fits

#### Scenario: Searching borrows the element
- **WHEN** `fun f(xs: &List<String>, name: String): Bool = xs.contains(name) && xs.indexOf(name) >= 0 && !xs.isEmpty()` is checked
- **THEN** the file typechecks successfully, and `name` is borrowed, not moved

### Requirement: Mutating Members
The mutating members MUST be `mut fun add(element: T)`, `mut fun insert(index: Int, element: T)`, `mut fun removeAt(index: Int): T`, `mut fun removeLast(): T`, `mut fun reverse()` and `mut fun clear()`, each returning `Unit` unless a type is given. `removeAt` and `removeLast` MUST give the removed element by move.

#### Scenario: Reading and changing a list
- **WHEN** `fun f(): Int { var xs = listOf(1, 2); xs.add(3); xs.insert(0, 0); val last = xs.removeLast(); val first = xs.removeAt(0); xs.reverse(); if (xs.isEmpty() || !xs.contains(2)) return -1; xs.clear(); return last + first + xs.indexOf(5) }` is checked
- **THEN** the file typechecks successfully, `last` and `first` have type `Int`, and each mutating call takes a mutable borrow of `xs`

#### Scenario: Mutating a list that is not a mutable place rejected
- **WHEN** `fun f(xs: &List<Int>) { xs.add(1) }` and `fun g() { val ys = listOf(1); ys.clear() }` are checked
- **THEN** the type checker emits `E4003` for each call, with help saying that a list is changed through a `var` or a `&mut`

#### Scenario: Removing gives the element by move
- **WHEN** `fun f(): String { var names = listOf("a", "b"); val first = names.removeAt(0); return first }` is checked
- **THEN** the file typechecks successfully, and `first` owns the removed `String`

### Requirement: Size and Indices Are Read Without Parentheses
A `List<T>`, read through any `&`, `&mut` and `Heap<T>` layers, MUST have the built-in members `size` of type `Int` and `indices` of type `Range` (from 0, end excluded, as `0..<size`), read without parentheses with a shared borrow of the list, as `length` of a `String` is; `xs?.size` on a nullable list MUST have type `Int?`. Calling either (`xs.size()`) MUST be rejected with `E4004`, as calling an `Int` or a `Range`.

#### Scenario: Reading the size
- **WHEN** `fun f(a: List<Int>, b: &List<String>, c: List<Int>?): Int { var n = a.size + b.size + (c?.size ?: 0); for (i in a.indices) n += a[i]; return n }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Calling the size rejected
- **WHEN** `fun f(xs: List<Int>): Int = xs.size()` is checked
- **THEN** the type checker emits `E4004`, saying that `Int` cannot be called

### Requirement: List Members Take Precedence Over Extensions
A call `xs.name(...)` or read `xs.name` on a `List<T>` MUST use the built-in member of that name when there is one, after the built-in `clone()` and before any extension. An extension on `List` named like a built-in member (`size`, `indices`, `isEmpty`, `isNotEmpty`, `contains`, `indexOf`, `add`, `insert`, `removeAt`, `removeLast`, `reverse`, `clear`, or `clone` when its element type is `Clone`) could never be called, so it MUST be rejected with `E4010`.

#### Scenario: Extensions named like built-in members rejected
- **WHEN** a file declares `fun <T> List<T>.add(element: T) {}` and `fun List<Int>.size(): Int = 0`
- **THEN** the type checker emits `E4010` for each, saying that the built-in member always takes precedence

### Requirement: Extensions on Lists
Extensions with other names MUST be declarable on `List<T>`, generic (`fun <T> List<T>.second(): &T`) or for one element type (`fun List<Int>.sum(): Int`), keyed by the `List` type constructor like the extensions of a generic struct, and called through any `&`, `&mut` and `Heap<T>` layers.

#### Scenario: An extension on lists
- **WHEN** `fun List<Int>.sum(): Int { var total = 0; for (n in this) total += n; return total }` and `fun <T> List<T>.second(): &T = &this[1]` are declared and `fun f(xs: &List<Int>): Int = xs.sum() + xs.second()` is checked
- **THEN** the file typechecks successfully

### Requirement: Elements Are Places
`xs[i]` on a `List<T>`, read through any `&`, `&mut` and `Heap<T>` layers, MUST take an `Int` index (`E4002` otherwise) and name element `i` of the list, a place of type `T`. A `Copy` element MUST be copied out where a value is read, and a borrow of `xs[i]`, written or implicit at an argument or receiver, MUST borrow the element where it lives. Indexing any type other than `List` and `String` MUST be rejected with `E4002`.

#### Scenario: Reading, borrowing and writing elements
- **WHEN** `struct Todo { val title: String, var done: Bool }` and `fun show(t: &Todo) {}` are declared and `fun f(n: Int) { var xs = listOf(1, 2); xs[0] = xs[1] + n; xs[1] *= 2; var todos = listOf(Todo("a", false)); todos[0].done = true; show(todos[0]); val t: &Todo = &todos[0]; val title = todos[0].title.clone() }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Nested lists
- **WHEN** `fun f(): Int { var grid = listOf(listOf(1, 2), listOf(3)); grid[1].add(4); grid[0][1] = 5; return grid[1][1] + grid.size }` is checked
- **THEN** the file typechecks successfully

#### Scenario: A non-integer index rejected
- **WHEN** `fun f(xs: List<Int>): Int = xs["a"]` is checked
- **THEN** the type checker emits `E4002`, expecting `Int`

### Requirement: Elements Are Not Moved Out
Taking a non-`Copy` element in a consuming position MUST be rejected with `E4017`, naming the element of the list, with help suggesting `.clone()` for a `Clone` type and `removeAt(i)` to take it out.

#### Scenario: Moving an element out rejected
- **WHEN** `fun f(names: List<String>): String { val first = names[0]; return first }` is checked
- **THEN** the type checker emits `E4017` naming an element of `names`, with help suggesting `names[0].clone()` or `removeAt(0)`

### Requirement: Writing Elements Needs a Mutable List
Assigning `xs[i] = v`, compound-assigning `xs[i] += v`, writing a field `xs[i].done = true`, borrowing `&mut xs[i]` and calling a mutating member `xs[i].add(v)` MUST need a mutable place, as writing a field does: the list must be a `var` binding, a `var` field reached by a writable path, or reached through `&mut` or a writable `Heap<T>` (`E4003` otherwise). An assignment MUST drop the old element and move the value in.

#### Scenario: Writing an element of a list that is not mutable rejected
- **WHEN** `fun f(xs: &List<Int>) { xs[0] = 1 }` and `fun g() { val ys = listOf(1); ys[0] += 1 }` are checked
- **THEN** the type checker emits `E4003` for each

### Requirement: For Loops Over Lists
`for (x in iterable)` MUST accept an iterable of type `List<T>`, `&List<T>` or `&mut List<T>`, behind any further `&`, `&mut` and `Heap<T>` layers, as well as a `Range`. The list MUST be evaluated once and borrowed for the whole loop, mutably when the iterable has a `&mut` type (`&mut xs`) and shared otherwise. A place MUST be borrowed where it lives, and any other iterable evaluated into a temporary that lives until the loop ends. Other iterable types stay `E4002` (`loops`).

#### Scenario: Iterating a list of scalars
- **WHEN** `fun f(xs: List<Int>): Int { var total = 0; for (n in xs) { total += n }; return total }` is checked
- **THEN** the file typechecks successfully, `n` has type `&Int`, and `xs` is borrowed, not moved, by the loop

#### Scenario: Iterating a temporary list
- **WHEN** `fun make(): List<String> = listOf("a")` is declared and `fun f() { for (s in make()) println(s) }` is checked
- **THEN** the file typechecks successfully, and the temporary list lives until the loop ends

### Requirement: Loop Variables Borrow Elements
In a `for` loop over a list, the loop variable MUST be a fresh immutable binding of type `&T` in each iteration, or `&mut T` when the list is borrowed mutably, whatever `T` is, in a scope enclosing the body.

#### Scenario: Iterating elements that own storage
- **WHEN** `struct Todo { val title: String, var done: Bool }` is declared and `fun f(todos: &List<Todo>) { for (t in todos) { if (!t.done) println(t.title) } }` is checked
- **THEN** the file typechecks successfully, and `t` has type `&Todo`

#### Scenario: Iterating mutably
- **WHEN** `fun f() { var xs = listOf(1, 2); for (x in &mut xs) { *x += 1 }; var todos = listOf(Todo("a", false)); for (t in &mut todos) t.done = true }` is checked with that `Todo`
- **THEN** the file typechecks successfully: `x` has type `&mut Int` and `t` has type `&mut Todo`

#### Scenario: Writing through a shared loop variable rejected
- **WHEN** `fun f() { var xs = listOf(1); for (x in xs) { *x = 2 } }` is checked
- **THEN** the type checker emits `E4003`, because `x` is a shared borrow, with help suggesting `for (x in &mut xs)`

### Requirement: Lists Are Borrowed for Their Loop
The borrow of a `for` loop's list MUST be live from the loop's start until it ends, so that using the list in the body in a way that conflicts with that borrow MUST be rejected: a mutating call, an assignment to the list or an element, a move of the list, or a mutable borrow of it while it is borrowed shared, and any use of it while it is borrowed mutably (`E5003` to `E5005`). The diagnostic MUST label the loop's borrow and the next iteration as its later use.

#### Scenario: Adding to a list while iterating it rejected
- **WHEN** `fun f() { var xs = listOf(1, 2); for (x in xs) { if (x == 1) xs.add(3) } }` is checked
- **THEN** the borrow checker emits `E5003` at `xs.add(3)`, labelling the borrow of `xs` by the loop

#### Scenario: Reading a list iterated mutably rejected
- **WHEN** `fun f() { var xs = listOf(1); for (x in &mut xs) { println(xs.size) } }` is checked
- **THEN** the borrow checker emits `E5003` at `xs.size`

#### Scenario: Changing a list by index during an indexed loop
- **WHEN** `fun f() { var xs = listOf(1, 2); for (i in xs.indices) { xs[i] *= 2; xs.add(i) } }` is checked
- **THEN** the file is accepted: the loop iterates a `Range`, so it holds no borrow of `xs`

### Requirement: Element Borrows Borrow the List
A borrow of an element MUST be a borrow of a part of its list, which owns its elements, as a box owns its contents: while it is live, a use of the list that conflicts with it (a mutating member call, which borrows the whole list mutably, an assignment, a move, a drop) MUST be rejected (`E5003` to `E5006`). A borrow of an element returned from a function MUST follow the elision rules, as a borrow of a field does.

#### Scenario: Holding an element across an add rejected
- **WHEN** `fun f() { var xs = listOf(1, 2); val first = &xs[0]; xs.add(3); println(first) }` is checked
- **THEN** the borrow checker emits `E5003` at `xs.add(3)`, labelling the borrow `&xs[0]` and its later use in `println(first)`

#### Scenario: Replacing, moving or dropping a list while an element is borrowed rejected
- **WHEN** `fun f() { var xs = listOf("a"); val first = &xs[0]; xs = listOf(); println(first) }`, `fun g() { val xs = listOf("a"); val first = &xs[0]; val ys = xs; println(first) }` or `fun h(other: &String) { var first = other; if (other.length > 0) { val xs = listOf("a"); first = &xs[0] }; println(first) }` is checked
- **THEN** the borrow checker emits `E5004`, `E5005` or `E5006` respectively, because the list owns the borrowed element

#### Scenario: Returning an element borrow
- **WHEN** `fun first(xs: &List<String>): &String = &xs[0]` and `fun bad(): &Int { val xs = listOf(1); return &xs[0] }` are checked
- **THEN** the borrow checker accepts `first` and emits `E5008` for `bad`

### Requirement: Elements of One List Overlap
Two element places of one list MUST be treated as overlapping whatever their indices, as the borrow checker does not compare indices. A mutating member call's receiver MUST be borrowed after its arguments are evaluated, as every mutable receiver is, so its arguments may read the list.

#### Scenario: Two elements borrowed mutably rejected
- **WHEN** `fun f() { var xs = listOf(1, 2); val a = &mut xs[0]; val b = &mut xs[1]; *a = b }` is checked
- **THEN** the borrow checker emits `E5003` at `&mut xs[1]`

#### Scenario: Arguments read the list before a mutating call
- **WHEN** `fun f() { var xs = listOf(1); xs.add(xs.size); xs.add(xs[0]); xs[xs.size - 1] = 0 }` is checked
- **THEN** the file is accepted

### Requirement: Lists Compare Their Elements
`==` and `!=` MUST be accepted on two lists of the same type exactly when they are accepted on two values of its element type, and `contains` and `indexOf` MUST be accepted on a list exactly when `==` is on its element type. In a generic body, comparing lists, or calling `contains` or `indexOf`, whose element type mentions a type parameter MUST be rejected with `E4002`, as comparing two values of a type parameter is.

#### Scenario: Comparing lists and searching them
- **WHEN** `fun f(a: List<String>, b: &List<String>, name: String): Bool = a == b && a.contains(name) && a.indexOf("x") < 0` is checked
- **THEN** the file typechecks successfully, and `name` is borrowed, not moved, by `contains`

#### Scenario: Comparing lists of a type parameter rejected
- **WHEN** `fun <T> same(a: &List<T>, b: &List<T>): Bool = a == b` and `fun <T> has(a: &List<T>, x: &T): Bool = a.contains(x)` are checked
- **THEN** the type checker emits `E4002` for each

### Requirement: Lists Are Matched as Values
A `when` over a list, read through any `&`, `&mut` and `Heap<T>` layers, MUST accept value patterns, compared with `==` as "Lists Compare Their Elements" says, bindings, wildcards and `else`, as for a `String`: a list has no constructor patterns, so only a binding, a wildcard or `else` covers every list. A destructuring declaration of a list MUST be rejected with `E4024`, as for any type other than a struct.

#### Scenario: Matching a list
- **WHEN** `fun f(xs: List<Int>): Int = when (xs) { listOf(1) -> 1; val other -> other.size }` and `fun g(xs: &List<Int>): Int = when (xs) { listOf() -> 0; else -> 1 }` are checked
- **THEN** the file typechecks successfully

#### Scenario: Destructuring a list rejected
- **WHEN** `fun f(xs: List<Int>) { val (a) = xs }` is checked
- **THEN** the type checker emits `E4024`

### Requirement: Elements Are Stored Values
A value given to `listOf`, `add` or `insert`, or assigned to an element, MUST be stored in the list as a value given to a parameter of a type parameter is, with the same rules for function values: a closure that may hold borrows MUST be rejected with `E5007`, and one that owns its captures (`escaping-closures`) MUST be stored. An element that is a function value MUST be callable in place (`handlers[0]()`).

#### Scenario: A list of handlers
- **WHEN** `fun f() { var handlers = listOf<() -> Unit>(); handlers.add { println("hi") }; handlers[0]() }` is checked
- **THEN** the file typechecks and borrow-checks successfully

#### Scenario: Storing a borrowing closure rejected
- **WHEN** `fun f(xs: &mut List<() -> Unit>, r: &Int) { xs.add { println(r) } }` is checked
- **THEN** the borrow checker emits `E5007`, because the closure holds the borrow `r`
