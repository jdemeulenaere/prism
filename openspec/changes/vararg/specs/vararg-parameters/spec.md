## ADDED Requirements

### Requirement: A Vararg Parameter Is an Owned List
A parameter declared `vararg name: E` MUST have type `List<E>` in the function's body, as an ordinary by-value parameter the callee owns: it MAY be read, iterated, moved, changed through a `var` it is moved into, and returned. An element type holding a reference outside a function type MUST be rejected with `E4022` at the declaration, with help naming the type to take instead.

#### Scenario: The body sees a list
- **WHEN** `fun sum(vararg xs: Int): Int { var total = 0; for (x in xs) total += x; return total + xs.size }` and `fun <T> listOf(vararg elements: T): List<T> = elements` are checked in a file without imports
- **THEN** the file typechecks, `xs` having type `List<Int>` and `elements` type `List<T>`

#### Scenario: Taking elements out of the list
- **WHEN** `fun <T> lastOf(vararg xs: T): T { var list = xs; return list.removeLast() }` is checked
- **THEN** the file passes `prism check`

#### Scenario: A borrowed element type rejected
- **WHEN** `fun show(vararg names: &String) {}` is checked
- **THEN** the type checker emits `E4022`, with help suggesting `vararg names: String`

### Requirement: Positional Arguments Fill the Vararg
A call MUST bind positional arguments to the parameters before the vararg in order, then every further positional argument, in order, as an element of the vararg. Parameters after the vararg MUST be given by name, by the trailing lambda, or by their default. A call with no element MUST pass an empty list, so a vararg parameter is never missing.

#### Scenario: Elements and later parameters
- **WHEN** `fun line(prefix: String, vararg words: String, separator: String = " "): Int = words.size` is declared and `fun f(): Int = line("a", "b", "c") + line("a") + line("a", "b", separator = "-")` is checked
- **THEN** the file typechecks: the calls pass two, zero and one elements, and the third gives `separator` by name

#### Scenario: A parameter after the vararg needs a name
- **WHEN** `fun f(vararg xs: Int, last: Int): Int = last` is declared and `fun g(): Int = f(1, 2)` and `fun h(): Int = f(1, 2, last = 3)` are checked
- **THEN** the type checker emits `E4004` for `g`, saying that `last` is missing, and accepts `h`

### Requirement: A Vararg's Elements Are Written Together
The elements of a vararg MUST be consecutive positional arguments: a named argument written between two of them MUST be rejected with `E4004`. The vararg's list MUST be evaluated as one argument, at the position of its first element, its elements left to right.

#### Scenario: A named argument between elements rejected
- **WHEN** `fun f(vararg xs: Int, last: Int = 0): Int = last` is declared and `fun g(): Int = f(1, last = 2, 3)` is checked
- **THEN** the type checker emits `E4004` at `3`, saying that the elements of `xs` must be written together

### Requirement: Elements Are Typed as Parameters of the Element Type
A call with `n` elements MUST be checked as a call of the declaration with the vararg replaced by `n` by-value parameters of its element type. For a generic element type, the written type arguments, the expected type and the elements' joined types MUST give the type arguments as for any generic call; a conflicting element MUST be `E4004` at it, and an unsolved type parameter `E4013`.

#### Scenario: Inferring the element type
- **WHEN** `fun <T> bag(vararg xs: T): List<T> = xs` is declared and `fun f(a: String, b: String) { val x = bag(1, 2); val y = bag(1, null); val z: List<String> = bag(); val w = bag<Double>(); val v = bag(a, b) }` is checked
- **THEN** the file typechecks: `x` is a `List<Int>`, `y` a `List<Int?>`, `z` a `List<String>`, `w` a `List<Double>` and `v` a `List<String>`, and `a` and `b` are moved into the call

#### Scenario: Conflicting or unsolved element types rejected
- **WHEN** `fun <T> bag(vararg xs: T): List<T> = xs` is declared and `fun f() { val a = bag(1, 2.0) }` and `fun g() { val b = bag() }` are checked
- **THEN** the type checker emits `E4004` at `2.0` and `E4013` for `b`, naming `T`

#### Scenario: Elements of a declared type
- **WHEN** `fun sum(vararg xs: Int): Int = xs.size` is declared and `fun f(r: &Int): Int = sum(1, r, "a")` is checked
- **THEN** the type checker emits `E4004` at `"a"` only: `r` is copied out of its borrow as an `Int` argument

