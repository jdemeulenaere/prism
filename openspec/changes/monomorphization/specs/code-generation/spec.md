# Code Generation Specification Delta

## ADDED Requirements

### Requirement: Instances Are Code Units of Their Own
Each instance of a generic body that a program needs MUST be compiled exactly once per program, into a code unit of its own, built from the instance's MIR alone, and MUST have its own slot in the function table. Every call of an instance, from any file or from another instance, MUST go through that slot, so that no other unit's code depends on a generic body.

#### Scenario: Calls of an instance go through its slot
- **WHEN** `lib/box.prism` declares `public fun <T> keep(x: T): T = x`, and `app/main.prism` imports `/lib/box` and declares `fun main() { println(keep(1)) }`
- **THEN** `prism dump-clif app/main.prism` shows the call in `main` as a load from the slot `lib/box::keep<Int>#slot` followed by an indirect call, and `prism run app/main.prism` prints `1`

#### Scenario: One instance for every caller
- **WHEN** in that workspace `app/other.prism` imports `/lib/box` and declares `public fun other(): Int = keep(2)`, and `app/main.prism` also imports `/app/other` and prints `other()`
- **THEN** `prism dump-clif --instances app/main.prism` prints the function `lib/box::keep<Int>` once, and `prism run app/main.prism` prints `1` and `2`

### Requirement: Instance Code Units Are Dumped on Request
`prism dump-clif --instances <file.prism>` MUST compile the program whose entry is the file and print, as deterministic text, the Cranelift IR of every instance code unit it needs, in order of the instances' symbols, each function headed by its symbol and followed by the unit's glue. When any file of the program has errors of any stage, it MUST report them, print no IR and exit with a failure status.

#### Scenario: Dumping a program's instances
- **WHEN** `prism dump-clif --instances main.prism` runs twice on a program that `prism run` accepts and that calls `fun <T> keep(x: T): T = x` as `keep(1)` and `keep("a")`
- **THEN** it prints the functions `main::keep<Int>` and `main::keep<String>` both times, in that order, and exits with status 0

## MODIFIED Requirements

### Requirement: Tier 1 Compiles Elaborated MIR in Memory
`prism run` MUST compile the elaborated MIR of every file of the program into native code in the memory of the `prism` process, without invoking any external program such as a linker or an assembler. Every body MUST be compiled except generic bodies, which are compiled only as the instances the program calls, and closure bodies, which wait for closure conversion.

#### Scenario: Running a scalar program
- **WHEN** `fun square(x: Int): Int = x * x` and `fun main() { println(square(7)) }` are declared in `main.prism` and `prism run main.prism` runs
- **THEN** it prints `49` and exits with status 0, and no other process is started

#### Scenario: An uncalled generic function is not compiled
- **WHEN** `fun <T> keep(x: T): T = x` and `fun main() { println(1) }` are declared and `prism run main.prism` runs
- **THEN** it prints `1` and exits with status 0

#### Scenario: A called generic function is compiled per instance
- **WHEN** `fun <T> keep(x: T): T = x` and `fun main() { println(keep(1)); println(keep(true)) }` are declared and `prism run main.prism` runs
- **THEN** it prints `1` and `true` and exits with status 0

### Requirement: Symbols Derive from Declaration Paths
Every compiled function of a declaration MUST have a symbol derived from its declaration's identity alone: `<module>::<name>` for a function, `<module>::<receiver>.<name>` for an extension, where the receiver is written as the ABI writes its type constructor, `<function symbol>#default(<parameter>)` for a default value's generator, and `<module>::<name>#init` and `<module>::<name>#get` for a top-level `val`'s initializer and accessor. An instance's symbol MUST be its generic function's or extension's symbol followed by its type arguments in angle brackets, separated by `, ` and written with every struct and enum qualified by its module, and an instance of a default value's generator MUST be `<instance symbol>#default(<parameter>)`. Symbols MUST be unique in a program. Drop, clone and equality glue MUST have no slot in the function table: each is local to the code unit that generates it and named `<unit>#drop(<type>)`, `<unit>#clone(<type>)` or `<unit>#equals(<type>)` after the type it handles and that unit's name, which is its module for a file's unit and its instance's symbol for an instance's unit.

#### Scenario: Symbols of a file
- **WHEN** `lib/math.prism`, in package `lib`, declares `public fun Int.double(): Int = this * 2`, `public fun scale(a: Int, b: Int = 2): Int = a * b` and `public val BASE: Int = 3`
- **THEN** `prism dump-clif lib/math.prism` prints the functions `lib/math::Int.double`, `lib/math::scale`, `lib/math::scale#default(b)`, `lib/math::BASE#init` and `lib/math::BASE#get`

#### Scenario: Glue of one type in two files
- **WHEN** `lib/user.prism` declares `public struct User { val name: String }` and `public fun drop(u: User) {}`, and `app/main.prism` imports `/lib/user` and declares `fun keep(u: User) {}` and `fun main() { drop(User("a")); keep(User("b")) }`
- **THEN** `prism run --check-allocations app/main.prism` exits with status 0, and `prism dump-clif` of each file prints its own drop glue of `lib/user::User`, `lib/user#drop(lib/user::User)` and `app/main#drop(lib/user::User)`

#### Scenario: Symbols of instances
- **WHEN** `lib/box.prism` declares `public struct Box<T>(val v: T)`, `public fun <T> keep(x: T): T = x`, `public fun <T> Box<T>.get(): &T = &v` and `public fun <T : Copy> pad(x: T, y: T = x): T = y`, `lib/user.prism` declares `public struct User { val name: String }`, and `app/main.prism` imports both and declares `fun main() { val u = keep(User("a")); println(Box(1).get()); println(pad(2)) }`
- **THEN** `prism dump-clif --instances app/main.prism` prints the functions `lib/box::keep<lib/user::User>`, `lib/box::lib/box.Box.get<Int>`, `lib/box::pad<Int>` and `lib/box::pad<Int>#default(y)`, and `prism run --check-allocations app/main.prism` prints `1` and `2`

