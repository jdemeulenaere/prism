## ADDED Requirements

### Requirement: Composables With Explicit Groups
A composable annotated `@ExplicitGroupsComposable` MUST start no group of its own, keep no parameter, never skip and be no restart scope: its body runs in its caller's group, at the call's position. Outside closures, its body MUST make no composable call inside a branch, a loop, the right operand of `&&` or `||`, the fallback of `?:`, a safe call or a `when` with composable tests, or after a `return` taken in one of those, and MUST NOT call `remember` or `key` (`E4029`).

#### Scenario: A side effect adds no group
- **WHEN** `compose fun Log(n: Int) { SideEffect { println("effect $n") } }` and `fun main() { val c = composition { Log(1) }; print(c.dump()) }` run with `--check-allocations`
- **THEN** stdout is `effect 1`, then the dump, in which `Log`'s `restart` group holds one `slot Int` and no group

#### Scenario: A side effect never skips
- **WHEN** `compose fun Log(n: Int, tick: Int) { println("log $tick"); SideEffect { println("effect $n") } }`, `fun screen(t: Int): compose () -> Unit = { Log(1, t) }` and `fun main() { val c = composition(screen(1)); c.setContent(screen(2)) }` run with `--check-allocations`
- **THEN** stdout is `log 1`, `effect 1`, `log 2` and `effect 1`: on the second pass the effect, which captures only an equal `Int`, is queued again, since `SideEffect` never skips

#### Scenario: Grouped code in a body with explicit groups rejected
- **WHEN** a library test module declares `compose fun Leaf() {}`, `@ExplicitGroupsComposable compose fun A(on: Bool) { if (on) { Leaf() } }` and `@ExplicitGroupsComposable compose fun B() { val n = remember { 1 } }`, or `@ExplicitGroupsComposable compose fun C(on: Bool) { if (on) { return }; Leaf() }`
- **THEN** the type checker emits `E4029` at the call of `Leaf` in `A`, at `remember` in `B` and at the call of `Leaf` in `C`, saying that a composable with explicit groups composes only outside branches and loops, and remembers nothing

### Requirement: Root Content Is Called With the Force Bit
When the compose runtime runs a composition's root and its content is composable content given to it by `composition`, `setContent` or `runApp`, it MUST call that content with the composition's composer and one change word holding only the force bit, so that the content runs even when it is a restartable composable whose scope is valid.

#### Scenario: A composable function as root content
- **WHEN** `compose fun Screen() { println("screen") }` and `fun main() { val c = composition(Screen); c.setContent(Screen) }` run with `--check-allocations`
- **THEN** stdout is `screen` twice, and the exit status is 0

## MODIFIED Requirements

### Requirement: Compositions Are Owned Values
`Composition` MUST be neither `Copy` nor `Clone`. It MUST be a library struct of `/prism/compose/runtime` over a private handle, so no program constructs or takes one apart, and its drop hook MUST dispose it: every group it holds is removed and released, with its kept and remembered values, and its root content is released. Dropping a composition while a pass runs MUST be the misuse `composition operation during a pass` (compose, "One Pass at a Time").

#### Scenario: Dropping a composition frees everything
- **WHEN** `compose fun Plus(onClick: escaping () -> Unit) { println("plus") }` and `fun main() { val c = composition { val a = 1; val b = 2; Plus(onClick = { println(a + b) }) } }` run with `--check-allocations`
- **THEN** stdout is `plus`, and the program exits with status 0: the handler, the copy of it that `Plus` keeps and the root content are freed when `c` is dropped

#### Scenario: A composition is not copied
- **WHEN** `fun main() { val c = composition { }; val d = c; val e = c }` is checked
- **THEN** the borrow checker reports `E5001` for the second use of `c`

#### Scenario: A composition cannot be forged
- **WHEN** a file importing `/prism/compose/runtime` declares `fun forge(): Composition = Composition(1)` and `fun open(c: &Composition): Int { val (handle) = c; return handle }`
- **THEN** the type checker emits `E4007` for the construction and for `handle`, each naming the private field `handle` of `prism/compose/runtime`

### Requirement: Composition Operations Are Rejected in Composable Contexts
Calling a function annotated `@DrivesComposition` (`composition`, `setContent` and `recompose` of `/prism/compose/runtime`, and `runApp` of `/prism/compose/ui`) in a composable context MUST be rejected with `E4030` (`CompositionInComposable`), at the call, with a note that compositions are started and driven outside composition. In a non-composable closure written inside a composable, such calls MUST be accepted. Such a function called during a pass through a function value, which the type checker does not follow, MUST end the program with the misuse `composition operation during a pass`, `runApp` before it opens any window.

#### Scenario: Starting a composition inside a composable
- **WHEN** `compose fun Screen(c: &Composition) { val d = composition { }; c.recompose() }` is checked
- **THEN** the type checker emits `E4030` at `composition` and at `recompose`

#### Scenario: A handler may start a composition
- **WHEN** `fun later(f: () -> Unit) { f() }` and `compose fun Screen() { later { val d = composition { } } }` are checked
- **THEN** the file typechecks

#### Scenario: A composition function called through a function value
- **WHEN** `compose fun Screen(start: (attached compose () -> Unit) -> Composition) { println("screen"); val d = start { } }` and `fun main() { val c = composition { Screen(composition) } }` run, or `runApp` is given likewise to a composable that calls it, headless
- **THEN** stdout is `screen`, and the program ends with `panic: composition misuse: composition operation during a pass` and status 101
