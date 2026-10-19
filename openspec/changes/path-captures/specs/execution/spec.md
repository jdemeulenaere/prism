## ADDED Requirements

### Requirement: Closures Use the Paths They Capture
A closure that borrows a captured path MUST read and write that place where it lives each time it is called, and a closure that copies a captured path MUST read the value the path held when the closure was built. Neither MUST take the path's binding, which its owner frees exactly once.

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
- **WHEN** `struct Todo { val id: Int, val done: MutableState<Bool> }`, `struct Toggle { val onToggle: (Bool) -> Unit }` and `fun toggle(todo: &Todo): Toggle = Toggle({ todo.done.value = it })` are declared and `fun main() { val t = Todo(1, mutableStateOf(false)); val g = toggle(t); g.onToggle(true); println(t.done.value) }` runs with `--check-allocations`
- **THEN** stdout is `true`, and the exit status is 0: the handler owns a copy of the handle `todo.done`
