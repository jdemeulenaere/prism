# Execution Specification Delta

## MODIFIED Requirements

### Requirement: Generic Recursive Data Runs
Generic structs and enums that reach themselves through `Heap<T>` with the same type arguments MUST run through generic functions and extensions, with every box freed exactly once.

#### Scenario: A generic linked list
- **WHEN** `enum Chain<T> { Cons(T, Heap<Chain<T>>), Nil }`, `fun <T> Chain<T>.size(): Int = when (this) { Chain.Cons(_, val rest) -> 1 + rest.size(), Chain.Nil -> 0 }` and `fun <T> push(list: Chain<T>, x: T): Chain<T> = Chain.Cons(x, Heap(list))` are declared and `fun main() { val empty: Chain<String> = Chain.Nil; val names = push(push(empty, "a"), "b"); val none: Chain<Int> = Chain.Nil; val numbers = push(none, 1); println(names.size()); println(numbers.size()) }` runs under `prism run --check-allocations`
- **THEN** the program prints `2` and `1`, reports no live allocation and exits with status 0

## ADDED Requirements

### Requirement: Lists Hold Their Elements in Order
`listOf(e1, ..., en)` MUST evaluate its arguments left to right and give a list holding them in that order. `xs.size` MUST be the number of elements, `xs.indices` the range `0..<xs.size`, `isEmpty()` whether the size is 0 and `isNotEmpty()` its negation. `xs[i]` MUST read or write the element at index `i`, counting from 0, and an assignment to it MUST drop the old element.

#### Scenario: Reading a list
- **WHEN** `fun main() { val xs = listOf(5, 6, 7); println(xs.size); println(xs[2]); println(xs.isNotEmpty()); for (i in xs.indices) print(i); println() }` runs
- **THEN** it prints `3`, `7`, `true` and `012`

### Requirement: Mutating Members Keep the Other Elements in Order
`add(e)` MUST append `e`; `insert(i, e)` MUST place `e` at index `i`, moving the elements from `i` on one index up; `removeAt(i)` MUST give the element at `i`, moving the later elements one index down; `removeLast()` MUST give the last element; `reverse()` MUST reverse the order of the elements; and `clear()` MUST drop every element, leaving the list empty. Each MUST keep the relative order of the elements it does not move.

#### Scenario: Building and changing a list
- **WHEN** `fun show(xs: &List<Int>) { for (x in xs) print(x); println() }` is declared and `fun main() { var xs = listOf(1, 2, 3); xs.add(4); xs.insert(0, 0); show(xs); println(xs.removeAt(2)); println(xs.removeLast()); xs[0] = 9; xs[1] += 10; show(xs); xs.reverse(); show(xs); println(xs.size); xs.clear(); println(xs.isEmpty()) }` runs with `--check-allocations`
- **THEN** it prints `01234`, `2`, `4`, `9113`, `3119`, `3` and `true`, writes nothing to stderr and exits with status 0

#### Scenario: Lists of strings
- **WHEN** `fun main() { var names = listOf("a" + "da", "bob".clone()); names.add(3.toString()); names[1] = "eve".clone(); val first = names.removeAt(0); println(first); for (n in names) println(n); println(names[0].length) }` runs with `--check-allocations`
- **THEN** it prints `ada`, `eve`, `3` and `3`, writes nothing to stderr and exits with status 0

#### Scenario: Lists in structs and nested lists
- **WHEN** `struct Todo { val title: String, var done: Bool }` and `struct Model { var todos: List<Todo> }` are declared and `fun main() { var m = Model(listOf()); m.todos.add(Todo("write".clone(), false)); m.todos.add(Todo("test".clone(), false)); m.todos[1].done = true; for (t in m.todos) if (t.done) println(t.title); var grid = listOf(listOf(1), listOf<Int>()); grid[1].add(2); grid[0][0] = 3; println(grid[0][0] + grid[1][0]) }` runs with `--check-allocations`
- **THEN** it prints `test` and `5`, writes nothing to stderr and exits with status 0

### Requirement: List Indices Are Checked
An element access, `removeAt(i)` or `insert(i, e)` whose index is outside `0..<size` (for `insert`, outside `0..size`) MUST end the program with the runtime error "list index <i> out of range for size <n>", located at the access or the call, after the operands of the access or the call are evaluated and before the list changes. `removeLast()` on an empty list MUST end it with the runtime error "removeLast() on an empty list".

#### Scenario: An index out of range
- **WHEN** `fun main() { val xs = listOf(1, 2, 3); println(xs[0]); println(xs[3]) }` runs
- **THEN** stdout holds `1` and a newline, stderr holds `panic: list index 3 out of range for size 3` and the location of `xs[3]`, and the exit status is 101

#### Scenario: Negative indices and bad removals
- **WHEN** `fun main() { var xs = listOf(1); xs.removeAt(-1) }`, `fun main() { var xs = listOf(1); xs.insert(2, 5) }` or `fun main() { var xs = listOf<Int>(); xs.removeLast() }` runs
- **THEN** stderr holds `panic: list index -1 out of range for size 1`, `panic: list index 2 out of range for size 1` or `panic: removeLast() on an empty list` respectively, with the call's location, and the exit status is 101

