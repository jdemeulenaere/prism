# References Delta

## ADDED Requirements

### Requirement: Borrows of a State's Value Hold No Loan
A borrow of a state place, taken in a composable context, MUST hold no loan of any local and MUST be valid until the body that took it ends, as a borrow of a top-level `val` is: no write of a local conflicts with it, and a body MAY return it under any elision rule. The runtime keeps it valid, since no state is written and no cell is freed while a pass runs.

#### Scenario: A borrow of a state outlives later calls
- **WHEN** `compose fun Leaf() { }` and `compose fun C(s: State<String>) { val r = &s.value; Leaf(); println(r) }` are declared and `fun main() { val s = mutableStateOf("a" + "b"); val c = composition { C(s) } }` runs with `--check-allocations`
- **THEN** stdout is `ab`, and the exit status is 0

#### Scenario: A composable returns a borrow of a state's value
- **WHEN** `compose fun items(s: State<List<Int>>): &List<Int> = &s.value` and `compose fun C(s: State<List<Int>>) { println(items(s).size) }` are checked
- **THEN** the file typechecks and passes the borrow checker

### Requirement: Values Written Into a State Escape
A value written into a state, by `mutableStateOf`, `value =`, the result of `update`'s transform or a compound assignment, MUST be at an owning position: a closure written there owns its captures, and a function value that may hold a borrow is `E5007`, as for a generic parameter.

#### Scenario: A borrowing closure stored in a state rejected
- **WHEN** `fun f(s: MutableState<() -> Int>, r: &Int) { s.value = { r + 1 } }` is checked
- **THEN** the borrow checker emits `E5007`, labelling the capture of `r`

#### Scenario: A handler stored in a state owns its captures
- **WHEN** `fun main() { val n = 2; val s = mutableStateOf({ 0 }); s.value = { n * 3 }; s.read { f -> println(f()) } }` runs with `--check-allocations`
- **THEN** stdout is `6`, and the exit status is 0
