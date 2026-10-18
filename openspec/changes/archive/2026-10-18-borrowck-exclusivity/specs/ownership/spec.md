# Ownership Specification Delta

## ADDED Requirements

### Requirement: A Closure Does Not Move Its Captures
Inside a closure, a non-`Copy` variable that the closure captures MUST NOT be moved, because the closure only borrows it (see `references`). Such a move MUST be rejected by the type checker with `CannotMoveOut` (`E4017`), naming the source "captured variable `s`, which the closure borrows", with help suggesting `.clone()` for a `Clone` type. A `Copy` capture MUST be copied, and a variable declared inside the closure MAY be moved.

#### Scenario: Moving a captured variable inside a closure
- **WHEN** `fun take(s: String) {}`, `fun run(f: () -> Int): Int = f()` and `struct Token { val id: Int }` are declared, and `fun f(s: String): Int = run { take(s); 1 }`, `fun g(t: Token): Int = run { val u = t; 1 }`, `fun h(s: String): Int = run { take(s.clone()); 1 }`, `fun k(n: Int): Int = run { val m = n; m }` and `fun l(): Int = run { val s = "a"; take(s); 1 }` are checked
- **THEN** the type checker emits `E4017` for `s` in `f`, naming the captured variable `s` and suggesting `s.clone()`, and for `t` in `g`, without a `.clone()` suggestion, and accepts `h`, `k` and `l`

#### Scenario: A closure in a default value moving an earlier parameter
- **WHEN** `fun take(s: String) {}` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(a: String, b: Int = run { take(a); 1 }): Int = b` is checked
- **THEN** the type checker emits `E4017` for `a`, naming the captured variable `a`

#### Scenario: Moving a captured variable after the closure ran
- **WHEN** `fun take(s: String) {}`, `fun greet(s: &String): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `fun f(s: String) { run { greet(s) }; take(s) }` is checked
- **THEN** the file passes `prism check`, because the closure's borrow of `s` ends when `run` returns
