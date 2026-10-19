## ADDED Requirements

### Requirement: Widened Captures Record the Path They Widened
An owned capture of a whole binding that the closure uses only through paths it cannot copy (see `ownership`, "Owning Closures Copy the Paths They Can") MUST record the first such path and why it was not copied: its type is not `Copy`, its binding is a `var`, it dereferences a `&mut` reference, or it reads a `var` field through a reference. `prism dump-typed-tree` MUST print it after the capture.

#### Scenario: A capture widened for a non-Copy field
- **WHEN** `struct User { val id: Int, val name: String }` is declared and `prism dump-typed-tree` runs on `fun f(u: User): () -> Int = { u.name.length + u.id }`
- **THEN** the closure captures `u` moved, widened from the path `u.name`, whose type is not `Copy`, with the reason "returned here"

## MODIFIED Requirements

### Requirement: Closures Record Their Captures
A closure MUST record its parameters as bindings, whether it is composable, and its captures: the paths its body uses of bindings of enclosing scopes (see `references`), each once, by binding in order of first use. Each capture MUST record whether the closure borrows, moves or copies it, and a closure owning a capture MUST record why: the position that makes it escape, or the assignment storing it outside a captured binding's block. Whether a borrow is shared or mutable is not recorded.

#### Scenario: Captured bindings
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and a body of `fun g(a: Int, b: Int): Int` runs `return run { b + a + b }`
- **THEN** the closure captures `b` then `a`, each once, both borrowed

#### Scenario: Nested closures capture through their parent
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `fun g(a: Int): Int = run { run { a } }` is checked
- **THEN** both closures capture `a`

#### Scenario: Captures taken by value
- **WHEN** `fun greet(s: &String): Int = 1` is declared and `prism dump-typed-tree` runs on `fun f(s: String, n: Int): () -> Int { var count = 0; val h = { count += 1; greet(s) + n }; h(); return { greet(s) + n } }`
- **THEN** the closure `h` captures `count`, `s` and `n` borrowed, and the returned closure captures `s` moved and `n` copied, with the reason "returned here"

#### Scenario: Captured paths
- **WHEN** `struct Todo { val id: Int, val title: String }`, `struct P { var x: Int, var y: Int }` and `fun run(f: () -> Int): Int = f()` are declared and `prism dump-typed-tree` runs on `fun f(t: &Todo, r: &Int, n: Int): () -> Int { var p = P(1, 2); run { p.x += n; p.x }; return { t.id + r + n } }`
- **THEN** the first closure prints its captures as `borrow p.x` and `borrow n`, and the returned closure as `copy t.id`, `copy *r` and `copy n`, with the reason "returned here"

#### Scenario: Nested closures capture their children's paths
- **WHEN** `struct Todo { val id: Int, val title: String }`, `fun store(f: escaping () -> Int): Int = 1` and `fun run(f: () -> Int): Int = f()` are declared and `prism dump-typed-tree` runs on `fun f(t: &Todo): Int = run { store { t.id } }`
- **THEN** the outer closure captures the path `t.id` borrowed, and the inner one captures `t.id` copied, with the reason "passed to an escaping parameter here"
