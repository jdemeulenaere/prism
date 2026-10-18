# Typed Tree Delta

## ADDED Requirements

### Requirement: State Values Are Identified
A read of a `Copy` `s.value` MUST be a call of the built-in state read with the handle as its receiver, copied. Any other `s.value` MUST be a state place node over the handle expression, of the value's type: the target of an assignment or compound assignment, or, in a composable context, a place read in place, borrowed or projected.

#### Scenario: A Copy read and a write
- **WHEN** `fun f(s: MutableState<Int>) { val n = s.value; s.value = n + 1 }` is checked and its typed tree dumped
- **THEN** the initializer of `n` is a call of the built-in `value` with a copy of `s`, and the assignment's target is a state place over a copy of `s`

#### Scenario: A borrowed list state
- **WHEN** `compose fun C(s: State<List<Int>>) { for (x in s.value) { } }` is checked and its typed tree dumped
- **THEN** the loop's iterable is an iteration borrow of a state place over a copy of `s`

### Requirement: State Operations Are Identified
Calls of `mutableStateOf`, `read`, `update`, `edit` and `SideEffect` MUST be calls of their built-ins, with the value type as their first type argument.

#### Scenario: An edit call
- **WHEN** `fun f(s: MutableState<List<Int>>) { s.edit { v -> v.add(1) } }` is checked and its typed tree dumped
- **THEN** the call is a call of the built-in `edit` with type argument `List<Int>`, the handle as its receiver and the closure as its argument
