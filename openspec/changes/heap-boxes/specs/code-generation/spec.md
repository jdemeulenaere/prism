# Code Generation Specification Delta

## ADDED Requirements

### Requirement: A Box Is a Pointer to an Exactly Sized Allocation
A `Heap<T>` value MUST be a non-null pointer to its contents, held in an allocation of exactly the size and alignment of `T` made through the runtime. Constructing a box MUST evaluate its argument before allocating. A box of a zero-sized `T` MUST allocate nothing and hold a non-null address aligned for `T`. A `Heap<T>?` MUST stay one pointer, `null` being 0.

#### Scenario: Constructing a box allocates its contents' size
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `fun f(p: Point): Heap<Point> = Heap(p)` are declared and `prism dump-clif main.prism` runs
- **THEN** the body of `f` calls the runtime's allocation function with size 16 and alignment 8 and writes `p` into the allocation

#### Scenario: A box of a zero-sized value allocates nothing
- **WHEN** `struct Marker : Copy {}` is declared and `fun main() { val h = Heap(Marker()); val g = h.clone(); println(1) }` runs
- **THEN** it prints `1`, and the program makes no allocation

### Requirement: Box Glue Drops the Contents Before Freeing
Dropping a `Heap<T>` MUST run the drop glue of `T` on its contents when `T` owns storage, and then free the allocation, which MUST be freed exactly once. Unboxing MUST move the contents out and free the allocation without dropping them. Each box type that a unit drops MUST have its own drop glue, named after the box type.

#### Scenario: Box glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun f(e: Expr) {}`
- **THEN** after the file's functions it prints `function main#drop(main::Expr)` and `function main#drop(Heap<main::Expr>)`, each calling the other, and the latter freeing the allocation after the call

### Requirement: Cloning a Box Allocates a New Box
The clone glue of `Heap<T>` MUST allocate a new box and clone the contents into it, by the clone glue of `T` when `T` owns storage and by a bitwise copy otherwise, so that the clone shares no memory with the original.

#### Scenario: Box clone glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `fun f(h: &Heap<Int>): Heap<Int> = h.clone()`
- **THEN** it prints `function main#clone(Heap<Int>)`, which `f` calls, and which calls the runtime's allocation function

### Requirement: Boxes of Aggregates Are Compared by Equality Glue
Comparing the contents of two boxes whose contents are a struct, an enum or a nullable of one MUST call the equality glue of the contents' type, a function of its code unit named `<module>#equals(<type>)` that compares two values structurally as `==` does, without comparing addresses. Contents of other types MUST be compared inline.

#### Scenario: Equality glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `enum Expr { Lit(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun same(a: &Expr, b: &Expr): Bool = a == b`
- **THEN** it prints `function main#equals(main::Expr)`, which calls itself for the boxed payloads of `Add`, and `same` compares the variants inline and calls the glue for `Add`'s payloads

### Requirement: Glue Checks the Stack
Every drop, clone and equality glue function MUST check the stack on entry, as compiled bodies do, so that recursion through glue that exceeds the program's stack raises the runtime error "stack overflow".

#### Scenario: Dropping a very deep list
- **WHEN** `struct Node { val next: Heap<Node>? }` is declared and `fun main() { var list: Heap<Node>? = null; var i = 0; while (i < 1000000) { list = Heap(Node(list)); i += 1 }; println(i) }` runs
- **THEN** stdout holds `1000000` and a newline, stderr holds exactly `panic: stack overflow` and a newline, and the exit status is 101

### Requirement: Constructs the Backend Lacks Are Rejected Before Code Generation
Before generating any code, the backend MUST check every body it compiles and report `E6001` (`UnsupportedByBackend`) for the first construct of each body that it does not support, naming the construct. An unsupported parameter or return type MUST be reported at the declaration, anything else at the first statement or terminator using it, in block order. A rejected program MUST NOT run. `prism check` MUST NOT report `E6001`.

#### Scenario: A struct is rejected
- **WHEN** `struct P { val run: () -> Int }`, `fun one(): Int = 1` and `fun main() { val p = P(one); println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` in `main`, naming function values and closures, with a note naming the `closures` change, prints nothing to stdout and exits with status 1

#### Scenario: A recursive enum runs
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun main() { val e = Expr.Add(Heap(Expr.Literal(1)), Heap(Expr.Literal(2))); println(1) }` are declared
- **THEN** `prism run --check-allocations main.prism` prints `1`, writes nothing to stderr and exits with status 0

#### Scenario: Unsupported constructs in several forms
- **WHEN** a program's `main` runs `val f = { 1 }`, or `println(id(1))` for `fun <T> id(x: T): T = x`, or the program declares `compose fun Label() {}` or `fun first(g: () -> Int): Int = 1`
- **THEN** `prism run` reports `E6001` naming closures at `{ 1 }`, calls of generic functions at `id(1)`, composable functions at `Label`, and function values at the declaration of `first`, and runs nothing

#### Scenario: One error per body
- **WHEN** `fun f(): Int { val a = { 1 }; val b = { 2 }; return 1 }` and `fun g(): Int { val c = { 3 }; return 2 }` are declared with a `main` that calls neither
- **THEN** `prism run` reports exactly two `E6001`, one at the first closure in `f` and one at the closure in `g`

## MODIFIED Requirements

### Requirement: Symbols Derive from Declaration Paths
Every compiled function of a declaration MUST have a symbol derived from its declaration's identity alone: `<module>::<name>` for a function, `<module>::<receiver>.<name>` for an extension, where the receiver is written as the ABI writes its type constructor, `<function symbol>#default(<parameter>)` for a default value's generator, and `<module>::<name>#init` and `<module>::<name>#get` for a top-level `val`'s initializer and accessor. Symbols MUST be unique in a program. Drop, clone and equality glue MUST have no slot in the function table: each is local to the code unit that generates it and named `<module>#drop(<type>)`, `<module>#clone(<type>)` or `<module>#equals(<type>)` after that unit's module and the type it handles.

