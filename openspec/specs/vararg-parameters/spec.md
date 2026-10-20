# vararg-parameters Specification

## Purpose
Defines Kotlin-style variadic parameters: the `vararg` modifier on a function's parameter, the parameter as an owned `List<T>` the caller builds, how a call's arguments fill it (elements, the parameters after it, empty calls), how its elements are typed with the existing generic inference, spread arguments passing a whole list, trailing lambdas never binding to it, vararg as a property of the declaration rather than of function types, elements as stored list values, which operator functions may take one, and the compile-time cost of a vararg call. The syntax is in `syntax`, `listOf` in `collections`, and how vararg calls are represented and run in `typed-tree`, `mir`, `compilation-and-abi` and `execution`.

## Requirements

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

#### Scenario: Reloading a vararg function
- **WHEN** a program runs under `prism run --watch` with `--check-allocations`, its `main` calls `sum(1, 2, 3)` twice, and between the calls the body of `lib/math.prism`'s `public fun sum(vararg xs: Int): Int` changes from summing its elements to `xs.size * 10`
- **THEN** the reload applies `lib/math.prism` alone, since the machine signature takes the same list, and the program prints `6` then `30` and frees every list once

### Requirement: Elements Are Stored List Values
A vararg's elements MUST be stored in its list as values given to `listOf` are (collections, "Elements Are Stored Values"): a closure element MUST own its captures, and one that may hold a borrow MUST be rejected with `E5007`.

#### Scenario: Closures as elements
- **WHEN** `fun runAll(vararg actions: () -> Unit) { for (a in actions) a() }` is declared and `fun f(r: &String) { val n = 7; runAll({ println(n) }, { println(1) }) }` and `fun g(r: &String) { runAll({ println(r) }) }` are checked
- **THEN** the borrow checker accepts `f`, whose first closure owns a copy of `n`, and emits `E5007` for `g`

### Requirement: Operator Functions Take a Vararg Only for Indices and Calls
As in Kotlin, only `get`, `set` and `invoke` operator functions MAY declare a vararg parameter, and `set` only among its indices, before its value parameter. Any other convention with a vararg, and a `set` whose value parameter is the vararg, MUST be rejected with `E4035`. An indexed assignment through a `set` whose indices end with a vararg MUST pass the written indices as the vararg's elements and the assigned value to the last parameter.

#### Scenario: Vararg operator shapes
- **WHEN** `struct V : Copy { val x: Int }` is declared with, in turn, `operator fun V.plus(vararg o: Int): Int = 0`, `operator fun V.contains(vararg o: Int): Bool = true`, `operator mut fun V.set(i: Int, vararg value: Int) {}`, and with `operator fun V.get(vararg i: Int): Int = 0` and `operator fun V.invoke(vararg o: Int): Int = 0`
- **THEN** the type checker emits `E4035` for each of the first three, saying which conventions may have a vararg or that `set`'s value parameter cannot be one, and accepts `get` and `invoke`

#### Scenario: Indexing with vararg indices
- **WHEN** `struct Grid { var cells: List<Int> }` is declared with `operator fun Grid.get(vararg at: Int): Int` and `operator mut fun Grid.set(vararg at: Int, value: Int)` over `cells[at.size]`, and `fun main() { var g = Grid(listOf(0, 0, 0)); g[1, 1] = 5; g[1, 1] += 2; g[9] = 3; println(g[1, 1]); println(g[4]) }` runs with `--check-allocations`
- **THEN** it prints `7` and `3`, each on its own line, and exits with status 0

### Requirement: Vararg Calls Cost Linear Compile Time
Typing a call with a vararg MUST cost time linear in its arguments, using the existing generic call inference over the expanded parameters, with no search over candidate types and no analysis beyond the call.

#### Scenario: Many vararg calls check in linear time
- **WHEN** `compile_timing`'s `vararg-calls` shape generates files of 256, 1,024 and 4,096 functions, each calling a generic and a non-generic vararg function with eight elements, a spread and a named argument after the vararg, and runs under `PRISM_ASSERT_TIMING=1`
- **THEN** checking time grows by at most 4.6 times per fourfold growth

