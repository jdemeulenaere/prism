# Type System Specification Delta

## ADDED Requirements

### Requirement: Function Values Convert Only Without Changing Representation
A function value MUST convert to another function type only when its parameters and result are equal or convert by `&mut T` to `&T`, also inside a nullable or a function type, with equal compose flags; otherwise its site's mismatch code (`E4002`, `E4004`, `E4005`) MUST be reported, with help suggesting a closure that calls it. A named function or variant constructor MAY convert its own parameters and result through any step, except function-typed ones.

#### Scenario: Converting a function value through a nullable parameter
- **WHEN** `fun show(n: Int?) {}` and `fun each(f: (Int) -> Unit) { f(1) }` are declared, and a body runs `val g: (Int?) -> Unit = show; each(g)`
- **THEN** the type checker emits `E4004` at `g` in `each(g)`, with help suggesting `{ x -> g(x) }`

#### Scenario: Converting a function value to a nullable result
- **WHEN** a body runs `val k = { 2 }; val m: () -> Int? = k`
- **THEN** the type checker emits `E4002` at `k` in the second declaration

#### Scenario: Branches joining function values of different representations
- **WHEN** `fun show(n: Int?) {}` and `fun count(n: Int) {}` are declared, and a body runs `val g: (Int?) -> Unit = show; val h = if (c) g else count` for a `Bool` `c`
- **THEN** the type checker emits `E4002` at `g`, since the branches join to `(Int) -> Unit`

#### Scenario: Named functions and closure literals still convert
- **WHEN** `fun show(n: Int?) {}`, `fun each(f: (Int) -> Unit) { f(1) }` and `fun pick(f: () -> Int?): Int? = f()` are declared, and a body runs `each(show)`, `val h: (Int) -> Unit = show` and `pick { 1 }`
- **THEN** the file passes `prism check`

#### Scenario: A named function's function-typed parameter converts as a value
- **WHEN** `fun each(f: (Int) -> Unit) { f(1) }` and `fun all(g: ((Int?) -> Unit) -> Unit) {}` are declared, and a body runs `all(each)`
- **THEN** the type checker emits `E4004` at `each`, because its parameter of type `(Int) -> Unit` would have to receive a `(Int?) -> Unit`

#### Scenario: A mutable reference parameter accepts a shared one
- **WHEN** `fun read(r: &Int): Int = r` is declared, and a body runs `val f: (&Int) -> Int = read; val g: (&mut Int) -> Int = f`
- **THEN** the file passes `prism check`

#### Scenario: A function value lent to a reference parameter
- **WHEN** `fun call(f: &(() -> Int)): Int = f()` is declared, and a body runs `val g: () -> Int = { 4 }; call(g); call(&g); call { 5 }`
- **THEN** the file passes `prism check`: lending a function value converts nothing
