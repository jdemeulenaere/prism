# Type System Specification Delta

## ADDED Requirements

### Requirement: Implicit Closure Parameter
A closure without a parameter header, checked against a function type with exactly one parameter, MUST have one parameter named `it`, bound as a `val` in the closure's scope with that parameter's type and `escaping` flag, as if written `{ it -> ... }`. Whether a closure has `it` MUST depend only on its header and its expected type, never on its body.

#### Scenario: it takes the expected parameter type
- **WHEN** `fun apply(f: (Int) -> Int): Int = f(1)` is declared and a body runs `val a = apply { it + 1 }` and `val g: (String) -> Int = { it.length }`
- **THEN** the file typechecks successfully, `it` having type `Int` in the first closure and `String` in the second

#### Scenario: it in a generic higher-order call
- **WHEN** `fun <T, R> map(x: T, f: (T) -> R): R = f(x)` is declared and a body runs `val n: Int = map(2) { it * 3 }` and `val s: String = map(2) { it.toString() }`
- **THEN** the file typechecks successfully: `T` is inferred as `Int` from the first argument before the closure is checked, and `R` from the closure's body

#### Scenario: it is a reference for a borrowed parameter
- **WHEN** `fun each(xs: &List<String>, f: (&String) -> Unit) { for (x in xs) f(x) }` and `fun count(s: &String): Int = s.length` are declared, and a body runs `var total = 0; each(listOf("a", "bc")) { total += it.length; total += count(it) }`
- **THEN** the file typechecks successfully, `it` having type `&String`, read through it by `.length` and passed on by `count(it)`

#### Scenario: it where a nullable function type is expected
- **WHEN** `fun apply(f: ((Int) -> Int)?, x: Int): Int { val fallback: (Int) -> Int = { -it }; val g = f ?: fallback; return g(x) }` is declared and `main` runs `val f: ((Int) -> Int)? = { it + 1 }`, `println(apply(f, 1))`, `println(apply({ x -> x * 2 }, 2))` and `println(apply({ it * 10 }, 4))`
- **THEN** the file typechecks successfully, each closure being checked against `(Int) -> Int` and then made nullable, and it prints `2`, `4` and `40`

#### Scenario: Unused it is accepted
- **WHEN** `fun each(f: (Int) -> Unit) { f(1) }` is declared and a body runs `each { println("x") }`
- **THEN** the file typechecks successfully

### Requirement: Closures Without an Implicit Parameter
A closure with a header, including the empty header `{ -> }`, a closure whose expected function type has no parameters, and a closure with no expected function type MUST have no implicit `it`. A use of `it` that no enclosing scope binds MUST be rejected with `E4001`, with help saying why the innermost closure has no `it`, or, outside any closure, which closures have one.

#### Scenario: it in a closure expecting no parameters rejected
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun f(): Int = run { it }` is checked
- **THEN** the type checker emits `E4001` at `it`, with help saying that the closure's type `() -> Int` has no parameters, so it has no `it`

#### Scenario: it in a closure without an expected type rejected
- **WHEN** a body runs `val f = { it + 1 }`
- **THEN** the type checker emits `E4001` at `it`, with help saying that only a closure passed where a function of one parameter is expected has an implicit `it`

#### Scenario: it in a closure with a header rejected
- **WHEN** `fun apply(f: (Int) -> Int): Int = f(1)` is declared and `fun f(): Int = apply { x -> it + x }` and `fun g(): Int = apply { -> 1 }` are checked
- **THEN** the type checker emits `E4001` at `it` in `f`, with help saying that the closure declares its parameter `x`, and `E4004` for the closure of `g`, which declares zero parameters where one is expected

#### Scenario: it outside a closure rejected
- **WHEN** `fun f(): Int = it` is checked
- **THEN** the type checker emits `E4001` at `it`, with help saying that `it` names the parameter of a closure that expects one

### Requirement: Implicit Parameters Shadow Outer Names
The implicit `it` MUST shadow every outer binding of `it`: a variable, a parameter, and the `it` of an enclosing closure. A closure without an implicit `it` MUST see the `it` of the enclosing scope, as any other name.

#### Scenario: Nested closures each have their own it
- **WHEN** `fun apply(f: (Int) -> Int): Int = f(1)` and `fun show(f: (String) -> Int): Int = f("ab")` are declared and `fun f(): Int = apply { it + show { it.length } }` is checked
- **THEN** the file typechecks successfully: the outer `it` is an `Int`, the inner one a `String`

#### Scenario: A closure without it sees the enclosing it
- **WHEN** `fun apply(f: (Int) -> Int): Int = f(1)` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(): Int = apply { run { it + 1 } }` and `fun g(it: String): Int = apply { it + 1 }` are checked
- **THEN** both typecheck successfully: in `f` the inner `it` is the outer closure's `Int`, and in `g` the closure's `Int` `it` shadows the `String` parameter

### Requirement: Header-Less Closures Expecting Several Parameters
A closure without a header checked against a function type with two or more parameters MUST be rejected with `E4004`, naming the expected parameter types, with help writing a header that declares them and saying that only a closure expecting one parameter has an implicit `it`.

#### Scenario: Header-less closure for two parameters rejected
- **WHEN** `fun pairs(f: (Int, String) -> Unit) { f(1, "a") }` is declared and a body runs `pairs { println(it) }`
- **THEN** the type checker emits `E4004` at the closure, saying that it expected 2 parameters of types `Int` and `String` but declares none, with help suggesting `{ a, b -> ... }`