## Design Rationale
Decisions from `vararg` (part L4 of agreed item A4 of the post–Milestone 2 review, user direction 2026-10-20 for `vararg`; the rest planned autonomously on 2026-10-20, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Kotlin's modifier on `fun` parameters** (D1, D9). `vararg` before the parameter's name, the written type being the element type, on top-level, extension, `compose` and `external` functions; a contextual modifier, so `vararg` stays usable as a name. `E2001` for a second vararg, a default (the empty list is its default), `escaping` or `attached` on it, and `vararg` on struct fields, closure, context or function-type parameters. Rejected: Swift's `xs: Int...`; a collection literal passed to a `List<T>` parameter; varargs in struct constructors (a follow-up if wanted).
- **The parameter is an owned `List<T>` the caller builds** (D2). One allocation of exactly the elements (none for zero), moved into the callee, which may read, change, keep or return it, so `listOf` is `= elements` with no copy; Kotlin's vararg is likewise an array the callee owns. Element types are borrow-free (`E4022`), as every list's. Rejected: a borrowed `&List<T>` (the caller allocates anyway, and keeping or returning the elements becomes impossible); a slice or array type (a new built-in type for an allocation lists already accept; possible later as an optimization); lists of borrows.
- **One vararg, anywhere; elements written together** (D3). Positional arguments fill the parameters before it, then every further one is an element; later parameters take a name, the trailing lambda or their default. A named argument between two elements is `E4004`, so the list is one argument evaluated at its first element and evaluation stays in source order. Rejected: vararg last only; elements split around named arguments.
- **Elements typed as that many parameters, with the existing inference** (D4). A call of `n` elements is checked as the declaration with the vararg replaced by `n` by-value parameters of the element type, exactly as `listOf` was typed before: explicit type arguments, the expected type and joined lower bounds give `T` (`listOf(1, null)` is `List<Int?>`, `listOf(1, 2.0)` is `E4004`, `listOf()` alone `E4013`). Linear in the arguments; no new inference. Rejected: the first element fixing `T`; numeric widening or a common-supertype search.
- **Spread `*xs` passes a whole list, moved in** (D5). The vararg's only argument, of exactly its `List<E>`; a borrowed list is `E4017` with `clone()` help; a named vararg argument must be a spread, as in Kotlin. `*` could not start an argument before (`E2001`), so the syntax is unambiguous. Mixing spreads with elements and spreading a borrowed list without moving it are deferred.
- **A trailing lambda never binds to a vararg** (D6), Kotlin's rule, so `listOf({ 1 }) { 2 }`, which `collections` D22 accepted as a two-element list, is `E4004`; a function-typed parameter after the vararg takes the trailing lambda.
- **Vararg belongs to the declaration** (D7), like parameter names and defaults: a function value of `fun sum(vararg xs: Int): Int` has type `(List<Int>) -> Int` and its calls never pack; no `vararg` in function types.
- **Elements are stored list values** (D8): the list construction's declared parameters are its element type parameter, so the capture analysis and the borrow checker treat elements as `listOf`'s arguments always were (closures own their captures, `E5007` for one that may hold a borrow).
- **Operators take a vararg where Kotlin's do** (implementation notes, revised in review): `get`, `invoke`, and `set` among its indices; any other convention, or `set`'s value parameter, is `E4035`. An indexed assignment through a `set` with vararg indices passes the value to the last parameter by name. Rejected: any operator taking a vararg like any `fun` (`a + b` packed `b`, and `g[1, 2] = v` bound the value as an index); no vararg on operators at all.
- **`listOf` is ordinary Prism** (D10, D13): `public fun <T> listOf(vararg elements: T): List<T> = elements` in `/prism/collections`; the compiler keeps one built-in, the construction of a vararg's list (`builtin varargList<E>` in dumps, the former `ListOf` lowering, MIR tag 8), which a Rust-backed `List` needs. The typed tree gives the vararg one argument, that construction or the spread list; MIR builds it into a temporary moved into the call. Each element type instantiates `listOf` once, a one-line instance cached as any other. Rejected: keeping `listOf` an intrinsic; inlining it by name in the backend; packing in MIR; desugaring to `listOf` in the parser.
- **`print`, `println`, `remember` and `key` stay rule-typed** (D11): they take arguments of different types (Kotlin's `Any?`), which a homogeneous vararg cannot type.
- **No new error code** (D12): `E2001`, `E4004`, `E4013`, `E4017`, `E4022` and `E4035` cover every vararg mistake.
- **The ABI records the flag** (D14): `AbiParameter::is_vararg`, with the parameter's type its `List<E>`, covered by the ABI hash (the parameter-kind byte's value 3, so existing hashes keep their bytes); the machine signature is the same, so a function gaining or losing `vararg` keeps its slot contract under hot reload, and its importers are re-checked. `prism abi` prints `vararg xs: Int`.
- **Compile-time cost** (D16). One token test per parameter and argument, one expanded signature per vararg call, linear in its arguments, and one small instance of `listOf` per element type per program. `compile_timing`'s `vararg-calls` shape measured type 3.3 / 14.2 / 59.4 ms and clif 22.5 / 92.1 / 369.8 ms at 256 / 1,024 / 4,096 functions.
