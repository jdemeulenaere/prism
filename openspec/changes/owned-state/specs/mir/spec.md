## ADDED Requirements

### Requirement: Flags Outside Composition End With Their Scope
A borrow of a state's value that holds the state's flag (compose, "Borrows of a State's Value Outside Composition Hold Its Flag") MUST lower to a call taking the flag, shared or exclusive, before the borrow, and a call ending it where the temporary scope of its statement ends, or where the block of the binding it initializes ends, on every path out of that scope.

#### Scenario: A flag around a call that changes a list state
- **WHEN** `prism dump-mir` runs on `fun f(s: &MutableState<List<Int>>) { s.value.add(1); println(1) }`
- **THEN** the body calls `state begin exclusive<List<Int>>` with a copy of `s` before the call of `add`, and `state end<List<Int>>` after it, before the call of `println`

#### Scenario: A borrow bound to a val ends its flag with its block
- **WHEN** `prism dump-mir` runs on `fun g(s: &MutableState<List<Int>>): Int { val items = &s.value; println(items.size); return 1 }`
- **THEN** the body calls `state begin shared<List<Int>>` before `items` is initialized and `state end<List<Int>>` once, when the block's locals end, on the path that returns

### Requirement: Weak Captures Are Words of the Closure Value
A weak capture MUST be a capture local of the weak reference's type. The enclosing body MUST build the closure value from a weak capture of the captured path's place, which makes the word by the capture function's operation and which the borrow checker sees as a shared borrow held by the closure value. A use of the path in the closure's body MUST be a place that views the local as the state object, never moved or dropped.

#### Scenario: A handler's weak capture in MIR
- **WHEN** `prism dump-mir` runs on a file declaring `struct Todo { val id: Int, val done: MutableState<Bool> }`, `fun keep(f: attached (Bool) -> Unit) {}` and `fun row(todo: &Todo) { keep({ todo.done.value = it }) }`
- **THEN** `row` builds the closure value from `weak (*_1).done`, a weak capture of the place `(*todo).done`, and the closure's body calls `state write<Bool>` on a borrow of a view of its capture local `todo.done`

## MODIFIED Requirements

### Requirement: State Operations Are Calls
Every state operation MUST lower to `Call` terminators of state operations, each with the value type as its type argument and the state object passed by shared borrow: creating a state; reading it, which gives a shared reference to its value tied to that borrow; writing it; taking a shared or exclusive flag and ending it; moving its value out and putting one back; making a weak handle and resolving one. A state place MUST be the dereference of the reference a read gives.

#### Scenario: A compound assignment through a handle
- **WHEN** `prism dump-mir` runs on `fun f(s: &MutableState<Int>) { s.value += 1 }`
- **THEN** the body copies `s` into a temporary once, calls `state read<Int>` with it, adds 1 to a copy of the referent with an overflow check, and calls `state write<Int>` with the temporary and the sum

#### Scenario: An update call
- **WHEN** `prism dump-mir` runs on `fun f(s: &MutableState<Int>) { s.update { n -> n * 2 } }`
- **THEN** the body calls `state take<Int>`, calls the closure value with the taken value moved, and calls `state put<Int>` with the result moved

### Requirement: Composed MIR Marks Remember Windows and Side Effects
In composed MIR, the calculation of a `remember` that misses its slot MUST run between a `begin_remember` and an `end_remember` composer operation, the end after the result's slot is written, and `remember` of a non-`Copy` type MUST give a shared reference to the slot's value. A call of `SideEffect` MUST be a `side_effect` composer operation taking the effect moved.

#### Scenario: Composed remember of a state
- **WHEN** `prism dump-mir --composed` runs on `compose fun C() { val s = remember { mutableStateOf(0) } }`
- **THEN** the missing path calls `composer begin_remember`, then the calculation, then `composer set_slot<MutableState<Int>>`, then `composer end_remember`, and `s` holds a shared reference to the slot's `MutableState<Int>` on both paths