### Requirement: Iterating a List Visits Each Element in Order
A `for` loop over a list MUST run its body once per element, in index order, with the variable referring to that element; `break` MUST leave the loop and `continue` go to the next element. Writes through a variable of a loop over `&mut xs` MUST change the elements of `xs`. A temporary list a loop iterates MUST be dropped after the loop ends.

#### Scenario: Summing, skipping and stopping
- **WHEN** `fun main() { val xs = listOf(1, 2, 3, 4, 5); var sum = 0; for (x in xs) { if (x == 2) continue; if (x == 5) break; sum += x }; println(sum) }` runs
- **THEN** it prints `8`

#### Scenario: Updating elements in a loop
- **WHEN** `fun main() { var xs = listOf(1, 2, 3); for (x in &mut xs) { *x *= 10 }; for (x in xs) print(x); println() }` runs with `--check-allocations`
- **THEN** it prints `102030`, writes nothing to stderr and exits with status 0

#### Scenario: Iterating a temporary list of strings
- **WHEN** `fun make(): List<String> = listOf(1.toString(), 2.toString())` is declared and `fun main() { for (s in make()) println(s) }` runs with `--check-allocations`
- **THEN** it prints `1` and `2`, writes nothing to stderr and exits with status 0

### Requirement: Lists Compare and Search by Element Equality
Two lists MUST be equal when they have the same size and their elements at each index are equal by `==` of the element type, and `!=` MUST be its negation. `indexOf(e)` MUST give the smallest index whose element equals `e`, or -1 when none does, and `contains(e)` whether there is one.

#### Scenario: Comparing and searching
- **WHEN** `struct P : Copy { val x: Int }` is declared and `fun main() { val a = listOf("a".clone(), "b".clone()); val b = listOf("a", "b"); println(a == b); println(a != listOf("a")); println(listOf(P(1), P(2)).indexOf(P(2))); println(a.contains("c")); println(listOf(listOf(1)) == listOf(listOf(1))) }` runs with `--check-allocations`
- **THEN** it prints `true`, `true`, `1`, `false` and `true`, writes nothing to stderr and exits with status 0

### Requirement: Lists Free Their Elements Exactly Once
Dropping a list MUST drop each of its elements once and free its buffer. Cloning a list MUST give a list of clones of its elements that shares no memory with the original, so that changing or dropping either leaves the other unchanged. Elements removed by `removeAt` or `removeLast` MUST be owned by the caller, and elements overwritten or cleared MUST be dropped when they are.

#### Scenario: Cloning a list is deep
- **WHEN** `fun main() { var a = listOf(listOf(1.toString())); val b = a.clone(); a[0][0] = 2.toString(); a[0].add(3.toString()); println(b[0][0]); println(b[0].size); println(a[0].size) }` runs with `--check-allocations`
- **THEN** it prints `1`, `1` and `2`, writes nothing to stderr and exits with status 0

#### Scenario: A tree of nodes with child lists
- **WHEN** `struct Node { val name: String, var children: List<Node> }` and `fun Node.count(): Int { var n = 1; for (c in children) n += c.count(); return n }` are declared and `fun main() { var root = Node("root".clone(), listOf()); root.children.add(Node("a".clone(), listOf(Node("b".clone(), listOf())))); root.children.add(Node("c".clone(), listOf())); println(root.count()) }` runs with `--check-allocations`
- **THEN** it prints `4`, writes nothing to stderr and exits with status 0

#### Scenario: Lists in generic code and top-level values
- **WHEN** `val primes = listOf(2, 3, 5)`, `fun <T> lastOf(xs: &List<T>): &T = &xs[xs.size - 1]` and `fun <T> pushAll(into: &mut List<T>, from: List<T>) { var source = from; source.reverse(); while (source.isNotEmpty()) into.add(source.removeLast()) }` are declared and `fun main() { var names = listOf("a".clone()); pushAll(&mut names, listOf("b".clone(), "c".clone())); println(lastOf(names)); println(lastOf(primes)); println(names.size) }` runs with `--check-allocations`
- **THEN** it prints `c`, `5` and `3`, writes nothing to stderr and exits with status 0

#### Scenario: A list of handlers
- **WHEN** `fun main() { var handlers = listOf<() -> Unit>(); handlers.add { println("one") }; handlers.add { println("two") }; for (h in handlers) h(); handlers.removeAt(0); handlers[0]() }` runs with `--check-allocations`
- **THEN** it prints `one`, `two` and `two`, writes nothing to stderr and exits with status 0

### Requirement: Adding to a List Takes Amortized Constant Time
On a release build, `prism run` of a program that adds a million `Int`s to an empty list and then sums them MUST take under 20 ms of wall time, startup included, on a developer machine. A timing benchmark MUST report this time against the target; it MUST NOT fail a test run on the time alone.

#### Scenario: A million adds
- **WHEN** the ignored benchmark runs `prism run` on its program of a million `add`s on a release build
- **THEN** it reports the median and 90th percentile wall time over repeated runs against the 20 ms target, and fails on the time only when `PRISM_ASSERT_TIMING=1` is set
