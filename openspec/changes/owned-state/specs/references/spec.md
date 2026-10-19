## ADDED Requirements

### Requirement: Closures Given to Attached Parameters Capture States Through Weak Handles
A closure that owns its captures because its value reaches an `attached` parameter, and no escaping position, MUST capture each path whose type has a capture function (a state object) through a weak handle made where the closure is built, when the path's binding is a `val`, it dereferences no `&mut` reference and every field it reads after a reference is a `val` field. A weak handle MUST hold no borrow and own nothing.

#### Scenario: A row's handler captures its item's state
- **WHEN** `struct Todo { val id: Int, val done: MutableState<Bool> }` and `fun keep(f: attached (Bool) -> Unit) {}` are declared and `fun row(todo: &Todo) { keep({ todo.done.value = it }) }` is checked and its typed tree dumped
- **THEN** the file passes `prism check`, and the closure's only capture is `weak todo.done`

#### Scenario: A state binding captured by root content stays usable
- **WHEN** `fun main() { val s = mutableStateOf(0); val c = composition { println(s.value) }; s.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `0`, `1` and `true`, one per line, and the exit status is 0: the content captured `s` through a weak handle, so `main` still owns and writes it

#### Scenario: A state behind a var field is not captured weakly
- **WHEN** `struct Holder { var s: MutableState<Int> }` and `fun keep(f: attached () -> Unit) {}` are declared and `fun row(h: &Holder) { keep({ h.s.value += 1 }) }` is checked
- **THEN** the borrow checker emits `E5007`, saying that a closure that captures the reference `h` cannot escape, with a note that a closure captures a state read through a reference weakly only when every field on the way is a `val`

### Requirement: Attached Function Values Stay Attached
A value of an `attached` parameter, and a closure holding weak handles, MAY be called, borrowed, captured by a closure that is itself given to an `attached` parameter or does not escape, and passed to an `attached` parameter. Any other escape (returned, stored in a struct, state, list or top-level value, or passed to an `escaping` or generic parameter) MUST be rejected with `AttachedFunctionEscapes` (`E5010`).

#### Scenario: Forwarding an attached handler
- **WHEN** `fun keep(f: attached () -> Unit) {}` is declared and `fun forward(onClick: attached () -> Unit) { keep(onClick) }` and `fun wrap(onClick: attached () -> Unit) { keep { println("x"); onClick() } }` are checked
- **THEN** the file passes `prism check`

#### Scenario: Keeping an attached handler elsewhere rejected
- **WHEN** `struct Button { val onClick: () -> Unit }` and `fun store(f: escaping () -> Unit) {}` are declared and `fun a(onClick: attached () -> Unit): Button = Button(onClick)` and `fun b(onClick: attached () -> Unit) { store(onClick) }` are checked
- **THEN** the borrow checker emits `E5010` at `onClick` in each, saying that an attached function value is kept only by the composition, with help suggesting to declare the receiving parameter `attached`

### Requirement: Borrows of Remembered Values Hold No Loan
A borrow that `remember` gives MUST hold no loan of any local and MUST be valid until the body that took it ends, as a borrow of a top-level `val` is: no write of a local conflicts with it, and a composable MAY return it under any elision rule. The runtime keeps it valid, since no slot is released before its pass ends.

#### Scenario: A composable returns a remembered borrow
- **WHEN** `compose fun items(): &List<Int> = remember { listOf(1, 2) }` and `compose fun C() { println(items().size) }` are declared and `fun main() { val c = composition { C() } }` runs with `--check-allocations`
- **THEN** stdout is `2`, and the exit status is 0

### Requirement: An Escaping Closure Using a Borrowed State Suggests a Weak Reference
When a closure that owns its captures and reaches no `attached` parameter uses a state object through a reference, the `EscapingClosure` (`E5007`) it gets MUST have help suggesting a weak reference made before the closure and resolved inside it.

#### Scenario: A handler kept in a plain function
- **WHEN** `fun later(f: escaping () -> Unit) {}` is declared and `fun f(count: &MutableState<Int>) { later { count.value += 1 } }` is checked
- **THEN** the borrow checker emits `E5007`, saying that a closure that captures the reference `count` cannot escape, with help suggesting `val countRef = count.weak()` before the closure and `countRef.get()` inside it

## MODIFIED Requirements

### Requirement: Values Written Into a State Escape
A value written into a state, by `mutableStateOf`, `value =`, the result of `update`'s transform or a compound assignment, MUST be at an owning position: a closure written there owns its captures, without weak handles, and a function value that may hold a borrow is `E5007`, and an `attached` one `E5010`, as for a generic parameter.

#### Scenario: A borrowing closure stored in a state rejected
- **WHEN** `fun f(s: &MutableState<() -> Int>, r: &String) { s.value = { r.length + 1 } }` is checked
- **THEN** the borrow checker emits `E5007`, labelling the capture of `r`

#### Scenario: A handler stored in a state owns its captures
- **WHEN** `fun main() { val n = 2; val s = mutableStateOf({ 0 }); s.value = { n * 3 }; val f = &s.value; println(f()) }` runs with `--check-allocations`
- **THEN** stdout is `6`, and the exit status is 0

## REMOVED Requirements

### Requirement: Borrows of a State's Value Hold No Loan
**Reason**: A state object is owned, so a borrow of its value is an ordinary borrow of the state object, tied to it by the elision rules; the no-loan rule now covers the borrows `remember` gives.
**Migration**: See "Borrows of Remembered Values Hold No Loan". `compose fun items(s: &State<List<Int>>): &List<Int> = &s.value` is valid by rule 1.
