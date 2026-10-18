# MIR Specification Delta

## ADDED Requirements

### Requirement: Elements Are Index Projections of Their List
An element `xs[i]` MUST be the place of the list with an index projection by an `Int` local, into which the index is evaluated before the place is used. The projection MUST be valid only on a `List<T>` place, giving a place of type `T`, and, as for a dereference, the validator MUST reject a move out of a place that has one. Assigning to an element MUST drop the old element first, as every assignment drops the old value of its place.

#### Scenario: Reading and writing elements
- **WHEN** `fun f(i: Int): Int { var xs = listOf(1, 2); xs[i + 1] = 3; return xs[0] }` is lowered
- **THEN** the MIR evaluates `i + 1` into a temporary, assigns `3` to the place of `xs` indexed by that temporary, evaluates `0` into another temporary, and returns a copy of `xs` indexed by it

#### Scenario: Borrowing an element of a list behind a reference
- **WHEN** `fun f(xs: &List<String>): &String = &xs[0]` is lowered
- **THEN** the MIR borrows the place `(*_1)` indexed by a temporary holding `0`

### Requirement: For Loops Over Lists Walk Them by Index
A `for` loop over a list MUST evaluate its iterable once into a hidden local holding the loop's borrow, read the size once through it, and skip the loop when it is 0. Each iteration MUST bind the variable, in a fresh scope, to a borrow (of the loop borrow's kind) of the element at a hidden `Int` counter; the latch (`continue`'s target) MUST increment the counter and exit once it equals the size. A temporary list MUST be dropped after the loop ends.

#### Scenario: For loop over a list
- **WHEN** `fun g(x: &Int) {}` is declared and `fun f(xs: List<Int>) { for (x in xs) g(x) }` is lowered
- **THEN** the MIR borrows `_1` once into a local, calls `builtin size` once, branches past the loop when the size is 0, and in each iteration binds `x` to a shared borrow of the borrowed list's dereference indexed by the counter, calls `g`, then compares the incremented counter with the size

### Requirement: List Operations Are Built-In Calls
`listOf`, `size`, `indices` and each built-in list member MUST be `Call` terminators of a built-in callee, printed `builtin listOf`, `builtin size`, `builtin add` and so on, with the list's receiver borrow first (when it has one) and then the arguments, in the order `Calls Name Their Callee` and `Receivers Lead the Operands of Calls` give. `removeAt` and `removeLast` MUST write the removed element to their destination as a value the caller owns.

#### Scenario: Built-in list calls
- **WHEN** `fun f(): Int { var xs = listOf(1, 2); xs.add(3); return xs.removeAt(0) }` is lowered
- **THEN** the MIR calls `builtin listOf` with `1` and `2` into `xs`, `builtin add` with a mutable borrow of `xs` and `3`, and `builtin removeAt` with a mutable borrow of `xs` and `0` into the return place
