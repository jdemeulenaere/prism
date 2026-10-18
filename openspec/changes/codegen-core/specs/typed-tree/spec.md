# Typed Tree Specification Delta

## MODIFIED Requirements

### Requirement: Built-In Operations Are Identified
The built-in `clone()` MUST be a call of the built-in clone function with a shared receiver borrow, never a call of a declaration. A call of the output built-in `print` or `println` (see `type-system`) MUST be a call of the built-in print or print-line function with its argument, if any, adjusted as for a parameter of its type: copied out of a reference when it is `Copy`, and borrowed when it is a `String`. A range `a..b` or `a..<b` MUST be a range node of the built-in `Range` type that records whether its end is included.

#### Scenario: Built-in clone
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: &User): String = u.name.clone()` is checked
- **THEN** the body's value is a call of the built-in clone function whose receiver is a shared receiver borrow of field 0 of the dereferenced binding `u`

#### Scenario: Built-in output calls
- **WHEN** `fun f(r: &Int, s: String) { println(r); print(s) }` is checked
- **THEN** the body calls the built-in print-line function with a copy-out of the binding `r`, then the built-in print function with a shared argument borrow of the binding `s`

#### Scenario: Half-open range
- **WHEN** a body of `fun f(n: Int)` runs `for (i in 0..<n) {}`
- **THEN** the loop binds `i` and iterates a range node of type `Range` from the literal `0` to the binding `n`, with its end excluded
