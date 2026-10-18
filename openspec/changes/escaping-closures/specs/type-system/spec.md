# Type System Specification Delta

## ADDED Requirements

### Requirement: Escaping Parameters Are Part of Function Types
Whether each parameter of a function or function type is `escaping` MUST be part of its type, rendered `(escaping () -> Unit) -> Unit`. A function type whose parameter is unmarked MUST be assignable to one that is equal except that the parameter is `escaping`, in parameter position contravariantly as usual, and that step MUST keep the value's representation. The reverse MUST be a type mismatch.

#### Scenario: Unmarked parameters convert to escaping ones
- **WHEN** `fun runNow(f: () -> Unit) { f() }`, `fun keepIt(f: escaping () -> Unit) {}`, `fun register(r: (escaping () -> Unit) -> Unit) {}` and `fun now(r: (() -> Unit) -> Unit) {}` are declared, and `fun f() { register(runNow); register(keepIt); now(runNow); val g: (() -> Unit) -> Unit = runNow; register(g) }` and `fun h() { now(keepIt) }` are checked
- **THEN** the type checker accepts `f`, and emits `E4004` for `h`, saying that `(escaping () -> Unit) -> Unit` is not assignable to `(() -> Unit) -> Unit`

#### Scenario: A closure's parameters take the expected flags
- **WHEN** `fun keepIt(f: escaping () -> Unit) {}`, `fun register(r: (escaping () -> Unit) -> Unit) {}` and `fun now(r: (() -> Unit) -> Unit) {}` are declared, and `fun f() { register { g -> keepIt(g) } }` and `fun h() { now { g -> keepIt(g) } }` are checked
- **THEN** `f` passes `prism check`, because the closure's parameter `g` is `escaping`, and the borrow checker emits `E5007` for `h`, saying that the parameter `g` cannot escape

#### Scenario: Escaping types in diagnostics and dumps
- **WHEN** `fun keepIt(f: escaping () -> Unit): Int = 1` is declared and `fun h(): String = keepIt` is checked
- **THEN** the type checker emits `E4005`, naming the type `(escaping () -> Unit) -> Int`, and `prism dump-typed-tree` on a file declaring only `keepIt` shows its parameter `f` as escaping
