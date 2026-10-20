## ADDED Requirements

### Requirement: Cursor Loops Call the Protocol
A cursor loop MUST lower to: a borrow of the iterable for the whole loop, a cursor local assigned the call of `iterator` with that borrow reborrowed shared, a loop header calling `hasNext` with a reborrow and a shared borrow of the cursor and branching on its result, a body block that assigns `next`'s result (called with a reborrow of the loop's mode and a mutable borrow of the cursor) to the variable, and an exit dropping the cursor. `continue` MUST jump to the header.

#### Scenario: Cursor loop MIR
- **WHEN** `prism dump-mir main.prism` runs on a file declaring the `Bag` cursor protocol of `operators` and `fun f(b: &Bag) { for (item in b) { if (item.name == "x") continue } }`
- **THEN** the MIR of `f` calls `iterator` once before the loop, calls `hasNext` in the header block with `&cursor`, calls `next` with `&mut cursor` at the start of each iteration, jumps from the `continue` to the header, and marks the cursor's storage dead on the exit edge

### Requirement: Compound Assignments Through Operator Functions Lower Once
A compound assignment through an operator function MUST evaluate its target place once, call the function with that place as the receiver (borrowed, or moved for a consuming receiver) and the value, and assign the result to the same place, dropping the old value first unless it was moved.

#### Scenario: Compound assignment MIR
- **WHEN** `prism dump-mir main.prism` runs on a file declaring `struct Text { val s: String }`, `operator fun Text.plus(other: &Text): Text = Text(s + other.s)` and `fun f(xs: &mut List<Text>, i: Int, t: &Text) { xs[i] += t }`
- **THEN** the MIR of `f` computes the index place once, borrows the element for the call of `plus`, drops the old element and assigns the call's result to that same place

#### Scenario: Consuming compound assignment MIR
- **WHEN** `prism dump-mir main.prism` runs on a file declaring `struct Text { val s: String }`, `operator consuming fun Text.plus(other: &Text): Text = Text(s + other.s)` and `fun g(t: &Text): Text { var a = Text("x"); a += t; return a }`
- **THEN** the MIR of `g` moves `a` into the call of `plus` and assigns the call's result to `a`, with no drop of the moved value
