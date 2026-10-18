# MIR Delta

## ADDED Requirements

### Requirement: State Operations Are Calls
Every state operation MUST lower to `Call` terminators of state operations, each with the value type as its type argument and the handle passed by copy: creating a cell; reading it, which gives a shared reference to its value; writing it; borrowing it shared, moving its value out or borrowing it mutably, and ending each of those. A state place MUST be the dereference of the reference a read gives.

#### Scenario: A compound assignment through a handle
- **WHEN** `prism dump-mir` runs on `fun f(s: MutableState<Int>) { s.value += 1 }`
- **THEN** the body copies `s` into a temporary once, calls `state read<Int>` with it, adds 1 to a copy of the referent with an overflow check, and calls `state write<Int>` with the temporary and the sum

#### Scenario: An update call
- **WHEN** `prism dump-mir` runs on `fun f(s: MutableState<Int>) { s.update { n -> n * 2 } }`
- **THEN** the body calls `state take<Int>`, calls the closure value with the taken value moved, and calls `state put<Int>` with the result moved

### Requirement: Composed MIR Marks Remember Windows and Side Effects
In composed MIR, the calculation of a `remember` that misses its slot MUST run between a `begin remember` and an `end remember` composer operation, the `end` after the result's slot is written, and a call of `SideEffect` MUST be a `side effect` composer operation taking the effect moved.

#### Scenario: Composed remember of a state
- **WHEN** `prism dump-mir --composed` runs on `compose fun C() { val s = remember { mutableStateOf(0) } }`
- **THEN** the missing path calls `composer begin remember`, then the calculation, then `composer set slot<MutableState<Int>>`, then `composer end remember`
