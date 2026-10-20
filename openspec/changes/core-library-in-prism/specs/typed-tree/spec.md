## ADDED Requirements

### Requirement: Extension Property Reads Are Calls
A read of an extension property written in Prism MUST be a call of its getter's declaration path, which names the receiver type and the property (`main::Box.area`), with the getter's type arguments solved from the receiver and one argument, the receiver borrowed shared as a method receiver is. `prism dump-typed-tree` MUST print it as any extension call.

#### Scenario: An extension property read in the typed tree
- **WHEN** `struct Box : Copy { val w: Int, val h: Int }` and `val Box.area: Int get() = w * h` are declared and `prism dump-typed-tree` runs on `fun f(b: &Box): Int = b.area`
- **THEN** the body of `f` is a call of `main::Box.area` whose receiver is a shared receiver borrow of the dereferenced binding `b`, and the body of the getter `main::Box.area` multiplies the fields `w` and `h` of `this`

## MODIFIED Requirements

### Requirement: Built-In String and Scalar Members Are Identified
A string's `length` MUST be a call of the built-in length function, and a string's character `s[i]` a call of the built-in string-character function, each with a shared receiver borrow of the string, the latter also with the index. A scalar's `toString()` MUST be a call of its declaration in `/prism/core` (`prism/core::Int.toString`) with a shared receiver borrow, as any extension call with a shared receiver.

#### Scenario: Built-in string members
- **WHEN** `fun f(s: &String, n: Int): Char = s[n + s.length]` and `fun g(n: &Int): String = n.toString()` are checked
- **THEN** the body of `f` is a call of the built-in string-character function whose receiver is a shared receiver borrow of the dereferenced binding `s` and whose index adds `n` to a call of the built-in length function with a shared receiver borrow of the same place, and the body of `g` is a call of `prism/core::Int.toString` with a shared receiver borrow of the dereferenced binding `n`

#### Scenario: Built-in members through a box
- **WHEN** `fun f(h: Heap<Int>): String = h.toString()` and `fun g(s: Heap<String>) { println(s.length) }` are checked
- **THEN** the body of `f` is a call of `prism/core::Int.toString` with a shared receiver borrow of the dereferenced binding `h`, and the body of `g` calls the built-in length function with a shared receiver borrow of the dereferenced binding `s`

### Requirement: List Operations Are Identified
`listOf(...)` MUST be a call of `prism/collections::listOf`, whose vararg argument is a built-in list construction (vararg-parameters). A list's `size` and its intrinsic member calls MUST be calls of the member's built-in function, with the list as receiver, borrowed as the member's mode says, then the arguments. `isEmpty()`, `isNotEmpty()` and `indices` MUST be calls of their declarations in `/prism/core`, with the element type as type argument and a shared receiver borrow. An element `xs[i]` MUST be an index node over the list place, read through any dereferences, and the index, read, copied, borrowed or written like any place.

#### Scenario: Built-in list calls and element places
- **WHEN** `fun f(xs: &List<Int>, i: Int): Int { var ys = listOf(1, 2); ys.add(xs[i]); if (ys.isEmpty()) return 0; return ys.size + xs[0] }` is checked
- **THEN** `listOf(1, 2)` is a call of `prism/collections::listOf` with type argument `Int`; `ys.add(...)` is a call of the built-in `add` with a mutable receiver borrow of the binding `ys` and a copy of the index node of the dereferenced binding `xs` by the binding `i`; `ys.isEmpty()` is a call of `prism/core::List<T>.isEmpty` with type argument `Int` and a shared receiver borrow of `ys`; `ys.size` is a call of the built-in `size` with a shared receiver borrow of `ys`; and `xs[0]`, an operand read in place, is an index node of the dereferenced binding `xs` by the literal `0`
