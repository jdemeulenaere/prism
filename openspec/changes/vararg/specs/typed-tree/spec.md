## ADDED Requirements

### Requirement: Vararg Arguments Are List Constructions
A call's argument for a vararg parameter MUST be a call of the built-in `varargList` holding the elements in source order, with the element type as its type argument and type `List<E>`, or the spread's operand, consumed. Its evaluation order MUST place the vararg at its first element's position, or, with no element, after the written arguments before it. `prism dump-typed-tree` MUST print the construction as `call builtin varargList<E>`.

#### Scenario: Vararg arguments in the typed tree
- **WHEN** `fun sum(vararg xs: Int): Int = xs.size` is declared and `prism dump-typed-tree` runs on `fun f(ys: List<Int>): Int = sum(1, 2) + sum() + sum(*ys)`
- **THEN** the first call of `main::sum` has one argument, `call builtin varargList<Int>: List<Int>` with the literals `1` and `2`; the second has a `varargList` call with no argument; and the third has the move of `ys`