### Requirement: Spread Arguments Pass a Whole List
A spread argument `*e` MUST pass `e`, of exactly the vararg's type `List<E>`, as the whole vararg, moved in without copying. It MUST be the vararg's only argument, and a named vararg argument MUST be a spread. A spread mixed with elements or with another spread, a spread for a parameter that is not a vararg, and a named vararg argument that is not a spread MUST each be rejected with `E4004`.

#### Scenario: Forwarding a list
- **WHEN** `fun sum(vararg xs: Int): Int = xs.size` and `fun <T> bag(vararg xs: T): List<T> = xs` are declared and `fun f(numbers: List<Int>): Int = sum(*numbers)` and `fun <T> again(vararg xs: T): List<T> = bag(xs = *xs)` are checked
- **THEN** the file passes `prism check`, each spread moving its list into the call

#### Scenario: Spread forms rejected
- **WHEN** `fun sum(vararg xs: Int): Int = xs.size` and `fun g(n: Int) {}` are declared and `fun f(a: List<Int>, b: List<Int>, r: &Int) { sum(1, *a); sum(*a, *b); sum(xs = 1); g(*r) }` is checked
- **THEN** the type checker emits `E4004` for each of the four calls, the last saying that a spread passes a list to a vararg parameter and that reads through references are implicit

#### Scenario: Spreading a borrowed list rejected
- **WHEN** `fun sum(vararg xs: Int): Int = xs.size` is declared and `fun f(numbers: &List<Int>): Int = sum(*numbers)` is checked
- **THEN** the type checker emits `E4017`, with help suggesting `numbers.clone()`

### Requirement: Trailing Lambdas Never Bind to a Vararg
A trailing lambda whose callee's last parameter is a vararg MUST be rejected with `E4004`, with help saying to pass it inside the parentheses. A function-typed parameter after the vararg MUST take the trailing lambda as any last parameter does.

#### Scenario: A trailing lambda for a vararg rejected
- **WHEN** `fun <T> bag(vararg xs: T): List<T> = xs` is declared and `fun f() { val actions = bag({ 1 }) { 2 } }` is checked
- **THEN** the type checker emits `E4004` at the trailing lambda, saying that a vararg's elements are passed inside the parentheses

#### Scenario: A trailing lambda after a vararg
- **WHEN** `fun on(vararg keys: Int, action: () -> Unit) { action() }` is declared and `fun f() { on(1, 2) { println("run") } }` is checked
- **THEN** the file typechecks, `keys` taking two elements and `action` the trailing lambda

### Requirement: Vararg Belongs to the Declaration
Whether a parameter is a vararg MUST be a property of the function's declaration, like its parameter names and defaults, not of its type: the function's type MUST have a `List<E>` parameter in its place, and a call of a function value MUST never pack elements.

#### Scenario: A vararg function as a value
- **WHEN** `fun sum(vararg xs: Int): Int = xs.size` is declared and `fun f(): Int { val g = sum; return g(listOf(1, 2)) }` and `fun h(): Int { val g = sum; return g(1, 2) }` are checked
- **THEN** the type checker accepts `f`, where `g` has type `(List<Int>) -> Int`, and emits `E4004` for the call in `h`

### Requirement: Elements Are Stored List Values
A vararg's elements MUST be stored in its list as values given to `listOf` are (collections, "Elements Are Stored Values"): a closure element MUST own its captures, and one that may hold a borrow MUST be rejected with `E5007`.

#### Scenario: Closures as elements
- **WHEN** `fun runAll(vararg actions: () -> Unit) { for (a in actions) a() }` is declared and `fun f(r: &String) { val n = 7; runAll({ println(n) }, { println(1) }) }` and `fun g(r: &String) { runAll({ println(r) }) }` are checked
- **THEN** the borrow checker accepts `f`, whose first closure owns a copy of `n`, and emits `E5007` for `g`

### Requirement: Vararg Calls Cost Linear Compile Time
Typing a call with a vararg MUST cost time linear in its arguments, using the existing generic call inference over the expanded parameters, with no search over candidate types and no analysis beyond the call.

#### Scenario: Many vararg calls check in linear time
- **WHEN** `compile_timing`'s `vararg-calls` shape generates files of 256, 1,024 and 4,096 functions, each calling a generic and a non-generic vararg function with eight elements, a spread and a named argument after the vararg, and runs under `PRISM_ASSERT_TIMING=1`
- **THEN** checking time grows by at most 4.6 times per fourfold growth
