## ADDED Requirements

### Requirement: State Built-Ins Are Identified
Calls of `mutableStateOf`, `update`, `weak`, `get` and `SideEffect` MUST be calls of their built-ins, with their declared type arguments, the value type first for `mutableStateOf` and `update`.

#### Scenario: An update call
- **WHEN** `fun f(s: &MutableState<List<Int>>) { s.update { v -> v } }` is checked and its typed tree dumped
- **THEN** the call is a call of the built-in `update` with type argument `List<Int>`, a copy of `s` as its receiver and the closure as its argument

### Requirement: Weak Captures Are Recorded
A capture that a closure takes through a weak handle MUST be recorded with the mode weak, and the closure MUST record as its reason the `attached` parameter its value is given to. `prism dump-typed-tree` MUST print such a capture as `weak` followed by its path.

#### Scenario: A handler's weak capture in the tree
- **WHEN** `struct Todo { val id: Int, val done: MutableState<Bool> }` and `fun keep(f: attached (Bool) -> Unit) {}` are declared and `prism dump-typed-tree` runs on `fun row(todo: &Todo) { keep({ todo.done.value = it }) }`
- **THEN** the closure's captures print as `weak todo.done`, with the reason "passed to an attached parameter here"

## MODIFIED Requirements

### Requirement: State Values Are Identified
A read of a `Copy` `s.value` MUST be a call of the built-in state read with a shared borrow of the state object as its receiver, copied. Any other `s.value` MUST be a state place node over a shared borrow of the state object, of the value's type: the target of an assignment or compound assignment, or a place read in place, borrowed, borrowed mutably or projected.

#### Scenario: A Copy read and a write
- **WHEN** `fun f(s: &MutableState<Int>) { val n = s.value; s.value = n + 1 }` is checked and its typed tree dumped
- **THEN** the initializer of `n` is a call of the built-in `value` with a copy of the reference `s` as its receiver, and the assignment's target is a state place over a copy of `s`

#### Scenario: A borrowed list state
- **WHEN** `compose fun C(s: &State<List<Int>>) { for (x in s.value) { } }` is checked and its typed tree dumped
- **THEN** the loop's iterable is an iteration borrow of a state place over a copy of `s`

#### Scenario: A list state changed in place
- **WHEN** `fun g(s: &MutableState<List<Int>>) { s.value.add(1) }` is checked and its typed tree dumped
- **THEN** the receiver of `add` is a mutable receiver borrow of a state place over a copy of `s`

## REMOVED Requirements

### Requirement: State Operations Are Identified
**Reason**: `edit` is removed, so "An edit call" no longer holds.
**Migration**: See "State Built-Ins Are Identified", whose scenario identifies an `update` call.
