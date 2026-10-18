# Code Generation Specification Delta

## ADDED Requirements

### Requirement: Lists Are a Pointer, a Size and a Capacity
A `List<T>` value MUST be 24 bytes with the alignment of a pointer: a pointer to its elements at offset 0, its size (the number of elements) at 8 and its capacity (the number of elements its buffer holds) at 16. Element `i` MUST be at `i` times the element's stride (its size rounded up to its alignment) from the pointer. Aggregates MUST lay out a list like any field of that size and alignment, and `prism dump-layout` MUST show list types with their fields `pointer`, `size` and `capacity`.

#### Scenario: A struct holding a list
- **WHEN** `struct Model { val todos: List<Int>, val count: Int }` is declared and used in `fun main() { val m = Model(listOf(1), 1); println(m.count) }`
- **THEN** `prism dump-layout main.prism` shows `List<Int>` with size 24 and alignment 8, and `Model` with size 32, `todos` at offset 0 and `count` at 24, and `prism run main.prism` prints `1`

### Requirement: A List Owns a Buffer Exactly When It Has Capacity for Sized Elements
A list whose capacity is 0, or whose element type is zero-sized, MUST own no memory, and its pointer MUST then be the element type's alignment, never null. Any other list MUST alone own a buffer of capacity times stride bytes, with the element type's alignment, allocated through the runtime.

#### Scenario: Empty lists and lists of zero-sized elements allocate nothing
- **WHEN** `struct Marker : Copy {}` is declared and `fun main() { var a = listOf<Int>(); var b = listOf<Marker>(); for (i in 0..<3) b.add(Marker()); println(a.size + b.size) }` runs
- **THEN** it prints `3` and makes no allocation

### Requirement: Lists Grow by Doubling
`listOf` with `n` arguments MUST allocate a buffer of capacity exactly `n`, or none for `n = 0`. An `add` or `insert` on a full list MUST first grow it to twice its capacity, or to 4 from 0, moving the elements bitwise through the runtime, so `add` takes amortized constant time; otherwise `add` MUST write in place without calling the runtime. `removeAt`, `removeLast` and `clear` MUST keep the capacity. A buffer size that does not fit in a machine word MUST be the runtime error "out of memory".

#### Scenario: Growing during a million adds
- **WHEN** `fun main() { var xs = listOf<Int>(); for (i in 0..<1000000) xs.add(i); var sum = 0; for (x in xs) sum += x; println(sum) }` runs with `--check-allocations`
- **THEN** it prints `499999500000`, reports no live allocation and exits with status 0

### Requirement: Element Accesses Are Bounds Checked
Computing the address of an element place MUST compare its index, as an unsigned word, with the list's size, and call the runtime's list-index error when it is not smaller, located at the source of the access. `insert` MUST accept an index from 0 to the size inclusive, and `removeLast` MUST call the runtime's empty-list error on an empty list.

#### Scenario: The bounds check in the IR dump
- **WHEN** `fun get(xs: &List<Int>, i: Int): Int = xs[i]` is declared and `prism dump-clif main.prism` runs
- **THEN** the body of `get` loads the size of the list, compares `i` with it, and calls the runtime's list-index error on the failing branch

### Requirement: List Drop and Clone Glue Handle Every Element
Every list type MUST have drop and clone glue, whatever its element type. Dropping a list MUST drop its elements in index order, with the element type's drop glue when it has one, and then free its buffer. Cloning a list MUST allocate a buffer of capacity equal to its size (none when it is empty), copy the elements bitwise, and then replace each element that owns storage, in order, by a clone of the source's. List glue MUST check the stack, as a type may reach itself through a list.

#### Scenario: List glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `fun f(xs: List<String>) {}`
- **THEN** after the file's functions it prints the drop glue of `List<String>`, headed `function main#drop(List<String>)`, which drops each element with the runtime's string drop before freeing the buffer, and `f` calls it on its argument directly

### Requirement: List Equality and Search Are Glue
`==` on lists MUST call equality glue, `<module>#equals(List<T>)`, which compares sizes and then elements in order, stopping at the first difference. `contains` and `indexOf` MUST call index-of glue, `<module>#indexOf(List<T>)`, which compares the given element with each element in order and gives the first equal index or -1. Element comparisons MUST use `==` of the element type, through its equality glue when it has one.

#### Scenario: Equality glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `fun same(a: &List<Int>, b: &List<Int>): Bool = a == b`
- **THEN** it prints the equality glue of `List<Int>`, headed `function main#equals(List<Int>)`, and `same` calls it directly
