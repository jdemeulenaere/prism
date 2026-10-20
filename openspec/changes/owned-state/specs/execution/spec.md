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

## MODIFIED Requirements

### Requirement: Closures Use the Paths They Capture
A closure that borrows a captured path MUST read and write that place where it lives each time it is called, a closure that copies a captured path MUST read the value the path held when the closure was built, and one that captures a state path weakly MUST reach the state the path held then. None MUST take the path's binding, which its owner frees exactly once.

#### Scenario: A closure writes one field while another is written
- **WHEN** `struct P { var x: Int, var y: Int }` is declared and `fun main() { var p = P(1, 2); val bump = { p.x += 10 }; p.y += 1; bump(); bump(); println(p.x); println(p.y) }` runs with `--check-allocations`
- **THEN** stdout is `21` and then `3`, and the exit status is 0

#### Scenario: A copied path keeps the value it was built with
- **WHEN** `struct P { val x: Int }` and `fun make(p: &P): () -> Int = { p.x * 10 }` are declared and `fun main() { var p = P(1); val f = make(p); p = P(2); println(f()); println(p.x) }` runs with `--check-allocations`
- **THEN** stdout is `10` and then `2`, and the exit status is 0

#### Scenario: A closure copying a field leaves its binding to its owner
- **WHEN** `struct User { val id: Int, val name: String }` and `struct Button { val onClick: () -> Int }` are declared and `fun main() { val u = User(7, "a" + "b"); val b = Button({ u.id * 2 }); println(b.onClick()); println(u.name) }` runs with `--check-allocations`
- **THEN** stdout is `14` and then `ab`, no live allocation is reported, and the exit status is 0

#### Scenario: A handler writes a state field of a borrowed item
- **WHEN** `struct Todo { val id: Int, val done: MutableState<Bool> }`, `fun deliver(v: Bool, f: attached (Bool) -> Unit) { f(v) }` and `fun toggle(todo: &Todo) { deliver(true, { todo.done.value = it }) }` are declared and `fun main() { val t = Todo(1, mutableStateOf(false)); toggle(t); println(t.done.value) }` runs with `--check-allocations`
- **THEN** stdout is `true`, and the exit status is 0: the handler holds a weak handle of `todo.done`, and `t` frees the state once

## REMOVED Requirements

### Requirement: Program Cells Are Released Before the Allocation Check
**Reason**: Cells no longer have a program owner; a state belongs to its state object, and the cells left at exit are retired ones.
**Migration**: See "Retired Cells Are Freed Before the Allocation Check", which keeps the scenario as "A state holding a composition".
