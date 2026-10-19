## ADDED Requirements

### Requirement: States Owned by Data Run
A state object held by a struct, a list element or a local MUST be created, read, written and borrowed in place like any value, and its state MUST be retired exactly once, when its owner drops it, and freed exactly once.

#### Scenario: Items holding their own states
- **WHEN** `struct Todo { val id: Int, val done: MutableState<Bool> }` is declared and `main` builds `val todos = mutableStateOf(listOf(Todo(1, mutableStateOf(false)), Todo(2, mutableStateOf(true))))`, runs `todos.value[0].done.value = true`, `val removed = todos.value.removeAt(1)`, `println(removed.done.value)`, `var left = 0`, `for (t in todos.value) { if (!t.done.value) { left += 1 } }` and `println(left)`, with `--check-allocations`
- **THEN** stdout is `true` and `0`, one per line, and the exit status is 0

### Requirement: Retired Cells Are Freed Before the Allocation Check
Under `--check-allocations`, when `main` returns, every retired cell MUST be freed, its value dropped, after the top-level values are released and before compositions still live are reported and the allocation check reports. Without the check, cells still retired at exit MUST NOT be freed.

#### Scenario: A state holding a composition
- **WHEN** `compose fun Empty() { }` and `fun main() { val s = mutableStateOf(composition { Empty() }) }` run with `--check-allocations`
- **THEN** the program exits with status 0: freeing the retired cell disposes the composition before live compositions are counted

#### Scenario: A top-level state
- **WHEN** `val Count: MutableState<Int> = mutableStateOf(1)` and `fun main() { Count.value += 1; println(Count.value) }` run with `--check-allocations`
- **THEN** stdout is `2`, and the exit status is 0

## REMOVED Requirements

### Requirement: Program Cells Are Released Before the Allocation Check
**Reason**: Cells no longer have a program owner; a state belongs to its state object, and the cells left at exit are retired ones.
**Migration**: See "Retired Cells Are Freed Before the Allocation Check", which keeps the scenario as "A state holding a composition".
