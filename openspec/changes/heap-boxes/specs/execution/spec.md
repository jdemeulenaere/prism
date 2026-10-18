# Execution Specification Delta

## ADDED Requirements

### Requirement: Boxes Hold Their Contents
`Heap(v)` MUST give a box holding `v`, and every read through the box (a field, an operator, a copy-out, a borrow of the contents, a pattern) MUST read the value it holds. Each box MUST be freed exactly once: by its drop, by `unbox()`, or when it is taken apart.

#### Scenario: Evaluating an expression tree
- **WHEN** `enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>), Mul(Heap<Expr>, Heap<Expr>) }` and `fun eval(e: &Expr): Int = when (e) { Expr.Lit(val n) -> n; Expr.Add(val l, val r) -> eval(l) + eval(r); Expr.Mul(val l, val r) -> eval(l) * eval(r) }` are declared and `fun main() { val e = Expr.Add(Heap(Expr.Lit(2)), Heap(Expr.Mul(Heap(Expr.Lit(3)), Heap(Expr.Lit(4))))); println(eval(e)) }` runs with `--check-allocations`
- **THEN** it prints `14`, writes nothing to stderr and exits with status 0

#### Scenario: Summing a linked list
- **WHEN** `struct Node { val value: Int, val next: Heap<Node>? }` and `fun Node.sum(): Int = value + (next?.sum() ?: 0)` are declared and `fun main() { var list: Heap<Node>? = null; for (i in 1..4) { list = Heap(Node(i, list)) }; println(list?.sum() ?: 0) }` runs with `--check-allocations`
- **THEN** it prints `10`, writes nothing to stderr and exits with status 0

#### Scenario: Measuring a tree
- **WHEN** `struct Tree { val value: Int, val left: Heap<Tree>?, val right: Heap<Tree>? }`, `fun leaf(v: Int): Heap<Tree> = Heap(Tree(v, null, null))`, `fun Tree.depth(): Int { val l = left?.depth() ?: 0; val r = right?.depth() ?: 0; return 1 + (if (l > r) l else r) }` and `fun valueOf(t: (&Tree)?): Int = t?.value ?: 0` are declared and `fun main() { val t = Tree(1, Heap(Tree(2, leaf(3), null)), leaf(4)); println(t.depth()); println(valueOf(t.left)); println(valueOf(t.right?.left)) }` runs with `--check-allocations`
- **THEN** it prints `3`, `2` and `0`, writes nothing to stderr and exits with status 0

#### Scenario: Reading boxed scalars
- **WHEN** `fun show(n: &Int): Int = n * 10` is declared and `fun main() { val h = Heap(4); val n: Int = h; println(h + n); println(show(h)); println(h.toString() + "!"); println(h == Heap(4)) }` runs with `--check-allocations`
- **THEN** it prints `8`, `40`, `4!` and `true`, writes nothing to stderr and exits with status 0

### Requirement: Writing a Box's Contents Keeps the Box
`*h = v` MUST drop the box's old contents and move `v` into the same allocation, so that every reader of the box sees `v`, and MUST neither allocate nor free the box. Assigning a new box to a box place MUST drop the old box.

#### Scenario: Writing boxed strings
- **WHEN** `fun set(r: &mut Heap<String>, v: String) { **r = v }` is declared and `fun main() { var h = Heap(1.toString()); *h = 2.toString(); set(&mut h, 3.toString()); println(h); h = Heap(4.toString()); println(h) }` runs with `--check-allocations`
- **THEN** it prints `3` and then `4`, writes nothing to stderr and exits with status 0

### Requirement: Unboxing Moves the Contents Out
`h.unbox()` MUST give the value the box holds and free the box without dropping that value, and a `consuming fun` called through boxes MUST receive the contents, each box freed. `h?.unbox()` MUST give `null` for a `null` box.

#### Scenario: Unboxing and consuming through a box
- **WHEN** `struct User { val name: String }` and `consuming fun User.take(): String { val (name) = this; return name }` are declared and `fun main() { val u = Heap(User(1.toString())).unbox(); println(u.name); val h = Heap(Heap(User(2.toString()))); println(h.take()); val n: Heap<User>? = null; println(n?.unbox() == null) }` runs with `--check-allocations`
- **THEN** it prints `1`, `2` and `true`, writes nothing to stderr and exits with status 0

### Requirement: Dropping and Cloning Boxes
Dropping a box MUST drop its contents and then free it. Cloning a box MUST give a new box holding a clone of the contents, sharing no memory with the original, so that writing or dropping either leaves the other unchanged.

#### Scenario: Dropping a recursive value
- **WHEN** `enum Doc { Text(String), Pair(Heap<Doc>, Heap<Doc>) }` is declared and `fun main() { var i = 0; while (i < 100) { val d = Doc.Pair(Heap(Doc.Text(i.toString())), Heap(Doc.Pair(Heap(Doc.Text("x")), Heap(Doc.Text(i.toString()))))); i += 1 }; println(i) }` runs with `--check-allocations`
- **THEN** it prints `100`, writes nothing to stderr and exits with status 0

