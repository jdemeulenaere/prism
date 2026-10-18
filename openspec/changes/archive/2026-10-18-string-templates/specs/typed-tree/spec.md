# Typed Tree Specification Delta

## ADDED Requirements

### Requirement: Templates Are Nodes Over Their Parts
A string template MUST be a template node of type `String` holding its parts in source order: each text run as its decoded text, and each other part as a value of type `String`, `Int`, `Double`, `Bool` or `Char` read in place, as an operand of an operator is. A part that is a place (a binding, a field, the referent of a reference or a box) MUST be that place, with no move or copy. `prism dump-typed-tree` MUST print the node as `template`, with its parts in order.

#### Scenario: A template over places
- **WHEN** `fun f(n: &Int, s: String): String = "n=$n, s=$s"` is checked
- **THEN** the body's value is a template node whose parts are the text `n=`, the dereferenced binding `n`, the text `, s=` and the binding `s`, with no move, copy or borrow node around either binding

### Requirement: Computed Template Parts Are Temporaries
A computed `String` part, including the result of the `toString()` call that gives a part its text form, MUST be a temporary. A part of type `Nothing` MUST be converted to `String`.

#### Scenario: A toString part is a call in a temporary
- **WHEN** `struct Point : Copy { val x: Int }`, `fun Point.toString(): String = "p"` and `fun f(p: Point, a: String, b: String): String = "$p ${a + b}"` are checked
- **THEN** the template's first value part is a temporary holding a call of `Point.toString` with a shared receiver borrow of the binding `p`, and its second a temporary holding the concatenation of `a` and `b`
