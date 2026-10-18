# Type System Specification Delta

## ADDED Requirements

### Requirement: Function Types With a Receiver
A function type with a receiver `T.(A) -> R` MUST be a function type whose first parameter is the receiver, of type `&T`, followed by the written parameters; with `mut`, the receiver's parameter MUST be `&mut T`. Whether a function type has a receiver MUST be part of its identity and of how it is rendered (`ColumnScope.(Int) -> Unit`, `mut Builder.() -> Unit`, `compose ColumnScope.() -> Unit`), and its `compose` and `escaping` flags MUST behave as for any function type.

#### Scenario: Receiver function types in diagnostics
- **WHEN** `struct ColumnScope : Copy {}` and `fun Column(content: ColumnScope.(Int) -> Unit) {}` are declared and `fun f(): String = Column` is checked
- **THEN** the type checker emits `E4005`, naming the type `(ColumnScope.(Int) -> Unit) -> Unit`

#### Scenario: Composable receiver function types keep their compose flag
- **WHEN** `struct ColumnScope : Copy {}`, `compose fun Item() {}`, `compose fun Column(content: compose ColumnScope.() -> Unit) { content(ColumnScope()) }`, `compose fun Screen() { Column { Item() } }` and `fun later(f: ColumnScope.() -> Unit) {}` are declared, and `compose fun P(content: compose ColumnScope.() -> Unit) { later(content) }` is checked
- **THEN** `Screen` typechecks, the lambda being checked in a composable context, and the type checker emits `E4004` at `content` in `later(content)`, because the compose flags differ

### Requirement: Receiver and Plain Function Types Convert in Place
A function value of a receiver function type MUST convert to the plain function type with the same parameters (`T.(A) -> R` to `(&T, A) -> R`, `mut T.(A) -> R` to `(&mut T, A) -> R`) and back, in place, under the other rules of "Function Values Convert Only Without Changing Representation": a shared receiver's type MAY become a mutable receiver's, and a mutable receiver's MUST NOT become a shared one's.

#### Scenario: Converting between receiver and plain function types
- **WHEN** `struct Scope : Copy {}`, `fun header(s: &Scope) {}`, `fun column(content: Scope.() -> Unit) { content(Scope()) }` and `fun plain(f: (&Scope) -> Unit) { f(Scope()) }` are declared, and a body runs `val h: (&Scope) -> Unit = header`, `column(h)`, `val c: Scope.() -> Unit = { }`, `plain(c)`, `val m: mut Scope.() -> Unit = c` and `val p: (&mut Scope) -> Unit = m`
- **THEN** the file passes `prism check`

#### Scenario: A mutable receiver does not become a shared one
- **WHEN** `struct Builder { var size: Int }` and `fun read(f: Builder.() -> Unit) {}` are declared and a body runs `val m: mut Builder.() -> Unit = { size += 1 }; read(m)`
- **THEN** the type checker emits `E4004` at `m` in `read(m)`

### Requirement: Lambdas With a Receiver
A closure checked against a function type with a receiver MUST bind `this`, as a `val` of the receiver's parameter type, in its own scope before its parameters. Its header MUST declare the other parameters only, and a closure without a header MUST have the implicit `it` exactly when there is one other parameter. A closure with no expected type, or checked against a function type without a receiver, MUST have no receiver of its own.

#### Scenario: this in a lambda with a receiver
- **WHEN** `struct Scope : Copy { val depth: Int }` and `fun nest(f: Scope.(String) -> Int): Int = f(Scope(1), "ab")` are declared and a body runs `val a = nest { this.depth + it.length }` and `val b = nest { s -> depth + s.length }`
- **THEN** the file typechecks successfully: `this` has type `&Scope`, `it` and `s` have type `String`, and `depth` reads `this.depth`

#### Scenario: Receiver lambdas from every expected type
- **WHEN** `struct Scope : Copy { val depth: Int }`, `struct Section { val content: Scope.() -> Int }` and `fun <T> with(receiver: &T, block: T.() -> Int): Int = block(receiver)` are declared, and a body runs `val f: Scope.() -> Int = { depth }`, `val g: (Scope.() -> Int)? = { depth + 1 }`, `val s = Section { depth * 2 }` and `val n = with(Scope(3)) { depth }`
- **THEN** the file typechecks successfully, each closure having a receiver of type `&Scope`, the last one after `T` is inferred as `Scope` from the first argument

#### Scenario: A mutable receiver may be written through
- **WHEN** `struct Builder { var size: Int }` is declared and a body runs `val f: mut Builder.() -> Unit = { size += 1 }` and `val g: Builder.() -> Unit = { size += 1 }`
- **THEN** the type checker accepts `f` and emits `E4003` in `g`, because its `this` is a shared borrow

#### Scenario: A closure against a plain function type has no receiver
- **WHEN** `struct Scope : Copy { val depth: Int }` and `fun plain(f: (&Scope) -> Int): Int = f(Scope(1))` are declared and `fun f(): Int = plain { depth }` and `fun g(): Int = plain { this.depth }` are checked
- **THEN** the type checker emits `E4001` at `depth` in `f` and at `this` in `g`, saying that `this` is only available in an extension function or a lambda with a receiver

#### Scenario: Parameter counts exclude the receiver
- **WHEN** `struct Scope : Copy {}` and `fun pair(f: Scope.(Int, Int) -> Int): Int = f(Scope(), 1, 2)` are declared and a body runs `pair { a, b -> a + b }` and `pair { it }`
- **THEN** the type checker accepts the first closure and emits `E4004` for the second, saying that it expected 2 parameters of types `Int` and `Int` but declares none

### Requirement: Calling Function Values With a Receiver
A value `f` of a receiver function type MUST be callable as `f(r, a)`, with the receiver as the first argument, as `r.f(a)`, and as `f(a)` when the innermost `this` has the receiver's type, which is then passed as the receiver. A call `f(a)` with one argument fewer than its parameters and no innermost `this` of the receiver's type MUST be rejected with `E4004`, saying that `f` takes a receiver of that type and how to pass one.

#### Scenario: Three ways to call a receiver function value
- **WHEN** `struct Scope : Copy { val depth: Int }` is declared and `fun a(f: Scope.(Int) -> Int): Int = f(Scope(1), 2)`, `fun b(f: Scope.(Int) -> Int): Int { val s = Scope(1); return s.f(2) }` and `fun Scope.c(f: Scope.(Int) -> Int): Int = f(2)` are checked
- **THEN** the file typechecks successfully, each call passing a `Scope` as the receiver of `f`

#### Scenario: A receiver function value called without a receiver
- **WHEN** `struct Scope : Copy {}` and `struct Other : Copy {}` are declared and `fun a(f: Scope.() -> Int): Int = f()` and `fun Other.b(f: Scope.() -> Int): Int = f()` are checked
- **THEN** the type checker emits `E4004` for each call, saying that `f` takes a receiver of type `Scope`, passed as `f(scope)` or `scope.f()`
