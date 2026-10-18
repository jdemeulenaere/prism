# Typed Tree Specification Delta

## ADDED Requirements

### Requirement: Boxes Are Read Through Dereferences
Every implicit read through a `Heap<T>` MUST be explicit in the typed tree as a dereference of the box: a copy-out is a copy of a dereference, a box lending its contents to a reference parameter is a borrow of a dereference with origin argument, and a borrowed box converted to borrowed contents is a borrow of a dereference with origin conversion. No coercion node MUST convert one `Heap` type to another.

#### Scenario: Lending a box's contents
- **WHEN** `fun eval(e: &Int): Int = e` is declared and `fun f(h: Heap<Int>, r: &Heap<Int>): Int = eval(h) + eval(r)` is checked
- **THEN** the first argument is a shared argument borrow of the dereferenced binding `h`, and the second a shared argument borrow of the dereference of the dereferenced binding `r`

#### Scenario: Copying a box's contents out
- **WHEN** `fun f(h: Heap<Int>): Int = h` is checked
- **THEN** the body's value is a copy of the dereferenced binding `h`

## MODIFIED Requirements

### Requirement: Built-In Operations Are Identified
The built-in `clone()` MUST be a call of the built-in clone function with a shared receiver borrow, never a call of a declaration. A call of `print` or `println` (see `type-system`) MUST be a call of the built-in print or print-line function, its argument copied out of a reference when `Copy` and borrowed when a `String`. A string's `length` MUST be a call of the built-in length function, and a string's character `s[i]` a call of the built-in string-character function, each with a shared receiver borrow of the string, the latter also with the index. A scalar's `toString()` MUST be a call of the built-in to-string function with the receiver copied out of any reference. `Heap(value)` MUST be a call of the built-in heap-allocate function, with the contents' type as its type argument and the value consumed, and `h.unbox()` a call of the built-in unbox function with the box moved as its receiver; a `consuming fun` receiver reached through boxes MUST be wrapped in one unbox call per box. A range `a..b` or `a..<b` MUST be a range node of the built-in `Range` type that records whether its end is included.

#### Scenario: Built-in clone
- **WHEN** `struct User { val name: String }` is declared and `fun f(u: &User): String = u.name.clone()` is checked
- **THEN** the body's value is a call of the built-in clone function whose receiver is a shared receiver borrow of field 0 of the dereferenced binding `u`

#### Scenario: Built-in output calls
- **WHEN** `fun f(r: &Int, s: String) { println(r); print(s) }` is checked
- **THEN** the body calls the built-in print-line function with a copy-out of the binding `r`, then the built-in print function with a shared argument borrow of the binding `s`

#### Scenario: Built-in string members
- **WHEN** `fun f(s: &String, n: Int): Char = s[n + s.length]` and `fun g(n: &Int): String = n.toString()` are checked
- **THEN** the body of `f` is a call of the built-in string-character function whose receiver is a shared receiver borrow of the dereferenced binding `s` and whose index adds `n` to a call of the built-in length function with a shared receiver borrow of the same place, and the body of `g` is a call of the built-in to-string function with a copy-out of the binding `n`

#### Scenario: Built-in box operations
- **WHEN** `struct User { val name: String }` and `consuming fun User.close(): Int = 1` are declared, and `fun f(u: User): User = Heap(u).unbox()` and `fun g(h: Heap<User>): Int = h.close()` are checked
- **THEN** the body of `f` is a call of the built-in unbox function whose receiver is a call of the built-in heap-allocate function with type argument `User` and a move of the binding `u`, and the body of `g` calls `close` with a call of the built-in unbox function on a move of the binding `h` as its receiver

#### Scenario: Half-open range
- **WHEN** a body of `fun f(n: Int)` runs `for (i in 0..<n) {}`
- **THEN** the loop binds `i` and iterates a range node of type `Range` from the literal `0` to the binding `n`, with its end excluded
