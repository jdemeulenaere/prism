## ADDED Requirements

### Requirement: Vararg Lists Are Built Before the Call
A vararg's elements MUST be evaluated into operands in order, then passed to a `Call` terminator of `builtin varargList<E>` writing a temporary, which the call MUST take as a moved operand in the vararg's place; a spread's list MUST be moved as that operand. No other MIR construct MUST know about varargs.

#### Scenario: A vararg call in MIR
- **WHEN** `fun sum(vararg xs: Int): Int = xs.size` is declared and `fun f(a: Int, ys: List<Int>): Int = sum(a, 2) + sum(*ys)` is lowered
- **THEN** the MIR calls `builtin varargList<Int>` with a copy of `_1` and the constant `2` into a temporary, calls `main::sum` with a move of that temporary, and calls `main::sum` again with a move of `_2`

## MODIFIED Requirements

### Requirement: List Operations Are Built-In Calls
A vararg's list, `size`, `indices` and each built-in list member MUST be `Call` terminators of a built-in callee, printed `builtin varargList`, `builtin size`, `builtin add` and so on, with the list's receiver borrow first (when it has one) and then the arguments, in the order `Calls Name Their Callee` and `Receivers Lead the Operands of Calls` give. `removeAt` and `removeLast` MUST write the removed element to their destination as a value the caller owns.

#### Scenario: Built-in list calls
- **WHEN** `fun f(): Int { var xs = listOf(1, 2); xs.add(3); return xs.removeAt(0) }` is lowered
- **THEN** the MIR calls `builtin varargList` with `1` and `2` into a temporary and `prism/collections::listOf` with a move of that temporary into `xs`, then `builtin add` with a mutable borrow of `xs` and `3`, and `builtin removeAt` with a mutable borrow of `xs` and `0` into the return place