#### Scenario: A cloned box is independent
- **WHEN** `struct Node : Clone { var value: Int, val next: Heap<Node>? }` is declared and `fun main() { var a = Heap(Node(1, Heap(Node(2, null)))); val b = a.clone(); a.value = 5; println(a.value); println(b.value); println(b.next?.value ?: 0) }` runs with `--check-allocations`
- **THEN** it prints `5`, `1` and `2`, writes nothing to stderr and exits with status 0

#### Scenario: A box of a zero-sized value
- **WHEN** `struct Marker : Copy {}` is declared and `fun main() { val h = Heap(Marker()); val g = h.clone(); val m = g.unbox(); println(h == Heap(m)) }` runs with `--check-allocations`
- **THEN** it prints `true`, writes nothing to stderr and exits with status 0

### Requirement: Boxes Compare Their Contents
`a == b` on boxes MUST compare the values they hold structurally, through any depth of boxes, never their addresses, and `!=` MUST be its negation.

#### Scenario: Comparing recursive values
- **WHEN** `enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun sum(a: Int, b: Int): Expr = Expr.Add(Heap(Expr.Lit(a)), Heap(Expr.Lit(b)))` are declared and `fun main() { println(sum(1, 2) == sum(1, 2)); println(sum(1, 2) == sum(2, 1)); println(sum(1, 2) != Expr.Lit(3)) }` runs with `--check-allocations`
- **THEN** it prints `true`, `false` and `true`, writes nothing to stderr and exits with status 0

#### Scenario: A box holding NaN
- **WHEN** `fun main() { val h = Heap(0.0 / 0.0); println(h == h) }` runs
- **THEN** it prints `false`

### Requirement: Taking Apart Through a Box Frees the Box
When a pattern takes a value apart through a box, the box MUST be freed and the contents' parts bound or dropped as for any value taken apart, each exactly once.

#### Scenario: Consuming a linked list
- **WHEN** `struct Node { val value: Int, val next: Heap<Node>? }` and `fun total(list: Heap<Node>?): Int = when (list) { null -> 0; Node(val v, val rest) -> v + total(rest) }` are declared and `fun main() { println(total(Heap(Node(1, Heap(Node(2, Heap(Node(3, null)))))))) }` runs with `--check-allocations`
- **THEN** it prints `6`, writes nothing to stderr and exits with status 0

#### Scenario: Simplifying an expression tree
- **WHEN** `enum Expr { Lit(Int), Name(String), Add(Heap<Expr>, Heap<Expr>) }` and `fun simplify(e: Expr): Expr = when (e) { Expr.Add(Expr.Lit(0), val r) -> r.unbox(); Expr.Add(Expr.Name(val n), Expr.Lit(0)) -> Expr.Name(n); else -> e }` are declared and `fun main() { val a = simplify(Expr.Add(Heap(Expr.Lit(0)), Heap(Expr.Lit(7)))); val b = simplify(Expr.Add(Heap(Expr.Name(1.toString())), Heap(Expr.Lit(0)))); println(a == Expr.Lit(7)); println(b == Expr.Name("1")) }` runs with `--check-allocations`
- **THEN** it prints `true` twice, writes nothing to stderr and exits with status 0

## MODIFIED Requirements

### Requirement: Stack Overflow Is a Runtime Error
A program whose calls nest deeper than its stack allows MUST raise the runtime error "stack overflow" at the declaration of the function being entered, or with no location when the stack runs out inside drop, clone or equality glue, which ends the program as any runtime error does. A program MUST get a stack of the same size on every platform.

#### Scenario: Unbounded recursion
- **WHEN** `fun deep(n: Int): Int = 1 + deep(n + 1)` is declared and `fun main() { println(1); println(deep(0)) }` runs
- **THEN** stdout holds `1` and a newline, stderr holds `panic: stack overflow` and the location of the declaration of `deep`, and the exit status is 101

#### Scenario: Dropping a structure deeper than the stack
- **WHEN** `struct Node { val next: Heap<Node>? }` is declared and `fun main() { var list: Heap<Node>? = null; var i = 0; while (i < 1000000) { list = Heap(Node(list)); i += 1 }; println(i) }` runs
- **THEN** stdout holds `1000000` and a newline, stderr holds exactly `panic: stack overflow` and a newline, and the exit status is 101

### Requirement: Runtime Errors End the Program
A runtime error MUST flush the program's output, write `panic: <message>` and then `  at <path>:<line>:<column>` to stderr, where the path is the file's path relative to the workspace root and the position is that of the failing operation, and exit with status 101. "out of memory", and "stack overflow" raised inside glue, MUST write no location line. No further code of the program MUST run, and no drop MUST run.

#### Scenario: Runtime error location
- **WHEN** `app/main.prism` declares `fun zero(): Int = 0` on line 2 and `fun main() { val a = 1; println(a / zero()) }` on line 3, with `a / zero()` starting at column 33
- **THEN** stderr is exactly `panic: division by zero` and `  at app/main.prism:3:33`, each followed by a newline