#### Scenario: Symbols of a file
- **WHEN** `lib/math.prism`, in package `lib`, declares `public fun Int.double(): Int = this * 2`, `public fun scale(a: Int, b: Int = 2): Int = a * b` and `public val BASE: Int = 3`
- **THEN** `prism dump-clif lib/math.prism` prints the functions `lib/math::Int.double`, `lib/math::scale`, `lib/math::scale#default(b)`, `lib/math::BASE#init` and `lib/math::BASE#get`

#### Scenario: Glue of one type in two files
- **WHEN** `lib/user.prism` declares `public struct User { val name: String }` and `public fun drop(u: User) {}`, and `app/main.prism` imports `/lib/user` and declares `fun keep(u: User) {}` and `fun main() { drop(User("a")); keep(User("b")) }`
- **THEN** `prism run --check-allocations app/main.prism` exits with status 0, and `prism dump-clif` of each file prints its own drop glue of `lib/user::User`, `lib/user#drop(lib/user::User)` and `app/main#drop(lib/user::User)`

### Requirement: Glue Is Local to Its Code Unit
Each code unit MUST generate the drop, clone and equality glue of the types its bodies drop, clone or compare through boxes, including types of other modules and the types those reach through boxes, and call it directly, not through the function table, so that a unit's code depends only on its own MIR.

#### Scenario: Drop glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct User { val name: String }` and `fun f(u: User) {}`
- **THEN** after the file's functions it prints the drop glue of `main::User`, headed `function main#drop(main::User)`, and `f` calls it on its argument directly

#### Scenario: Clone glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct User : Clone { val name: String, val id: Int }` and `fun f(u: &User): User = u.clone()`
- **THEN** after the file's functions it prints the clone glue of `main::User`, headed `function main#clone(main::User)`, which `f` calls

#### Scenario: Glue of an imported recursive type
- **WHEN** `lib/tree.prism` declares `public enum Tree { Leaf, Node(Heap<Tree>, Int, Heap<Tree>) }` and `public fun leaf(): Tree = Tree.Leaf`, and `app/main.prism` imports `/lib/tree` and declares `fun main() { val t = Tree.Node(Heap(leaf()), 1, Heap(leaf())); println(t == t) }`
- **THEN** `prism run --check-allocations app/main.prism` prints `true` and exits with status 0, and `prism dump-clif app/main.prism` prints `app/main#drop(lib/tree::Tree)`, `app/main#drop(Heap<lib/tree::Tree>)` and `app/main#equals(lib/tree::Tree)`

### Requirement: Drops of Values That Own Nothing Do Nothing
A value whose type owns no storage (no `String`, `Heap<T>` or function value in it, through any field, payload or nullable) MUST be dropped without running any code. A struct or enum holding a function value MUST be rejected with `E6001` naming what it holds, until `closures` supports them.

#### Scenario: A non-Copy struct is moved and dropped
- **WHEN** `struct Counter { var n: Int }`, `fun take(c: Counter): Int = c.n` and `fun flag(): Bool = false` are declared and `fun main() { val c = Counter(4); if (flag()) println(take(c)); println(1) }` runs
- **THEN** the program prints `1` and exits with status 0

#### Scenario: A struct holding a string
- **WHEN** `struct User { val name: String }`, `fun size(u: &User): Int = 1` and `fun main() { println(size(User("a"))) }` are declared
- **THEN** `prism run --check-allocations main.prism` prints `1` and exits with status 0

#### Scenario: A struct holding a heap value
- **WHEN** `struct Node { val next: Heap<Int>? }`, `fun size(n: &Node): Int = 1` and `fun main() { println(size(Node(Heap(1)))) }` are declared
- **THEN** `prism run --check-allocations main.prism` prints `1` and exits with status 0

#### Scenario: A struct holding a function value
- **WHEN** `struct Task { val run: () -> Int }`, `fun size(t: &Task): Int = 1` and `fun main() { println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at the declaration of `size`, naming function values and closures, and exits with status 1

### Requirement: Cranelift IR Dump
`prism dump-clif <file.prism>` MUST print, as deterministic text, the Cranelift IR of each function the backend generates for the file, in body order, each headed by its symbol, followed by the file's drop, clone and equality glue, each headed by `<module>#drop(<type>)`, `<module>#clone(<type>)` or `<module>#equals(<type>)`, in order of their names. Like `prism dump-mir --elaborated`, it MUST compile the file alone, against its direct imports' ABIs. When the file has errors of any stage, including `E6001`, it MUST report them, print no IR, and exit with a failure status.

#### Scenario: Dumping a file's IR
- **WHEN** `prism dump-clif main.prism` runs twice on a file that `prism run` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: Dumping a file the backend rejects
- **WHEN** `prism dump-clif main.prism` runs on a file holding `fun f(g: () -> Int): Int = 1`
- **THEN** it reports `E6001`, prints no IR, and exits with a failure status

## REMOVED Requirements

### Requirement: Unsupported Constructs Are Rejected Before Code Generation
**Reason**: Its scenarios pin `Heap<T>` values as unsupported ("An enum holding a heap value is rejected"), which this change makes run, and a MODIFIED block cannot drop a scenario.
**Migration**: Replaced, with the same rule, by "Constructs the Backend Lacks Are Rejected Before Code Generation", whose scenarios use function values where these used `Heap<T>`, and by "A recursive enum runs".
