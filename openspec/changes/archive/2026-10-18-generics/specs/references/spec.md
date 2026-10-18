# References Specification Delta

## MODIFIED Requirements

### Requirement: Capturing Closures Do Not Escape
A function value that may be a closure with captures, or a function-typed parameter, MUST NOT escape: being returned from a function, closure or default value, initializing a top-level `val`, being stored in a struct field or enum payload, being written through a reference, being assigned to a captured variable, or being passed where the callee's declared parameter type has a type parameter outside a function type MUST be rejected by the borrow checker with `EscapingClosure` (`E5007`). A closure without captures MAY escape, and passing any closure to a parameter of function type is not an escape.

#### Scenario: Capturing closures do not escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String): () -> Int = { greet(s) }`, `fun g(s: String): Button = Button({ greet(s) })`, `fun h(slot: &mut (() -> Int), s: String) { *slot = { greet(s) } }`, `fun k(p: () -> Int): () -> Int = p` and `fun m(a: Int, p: () -> Int = { a }): Int = p()` are checked
- **THEN** the borrow checker emits `E5007` for each: for `f` saying that a closure that captures `s` cannot escape and labelling where it is returned, for `g`, `h` and `m` likewise, and for `k` saying that the function-typed parameter `p` cannot escape

#### Scenario: A closure without captures may escape
- **WHEN** `struct Button { val onClick: () -> Int }` and `fun run(f: () -> Int): Int = f()` are declared, and `fun f(): () -> Int = { 1 }`, `fun g(): Button = Button({ 2 })` and `fun h(s: String): Int = run { s.clone(); 3 }` are checked
- **THEN** the file passes `prism check`

#### Scenario: A capturing closure passed to a generic parameter
- **WHEN** `fun <T> keep(x: T): T = x`, `fun <T> remember(calc: () -> T): T = calc()` and `fun greet(s: &String): Int = 1` are declared, and `fun f(s: String): Int { val g = keep({ greet(s) }); return g() }`, `fun k(p: () -> Int): Int = keep(p)()`, `fun h(s: String): Int = remember { greet(s) }` and `fun n(): Int = keep({ 1 })()` are checked
- **THEN** the borrow checker emits `E5007` for `f`, labelling the closure "passed to a generic parameter here", and for `k`, and accepts `h`, whose closure is passed to a function-typed parameter, and `n`, whose closure captures nothing
