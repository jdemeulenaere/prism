# Typed Tree Specification Delta

## MODIFIED Requirements

### Requirement: Loops and Jumps Are Resolved
`while`, `do`/`while` and `for` loops MUST be statements; a `for` loop MUST bind its variable as a binding of the body, hold its iterable and record how it iterates: over a range, held by value, or over a list, held as an explicit borrow of the list (shared or mutable) whose origin is the loop. Every `break` and `continue` MUST name the loop it leaves or continues.

#### Scenario: Break names the innermost loop
- **WHEN** a body runs `for (i in 0..<3) { while (true) { break } }`
- **THEN** the `break` names the `while` statement, not the `for` statement

#### Scenario: A loop over a list holds a borrow of it
- **WHEN** `fun f(xs: List<Int>) { var ys = listOf(1); for (x in xs) {}; for (y in &mut ys) {} }` is checked
- **THEN** the first loop iterates a list through a shared borrow of the binding `xs` whose origin is the loop and binds `x` of type `&Int`, and the second iterates through the written mutable borrow of `ys` and binds `y` of type `&mut Int`

### Requirement: Assignments Are Statements Over Places
An assignment MUST be a statement whose target is a place: a binding (re-pointing it when it is a reference), a field path, an element of a list, or a dereference for a write through a reference. Its value is consumed.

#### Scenario: Write through a reference
- **WHEN** `fun f(r: &mut Int) { *r = 5 }` is checked
- **THEN** the assignment's target is the dereference of the binding `r` and its value the literal `5`

#### Scenario: Re-pointing a reference variable
- **WHEN** a body of `fun f(a: Int, b: Int)` runs `var r = &a; r = &b`
- **THEN** the assignment's target is the binding `r` and its value a written shared borrow of the binding `b`

#### Scenario: Writing an element
- **WHEN** a body of `fun f(i: Int)` runs `var xs = listOf(1); xs[i] = 2`
- **THEN** the assignment's target is the index node of the binding `xs` by the binding `i`, and its value the literal `2`

## ADDED Requirements

### Requirement: List Operations Are Identified
`listOf(...)` MUST be a call of the built-in list-construction function, with the element type as its type argument and each element consumed. A list's `size`, `indices` and built-in member calls MUST be calls of the member's built-in function with the list as receiver, borrowed as the member's mode says, then the arguments. An element `xs[i]` MUST be an index node over the list place, read through any dereferences, and the index, read, copied, borrowed or written like any place.

#### Scenario: Built-in list calls and element places
- **WHEN** `fun f(xs: &List<Int>, i: Int): Int { var ys = listOf(1, 2); ys.add(xs[i]); return ys.size + xs[0] }` is checked
- **THEN** `listOf(1, 2)` is a call of the built-in list-construction function with type argument `Int`; `ys.add(...)` is a call of the built-in `add` with a mutable receiver borrow of the binding `ys` and a copy of the index node of the dereferenced binding `xs` by the binding `i`; `ys.size` is a call of the built-in `size` with a shared receiver borrow of `ys`; and `xs[0]`, an operand read in place, is an index node of the dereferenced binding `xs` by the literal `0`
