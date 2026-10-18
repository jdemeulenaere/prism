# Typed Tree Specification Delta

## MODIFIED Requirements

### Requirement: Closures Record Their Captures
A closure MUST record its parameters as bindings, whether it is composable, and its captures: the bindings of enclosing scopes that its body uses, each once, in order of first use. Each capture MUST record whether the closure borrows it, moves it or copies it (see `references`), and a closure that owns a capture MUST record the reason: the position that makes it escape, or the assignment that stores it outside a captured binding's block. How a borrowed capture is borrowed, shared or mutable, is not recorded.

#### Scenario: Captured bindings
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and a body of `fun g(a: Int, b: Int): Int` runs `return run { b + a + b }`
- **THEN** the closure captures `b` then `a`, each once, both borrowed

#### Scenario: Nested closures capture through their parent
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun g(a: Int): Int = run { run { a } }` is checked
- **THEN** both closures capture `a`

#### Scenario: Captures taken by value
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `prism dump-typed-tree` runs on `fun f(s: String, n: Int): () -> Int { var count = 0; val h = { count += 1; greet(s) + n }; h(); return { greet(s) + n } }`
- **THEN** the closure `h` captures `count`, `s` and `n` borrowed, and the returned closure captures `s` moved and `n` copied, with the reason "returned here"
