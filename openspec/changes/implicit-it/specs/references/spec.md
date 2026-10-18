# References Specification Delta

## ADDED Requirements

### Requirement: Implicit Closure Parameters Borrow and Escape Like Declared Ones
The borrow checker MUST treat a closure's implicit `it` exactly as the declared parameter of `{ it -> ... }`: a reference `it` follows the elision rules of the closure's function type, and an `it` whose parameter is not `escaping` MUST NOT be kept (`E5007`, naming the parameter `it`).

#### Scenario: A closure returning its implicit reference parameter
- **WHEN** `fun app(g: (&Int) -> &Int): Int = 1` is declared and `fun f(): Int = app { it }` and `fun h(): Int { val n = 1; return app { if (it > 0) it else &n } }` are checked
- **THEN** the borrow checker accepts `f` and emits `E5009` at `&n` in `h`, because the closure's result is tied to its parameter `it` only

#### Scenario: An implicit function-typed parameter escapes only when escaping
- **WHEN** `fun keepIt(f: escaping () -> Unit) {}`, `fun register(r: (escaping () -> Unit) -> Unit) {}` and `fun now(r: (() -> Unit) -> Unit) {}` are declared, and `fun f() { register { keepIt(it) } }` and `fun h() { now { keepIt(it) } }` are checked
- **THEN** `f` passes `prism check`, and the borrow checker emits `E5007` for `h`, saying that the parameter `it` cannot escape
