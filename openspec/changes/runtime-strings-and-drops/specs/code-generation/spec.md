# Code Generation Specification Delta

## ADDED Requirements

### Requirement: Strings Are a Pointer, Two Lengths and a Capacity
A `String` value MUST be 32 bytes with the alignment of a pointer: a pointer to its UTF-8 text at offset 0, the length of the text in bytes at 8, its length in characters (Unicode scalar values) at 16 and the capacity of its buffer in bytes at 24. The pointer MUST never be null. Structs, enums and nullables MUST lay out a `String` like any field of that size and alignment.

#### Scenario: A struct holding a string
- **WHEN** `struct User { val name: String, val age: Int }` is declared and used in `fun main() { val u = User("Ada", 36); println(u.age) }`
- **THEN** `prism dump-layout main.prism` shows `User` with size 40 and alignment 8, `name` at offset 0 and `age` at 32, and `prism run main.prism` prints `36`

#### Scenario: A nullable string
- **WHEN** a body holds a value of type `String?`
- **THEN** `prism dump-layout` shows `String?` with size 40, its tag at offset 0 and its payload at 8

### Requirement: A String Owns a Buffer Exactly When Its Capacity Is Positive
A `String` whose capacity is 0 MUST own no memory: its text is static data or empty. A `String` whose capacity is positive MUST alone own a buffer of that many bytes, allocated through the runtime, that holds its text.

#### Scenario: An empty concatenation owns nothing
- **WHEN** `fun main() { val s = "" + ""; println(s.length) }` runs
- **THEN** it prints `0` and makes no allocation

### Requirement: String Literals Are Static Data
The bytes of a string literal MUST be stored in its file's code unit, and evaluating the literal MUST build a value that points at them and owns no buffer, without allocating. Dropping such a value MUST free nothing, and cloning it MUST copy the value without allocating.

#### Scenario: Literals do not allocate
- **WHEN** `fun main() { val s = "hi"; val t = s.clone(); println(s); println(t) }` runs
- **THEN** it prints `hi` twice and makes no allocation

### Requirement: The Runtime Allocates and Frees
Every allocation and every free of a running program MUST go through the runtime's allocation functions, which take the size and the alignment of the memory, both when allocating and when freeing. Generated code MUST NOT call an allocator of the platform directly. String operations other than building a literal and reading `length` MUST be calls of runtime functions.

#### Scenario: Concatenation calls the runtime
- **WHEN** `fun join(a: &String, b: &String): String = a + b` is declared and `prism dump-clif main.prism` runs
- **THEN** the body of `join` builds its result with a call of a runtime function and calls neither `malloc` nor `free`

### Requirement: Drops Run Drop Glue
Each drop that the elaborated MIR keeps, guarded by a drop flag or not, MUST run the drop glue of the dropped place's type on that place. A type that owns no storage (no `String`, `Heap<T>` or function value in it, through any field, payload or nullable) MUST have no drop glue: its drops MUST run no code.

#### Scenario: A non-Copy struct is moved and dropped
- **WHEN** `struct Counter { var n: Int }`, `fun take(c: Counter): Int = c.n` and `fun flag(): Bool = false` are declared and `fun main() { val c = Counter(4); if (flag()) println(take(c)); println(1) }` runs
- **THEN** the program prints `1` and exits with status 0

#### Scenario: Dropping a struct frees its strings
- **WHEN** `struct User { val name: String, val nick: String?, val age: Int }` is declared and `fun main() { val u = User("A" + "da", "a".clone() + "da", 36); println(u.age) }` runs with `--check-allocations`
- **THEN** it prints `36`, reports no live allocation and exits with status 0

### Requirement: Drop Glue Drops Each Owned Part
Dropping a `String` MUST free its buffer when it owns one. Dropping a struct MUST drop its fields in declaration order, dropping an enum the payloads of its current variant in order, and dropping a nullable its payload when it is not `null`. A field or payload whose type owns no storage MUST be skipped.

#### Scenario: Dropping an enum drops its current variant
- **WHEN** `enum Message { Text(String, Int), Empty }` is declared and `fun main() { val a = Message.Text(1.toString(), 2); val b = Message.Empty; println(1) }` runs with `--check-allocations`
- **THEN** it prints `1`, reports no live allocation and exits with status 0

