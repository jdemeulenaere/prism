## ADDED Requirements

### Requirement: Attached Parameters Are Part of Function Types
Whether each parameter of a function or function type is `attached` MUST be part of its type, rendered `(attached () -> Unit) -> Unit`. A function type MUST be assignable to one equal to it except that a parameter unmarked in it is `attached`, or one `attached` in it is `escaping`, keeping its representation. Any other difference of the flags MUST be a type mismatch.

#### Scenario: Attached parameters in conversions
- **WHEN** `fun runNow(f: () -> Unit) { f() }`, `fun keepIt(f: attached () -> Unit) {}`, `fun store(f: escaping () -> Unit) {}`, `fun register(r: (attached () -> Unit) -> Unit) {}` and `fun registerEscaping(r: (escaping () -> Unit) -> Unit) {}` are declared, and `fun f() { register(runNow); register(keepIt); registerEscaping(keepIt) }` and `fun h() { register(store) }` are checked
- **THEN** the type checker accepts `f`, and emits `E4004` for `h`, saying that `(escaping () -> Unit) -> Unit` is not assignable to `(attached () -> Unit) -> Unit`

#### Scenario: An escaping value given to an attached parameter
- **WHEN** `fun keepIt(f: attached () -> Unit) {}` is declared and `fun g(h: escaping () -> Unit) { keepIt(h) }` is checked
- **THEN** the file passes `prism check`

## MODIFIED Requirements

### Requirement: Mutable State Handles Convert to Read-Only Ones
A `MutableState<T>` MUST be assignable where a `State<T>` of the same `T` is expected, by value and behind a shared reference (`&MutableState<T>` as `&State<T>`), as a subtype step that keeps the state object, and an argument of type `MutableState<T>` MUST be borrowed implicitly for a `&State<T>` parameter. A `State<T>` MUST NOT convert to `MutableState<T>`, and `MutableState<A>` MUST NOT convert to `State<B>` for another `B`.

#### Scenario: Passing a mutable handle as a read-only one
- **WHEN** `fun show(s: &State<Int>): Int = s.value` and `fun main() { val m = mutableStateOf(7); println(show(m)) }` run
- **THEN** stdout is `7`

#### Scenario: Type arguments inferred through the conversion
- **WHEN** `fun <T : Copy> get(s: &State<T>): T = s.value` and `fun main() { println(get(mutableStateOf(5))) }` run
- **THEN** stdout is `5`: the `MutableState<Int>` argument, borrowed as a temporary, solves `T` as `Int`

#### Scenario: A read-only handle does not become mutable
- **WHEN** `fun f(s: &State<Int>): &MutableState<Int> = s` is checked
- **THEN** the type checker emits `E4005`

### Requirement: Lookups on a Mutable Handle Fall Back to State
A member or extension looked up on a `MutableState<T>` receiver and not found for `MutableState` MUST be looked up for `State<T>`, with the receiver converted, so the members and extensions of `State<T>` apply to both handles.

#### Scenario: Reading through a mutable handle
- **WHEN** `fun <T> State<T>.describe(): String = "state"` and `fun main() { val m = mutableStateOf(2); println(m.value + 1); println(m.describe()) }` run
- **THEN** stdout is `3` and `state`
