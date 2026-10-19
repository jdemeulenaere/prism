## MODIFIED Requirements

### Requirement: Elements Are Stored Values
A value given to `listOf`, `add` or `insert`, or assigned to an element, MUST be stored in the list as a value given to a parameter of a type parameter is, with the same rules for function values: a closure that may hold borrows MUST be rejected with `E5007`, and one that owns its captures (`escaping-closures`) MUST be stored. An element that is a function value MUST be callable in place (`handlers[0]()`).

#### Scenario: A list of handlers
- **WHEN** `fun f() { var handlers = listOf<() -> Unit>(); handlers.add { println("hi") }; handlers[0]() }` is checked
- **THEN** the file typechecks and borrow-checks successfully

#### Scenario: Storing a borrowing closure rejected
- **WHEN** `fun f(xs: &mut List<() -> Unit>, r: &String) { xs.add { println(r) } }` is checked
- **THEN** the borrow checker emits `E5007`, because the closure holds the borrow `r`

#### Scenario: Storing a closure that copies through a borrow
- **WHEN** `fun main() { var handlers = listOf<() -> Unit>(); val n = 7; val r = &n; handlers.add { println(r) }; handlers[0]() }` runs with `--check-allocations`
- **THEN** stdout is `7`, and the exit status is 0: the stored closure owns a copy of the `Int` that `r` refers to