### Requirement: Clone Glue Builds Independent Values
The built-in `clone()` of a value whose type owns storage MUST run the clone glue of its type: a `String` that owns a buffer MUST be cloned into a new buffer holding exactly its text, one that owns none copied; a struct cloned field by field in declaration order; an enum by its current variant and that variant's payloads in order; a nullable as `null` or a clone of its payload. A part that owns no storage MUST be copied bitwise.

#### Scenario: Cloning a struct of strings
- **WHEN** `struct User : Clone { val name: String, val id: Int }` is declared and `fun main() { val a = User(7.toString(), 7); val b = a.clone(); println(b.name); println(a.name) }` runs with `--check-allocations`
- **THEN** it prints `7` twice, reports no live allocation and exits with status 0

### Requirement: Glue Is Local to Its Code Unit
Each code unit MUST generate the drop and clone glue of the types its bodies drop or clone, including types of other modules, and call it directly, not through the function table, so that a unit's code depends only on its own MIR.

#### Scenario: Drop glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct User { val name: String }` and `fun f(u: User) {}`
- **THEN** after the file's functions it prints the drop glue of `main::User`, headed `function main#drop(main::User)`, and `f` calls it on its argument directly

#### Scenario: Clone glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct User : Clone { val name: String, val id: Int }` and `fun f(u: &User): User = u.clone()`
- **THEN** after the file's functions it prints the clone glue of `main::User`, headed `function main#clone(main::User)`, which `f` calls

## MODIFIED Requirements

### Requirement: Symbols Derive from Declaration Paths
Every compiled function of a declaration MUST have a symbol derived from its declaration's identity alone: `<module>::<name>` for a function, `<module>::<receiver>.<name>` for an extension, where the receiver is written as the ABI writes its type constructor, `<function symbol>#default(<parameter>)` for a default value's generator, and `<module>::<name>#init` and `<module>::<name>#get` for a top-level `val`'s initializer and accessor. Symbols MUST be unique in a program. Drop and clone glue MUST have no slot in the function table: each is local to the code unit that generates it and named `<module>#drop(<type>)` or `<module>#clone(<type>)` after that unit's module and the type it handles.

#### Scenario: Symbols of a file
- **WHEN** `lib/math.prism`, in package `lib`, declares `public fun Int.double(): Int = this * 2`, `public fun scale(a: Int, b: Int = 2): Int = a * b` and `public val BASE: Int = 3`
- **THEN** `prism dump-clif lib/math.prism` prints the functions `lib/math::Int.double`, `lib/math::scale`, `lib/math::scale#default(b)`, `lib/math::BASE#init` and `lib/math::BASE#get`

#### Scenario: Glue of one type in two files
- **WHEN** `lib/user.prism` declares `public struct User { val name: String }` and `public fun drop(u: User): Unit {}`, and `app/main.prism` imports `/lib/user` and declares `fun keep(u: User) {}` and `fun main() { drop(User("a")); keep(User("b")) }`
- **THEN** `prism run --check-allocations app/main.prism` exits with status 0, and `prism dump-clif` of each file prints its own drop glue of `lib/user::User`, `lib/user#drop(lib/user::User)` and `app/main#drop(lib/user::User)`

### Requirement: Aggregates Are Passed by Address
A parameter whose type is a `String`, a struct, an enum or a nullable laid out with a tag MUST be passed as the address of memory the caller owns for the duration of the call, and such a result MUST be written to memory whose address the caller passes before the other arguments. A function's machine signature MUST NOT depend on the layout of any type.

#### Scenario: Signature of a function over structs
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `fun f(p: Point, n: Int): Point = p` are declared and `prism dump-clif main.prism` runs
- **THEN** `f` takes three machine values, the result's address, the address of `p` and the 64-bit `n`, and returns nothing

#### Scenario: A field added keeps the signature
- **WHEN** `Point` of that file gains a field `val z: Int`
- **THEN** `prism dump-clif main.prism` shows the same signature for `f`

#### Scenario: Signature of a function over strings
- **WHEN** `fun greet(name: String, n: Int): String = name` is declared and `prism dump-clif main.prism` runs
- **THEN** `greet` takes three machine values, the result's address, the address of `name` and the 64-bit `n`, and returns nothing

### Requirement: Layout Dump
`prism dump-layout <file.prism>` MUST print, as deterministic text, the layout of each struct and enum the file declares, in declaration order, then of every other struct, enum and nullable type that a compiled body of the file holds, by rendered name: size, alignment, and the offset of every field, tag and payload. A type without a layout yet MUST be listed with the construct it waits for. With type or borrow errors it MUST report them, print nothing and exit with a failure status.

