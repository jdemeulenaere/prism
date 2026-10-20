## ADDED Requirements

### Requirement: Lists Answer Membership Tests
`x in xs` and `x !in xs` on a `List<T>`, read through any `&`, `&mut` and `Heap<T>` layers, MUST call its `contains` member, declared `intrinsic operator` by `/prism/collections`, borrowing `x`; they MUST be accepted exactly when `contains` is (`==` on `T`).

#### Scenario: Membership in a list
- **WHEN** `fun f(names: &List<String>, n: String): Bool = n in names && "x" !in names` is checked
- **THEN** the file typechecks, each test calling the `contains` member with `n` and `"x"` borrowed, and `n` is not moved

#### Scenario: Membership in a list of a type parameter rejected
- **WHEN** `fun <T> has(xs: &List<T>, x: &T): Bool = x in xs` is checked
- **THEN** the type checker emits `E4002`, as for `xs.contains(x)`

## MODIFIED Requirements

### Requirement: Elements Are Places
`xs[i]` on a `List<T>`, read through any `&`, `&mut` and `Heap<T>` layers, MUST take an `Int` index (`E4002` otherwise) and name element `i` of the list, a place of type `T`. A `Copy` element MUST be copied out where a value is read, and a borrow of `xs[i]`, written or implicit at an argument or receiver, MUST borrow the element where it lives. Indexing any other type MUST call its `get` operator function (`operators`), and one without it MUST be rejected with `E4002`.

#### Scenario: Reading, borrowing and writing elements
- **WHEN** `struct Todo { val title: String, var done: Bool }` and `fun show(t: &Todo) {}` are declared and `fun f(n: Int) { var xs = listOf(1, 2); xs[0] = xs[1] + n; xs[1] *= 2; var todos = listOf(Todo("a", false)); todos[0].done = true; show(todos[0]); val t: &Todo = &todos[0]; val title = todos[0].title.clone() }` is checked
- **THEN** the file typechecks successfully

#### Scenario: Nested lists
- **WHEN** `fun f(): Int { var grid = listOf(listOf(1, 2), listOf(3)); grid[1].add(4); grid[0][1] = 5; return grid[1][1] + grid.size }` is checked
- **THEN** the file typechecks successfully

#### Scenario: A non-integer index rejected
- **WHEN** `fun f(xs: List<Int>): Int = xs["a"]` is checked
- **THEN** the type checker emits `E4002`, expecting `Int`

### Requirement: For Loops Over Lists
`for (x in iterable)` MUST accept an iterable of type `List<T>`, `&List<T>` or `&mut List<T>`, behind any further `&`, `&mut` and `Heap<T>` layers, as well as a `Range`. The list MUST be evaluated once and borrowed for the whole loop, mutably when the iterable has a `&mut` type (`&mut xs`) and shared otherwise. A place MUST be borrowed where it lives, and any other iterable evaluated into a temporary that lives until the loop ends. Other types follow `loops`.

#### Scenario: Iterating a list of scalars
- **WHEN** `fun f(xs: List<Int>): Int { var total = 0; for (n in xs) { total += n }; return total }` is checked
- **THEN** the file typechecks successfully, `n` has type `&Int`, and `xs` is borrowed, not moved, by the loop

#### Scenario: Iterating a temporary list
- **WHEN** `fun make(): List<String> = listOf("a")` is declared and `fun f() { for (s in make()) println(s) }` is checked
- **THEN** the file typechecks successfully, and the temporary list lives until the loop ends
