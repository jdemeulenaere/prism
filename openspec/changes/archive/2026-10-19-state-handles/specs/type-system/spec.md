# Type System Delta

## ADDED Requirements

### Requirement: Mutable State Handles Convert to Read-Only Ones
A `MutableState<T>` MUST be assignable where a `State<T>` of the same `T` is expected, as a subtype step that keeps the handle, wherever `&mut T` converts to `&T`. A `State<T>` MUST NOT convert to `MutableState<T>`, and `MutableState<A>` MUST NOT convert to `State<B>` for another `B`.

#### Scenario: Passing a mutable handle as a read-only one
- **WHEN** `fun show(s: State<Int>): Int = s.value` and `fun main() { val m = mutableStateOf(7); println(show(m)) }` run
- **THEN** stdout is `7`

#### Scenario: Type arguments inferred through the conversion
- **WHEN** `fun <T : Copy> get(s: State<T>): T = s.value` and `fun main() { println(get(mutableStateOf(5))) }` run
- **THEN** stdout is `5`: the `MutableState<Int>` argument solves `T` as `Int`

#### Scenario: A read-only handle does not become mutable
- **WHEN** `fun f(s: State<Int>): MutableState<Int> = s` is checked
- **THEN** the type checker emits `E4005`

### Requirement: Lookups on a Mutable Handle Fall Back to State
A member or extension looked up on a `MutableState<T>` receiver and not found for `MutableState` MUST be looked up for `State<T>`, with the receiver converted, so the members and extensions of `State<T>` apply to both handles.

#### Scenario: Reading through a mutable handle
- **WHEN** `fun <T> State<T>.describe(): String = "state"` and `fun main() { val m = mutableStateOf(2); println(m.read { n -> n + 1 }); println(m.describe()) }` run
- **THEN** stdout is `3` and `state`