#### Scenario: Dumping a file's layouts
- **WHEN** `prism dump-layout main.prism` runs twice on a file that `prism check` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: A type without a layout yet
- **WHEN** `prism dump-layout main.prism` runs on a file declaring `struct Task { val run: () -> Unit }` and `struct Box<T>(val v: T)`
- **THEN** it lists `Task` as having no layout yet because of function values and closures, and `Box` as laid out per instance, and exits with status 0

#### Scenario: Dumping a file with errors
- **WHEN** `prism dump-layout main.prism` runs on `struct P { val x: Int }` and `fun f(): Int = nope`
- **THEN** it reports `E4001`, prints no layout, and exits with a failure status

### Requirement: Unsupported Constructs Are Rejected Before Code Generation
Before generating any code, the backend MUST check every body it compiles and report `E6001` (`UnsupportedByBackend`) for the first construct of each body that it does not support, naming the construct. A struct, enum or nullable holding a `Heap<T>` or a function value MUST be rejected naming what it holds, until `heap-boxes` and `closures` support them. An unsupported parameter or return type MUST be reported at the declaration, anything else at the first statement or terminator using it, in block order. A rejected program MUST NOT run. `prism check` MUST NOT report `E6001`.

#### Scenario: A struct holding a heap value
- **WHEN** `struct Node { val next: Heap<Int>? }`, `fun size(n: &Node): Int = 1` and `fun main() { println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at the declaration of `size`, naming `'Heap' values`, and exits with status 1

#### Scenario: A struct is rejected
- **WHEN** `struct P { val next: Heap<Int>? }` and `fun main() { val p = P(null); println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at `P(null)`, naming `'Heap' values`, with a note naming the `heap-boxes` change, prints nothing to stdout and exits with status 1

#### Scenario: An enum holding a heap value is rejected
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun main() { val e = Expr.Literal(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at `Expr.Literal(1)`, naming `'Heap' values`, prints nothing to stdout and exits with status 1

#### Scenario: Unsupported constructs in several forms
- **WHEN** a program's `main` runs `val h: Heap<Int>? = null`, or `val f = { 1 }`, or `println(id(1))` for `fun <T> id(x: T): T = x`, or the program declares `compose fun Label() {}` or `fun first(h: Heap<Int>): Int = 1`
- **THEN** `prism run` reports `E6001` naming `'Heap' values` at `null`, closures at `{ 1 }`, calls of generic functions at `id(1)`, composable functions at `Label`, and `'Heap' values` at the declaration of `first`, and runs nothing

#### Scenario: One error per body
- **WHEN** `fun f(): Int { val a: Heap<Int>? = null; val b: Heap<Int>? = null; return 1 }` and `fun g(): Int { val c: Heap<Int>? = null; return 2 }` are declared with a `main` that calls neither
- **THEN** `prism run` reports exactly two `E6001`, one at the first `null` in `f` and one at the `null` in `g`

### Requirement: Cranelift IR Dump
`prism dump-clif <file.prism>` MUST print, as deterministic text, the Cranelift IR of each function the backend generates for the file, in body order, each headed by its symbol, followed by the file's drop and clone glue, each headed by `<module>#drop(<type>)` or `<module>#clone(<type>)`, in order of their names. Like `prism dump-mir --elaborated`, it MUST compile the file alone, against its direct imports' ABIs. When the file has errors of any stage, including `E6001`, it MUST report them, print no IR, and exit with a failure status.

#### Scenario: Dumping a file's IR
- **WHEN** `prism dump-clif main.prism` runs twice on a file that `prism run` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: Dumping a file the backend rejects
- **WHEN** `prism dump-clif main.prism` runs on a file holding `fun f(h: Heap<Int>): Int = 1`
- **THEN** it reports `E6001`, prints no IR, and exits with a failure status

## REMOVED Requirements

### Requirement: Drops of Values That Own Nothing Do Nothing
**Reason**: Drops of values that own storage now run drop glue, so the rule for values that own nothing is one clause of "Drops Run Drop Glue", which also takes its scenario "A non-Copy struct is moved and dropped"; the rejection of aggregates holding a `Heap<T>` or a function value moves to "Unsupported Constructs Are Rejected Before Code Generation", with the scenario "A struct holding a heap value". Its scenario "A struct holding a string" pinned strings as unsupported; the name now belongs to the layout scenario of "Strings Are a Pointer, Two Lengths and a Capacity".
**Migration**: None: no program changes meaning beyond strings running.