### Requirement: Glue Is Local to Its Code Unit
Each code unit, a file's or an instance's, MUST generate the drop, clone and equality glue of the types its bodies drop, clone or compare through boxes, including types of other modules and the types those reach through boxes, and call it directly, not through the function table, so that a unit's code depends only on its own MIR.

#### Scenario: Drop glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct User { val name: String }` and `fun f(u: User) {}`
- **THEN** after the file's functions it prints the drop glue of `main::User`, headed `function main#drop(main::User)`, and `f` calls it on its argument directly

#### Scenario: Clone glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct User : Clone { val name: String, val id: Int }` and `fun f(u: &User): User = u.clone()`
- **THEN** after the file's functions it prints the clone glue of `main::User`, headed `function main#clone(main::User)`, which `f` calls

#### Scenario: Glue of an imported recursive type
- **WHEN** `lib/tree.prism` declares `public enum Tree { Leaf, Node(Heap<Tree>, Int, Heap<Tree>) }` and `public fun leaf(): Tree = Tree.Leaf`, and `app/main.prism` imports `/lib/tree` and declares `fun main() { val t = Tree.Node(Heap(leaf()), 1, Heap(leaf())); println(t == t) }`
- **THEN** `prism run --check-allocations app/main.prism` prints `true` and exits with status 0, and `prism dump-clif app/main.prism` prints `app/main#drop(lib/tree::Tree)`, `app/main#drop(Heap<lib/tree::Tree>)` and `app/main#equals(lib/tree::Tree)`

#### Scenario: Glue of an instance
- **WHEN** `struct User : Clone { val name: String }`, `fun <T> discard(x: T) {}` and `fun <T : Clone> copy(x: &T): T = x.clone()` are declared and `fun main() { discard(User("a")); val u = copy(&User("b")); println(1) }` runs
- **THEN** `prism run --check-allocations main.prism` prints `1` and exits with status 0, and `prism dump-clif --instances main.prism` prints `main::discard<main::User>#drop(main::User)` after `main::discard<main::User>`, and `main::copy<main::User>#clone(main::User)` after `main::copy<main::User>`

### Requirement: Constructs the Backend Lacks Are Rejected Before Code Generation
Before generating any code, the backend MUST check every body it compiles, each instance included, and report `E6001` (`UnsupportedByBackend`) for the first construct of each body that it does not support, naming the construct. An unsupported parameter or return type MUST be reported at the declaration, anything else at the first statement or terminator using it, in block order; for an instance, at that location in the generic's source, once however many instances share it. A rejected program MUST NOT run. `prism check` MUST NOT report `E6001`.

#### Scenario: A struct is rejected
- **WHEN** `struct P { val run: () -> Int }`, `fun one(): Int = 1` and `fun main() { val p = P(one); println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` in `main`, naming function values and closures, with a note naming the `closures` change, prints nothing to stdout and exits with status 1

#### Scenario: A recursive enum runs
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun main() { val e = Expr.Add(Heap(Expr.Literal(1)), Heap(Expr.Literal(2))); println(1) }` are declared
- **THEN** `prism run --check-allocations main.prism` prints `1`, writes nothing to stderr and exits with status 0

#### Scenario: Unsupported constructs in several forms
- **WHEN** a program's `main` runs `val f = { 1 }`, or the program declares `compose fun Label() {}` or `fun first(g: () -> Int): Int = 1`
- **THEN** `prism run` reports `E6001` naming closures at `{ 1 }`, composable functions at `Label`, and function values at the declaration of `first`, and runs nothing

#### Scenario: One error per body
- **WHEN** `fun f(): Int { val a = { 1 }; val b = { 2 }; return 1 }` and `fun g(): Int { val c = { 3 }; return 2 }` are declared with a `main` that calls neither
- **THEN** `prism run` reports exactly two `E6001`, one at the first closure in `f` and one at the closure in `g`

#### Scenario: An unsupported construct in a generic body
- **WHEN** `lib/run.prism` declares `public fun <T> later(x: T): Int { val f = { 1 }; return 1 }`, and `app/main.prism` imports `/lib/run` and declares `fun main() { println(later(1)); println(later(true)) }`
- **THEN** `prism check app/main.prism` passes, and `prism run app/main.prism` reports exactly one `E6001`, at `{ 1 }` in `lib/run.prism`, naming closures, and runs nothing

### Requirement: Cranelift IR Dump
`prism dump-clif <file.prism>` MUST print, as deterministic text, the Cranelift IR of each function the backend generates for the file, in body order, each headed by its symbol, followed by the file's drop, clone and equality glue, each headed by `<module>#drop(<type>)`, `<module>#clone(<type>)` or `<module>#equals(<type>)`, in order of their names. Like `prism dump-mir --elaborated`, it MUST compile the file alone, against its direct imports' ABIs, so it prints no instance: a call of an instance appears as a load from its slot. When the file has errors of any stage, including `E6001`, it MUST report them, print no IR, and exit with a failure status.

#### Scenario: Dumping a file's IR
- **WHEN** `prism dump-clif main.prism` runs twice on a file that `prism run` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: Dumping a file the backend rejects
- **WHEN** `prism dump-clif main.prism` runs on a file holding `fun f(g: () -> Int): Int = 1`
- **THEN** it reports `E6001`, prints no IR, and exits with a failure status
