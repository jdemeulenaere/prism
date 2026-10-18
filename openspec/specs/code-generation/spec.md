# Code Generation Specification

## Purpose

The code generation specification defines the Tier 1 backend: how the elaborated MIR of each file is compiled into native code in memory, which constructs the backend compiles today and how it rejects the others, how generated functions are named and called, and how values are represented. What a running program does is the `execution` spec's.

## Requirements

### Requirement: Tier 1 Compiles Elaborated MIR in Memory
`prism run` MUST compile the elaborated MIR of every file of the program into native code in the memory of the `prism` process, without invoking any external program such as a linker or an assembler. Every body MUST be compiled except generic bodies, which wait for monomorphization, and closure bodies, which wait for closure conversion.

#### Scenario: Running a scalar program
- **WHEN** `fun square(x: Int): Int = x * x` and `fun main() { println(square(7)) }` are declared in `main.prism` and `prism run main.prism` runs
- **THEN** it prints `49` and exits with status 0, and no other process is started

#### Scenario: An uncalled generic function is not compiled
- **WHEN** `fun <T> keep(x: T): T = x` and `fun main() { println(1) }` are declared and `prism run main.prism` runs
- **THEN** it prints `1` and exits with status 0

### Requirement: One Code Unit per File
Each file MUST be compiled into its own code unit from its own elaborated MIR alone. A code unit MUST refer to the functions and top-level values of other files only by their symbols, so that its code does not depend on how those files are compiled.

#### Scenario: Compiling independent source files
- **WHEN** a program has an entry file `app/main.prism` importing `/lib/math`
- **THEN** each of the two files is code-generated as a standalone unit, and `prism dump-clif app/main.prism` names the functions of `lib/math` only by their symbols

### Requirement: Symbols Derive from Declaration Paths
Every compiled function of a declaration MUST have a symbol derived from its declaration's identity alone: `<module>::<name>` for a function, `<module>::<receiver>.<name>` for an extension, where the receiver is written as the ABI writes its type constructor, `<function symbol>#default(<parameter>)` for a default value's generator, and `<module>::<name>#init` and `<module>::<name>#get` for a top-level `val`'s initializer and accessor. Symbols MUST be unique in a program.

#### Scenario: Symbols of a file
- **WHEN** `lib/math.prism`, in package `lib`, declares `public fun Int.double(): Int = this * 2`, `public fun scale(a: Int, b: Int = 2): Int = a * b` and `public val BASE: Int = 3`
- **THEN** `prism dump-clif lib/math.prism` prints the functions `lib/math::Int.double`, `lib/math::scale`, `lib/math::scale#default(b)`, `lib/math::BASE#init` and `lib/math::BASE#get`

### Requirement: Glue Has No Function Table Slot
Drop, clone and equality glue MUST have no slot in the function table: each is local to the code unit that generates it and named `<module>#drop(<type>)`, `<module>#clone(<type>)` or `<module>#equals(<type>)` after that unit's module and the type it handles.

#### Scenario: Glue of one type in two files
- **WHEN** `lib/user.prism` declares `public struct User { val name: String }` and `public fun drop(u: User): Unit {}`, and `app/main.prism` imports `/lib/user` and declares `fun keep(u: User) {}` and `fun main() { drop(User("a")); keep(User("b")) }`
- **THEN** `prism run --check-allocations app/main.prism` exits with status 0, and `prism dump-clif` of each file prints its own drop glue of `lib/user::User`, `lib/user#drop(lib/user::User)` and `app/main#drop(lib/user::User)`

### Requirement: Calls Go Through the Function Table
Every call of a top-level function, extension, default value generator or top-level `val` accessor, in the same file or another, MUST load its target from the callee's slot in a process-wide function table and call it indirectly. A slot MUST keep its address for the life of the process, and a slot whose function is not linked MUST hold a stub that ends the program with a runtime error. No call MUST be inlined.

#### Scenario: Calls within and across files are indirect
- **WHEN** `app/main.prism` imports `/lib/math`, which declares `public fun twice(n: Int): Int = n * 2`, and declares `fun helper(): Int = 1` and `fun main() { println(twice(helper())) }`
- **THEN** `prism dump-clif app/main.prism` shows each of the two calls in `main` as a load from the callee's slot followed by an indirect call, and `prism run app/main.prism` prints `2`

#### Scenario: Recursive calls
- **WHEN** `fun fact(n: Int): Int = if (n <= 1) 1 else n * fact(n - 1)` and `fun main() { println(fact(20)) }` run
- **THEN** the program prints `2432902008176640000`

### Requirement: Scalar Values Are Represented Natively
An `Int` MUST be a 64-bit integer, a `Double` a 64-bit IEEE 754 float, a `Bool` a byte holding 0 or 1, a `Char` a 32-bit Unicode scalar value, and a reference a pointer. `Unit` and `Nothing` MUST have no representation: a `Unit` parameter or result is left out of a function's signature, and a function returning `Nothing` has no result. Functions MUST use the host platform's C calling convention.

#### Scenario: Signatures of scalar functions
- **WHEN** `fun f(n: Int, b: Bool, c: Char, d: Double, r: &Int): Double = d` and `fun g(n: Int) {}` are declared and `prism dump-clif main.prism` runs
- **THEN** `f` takes a 64-bit integer, an 8-bit integer, a 32-bit integer, a 64-bit float and a pointer and returns a 64-bit float, and `g` takes a 64-bit integer and returns nothing

### Requirement: References to Scalars
A shared or mutable reference whose referent the backend supports MUST point at the borrowed place itself: borrowing a local MUST keep that local in memory for as long as its storage lives and give its address, and reading or writing through a reference MUST read or write the referent.

#### Scenario: Writing through a mutable reference
- **WHEN** `fun bump(r: &mut Int) { *r = r + 1 }` is declared and `fun main() { var n = 1; bump(&mut n); bump(&mut n); println(n) }` runs
- **THEN** the program prints `3`

#### Scenario: Default value reading an earlier argument
- **WHEN** `fun f(a: Int, b: Int = a + 1): Int = a * b` is declared and `fun main() { println(f(3)); println(f(3, 5)) }` runs
- **THEN** the program prints `12` and then `15`

### Requirement: Structs Are Laid Out Inline
A struct value MUST be stored inline, its fields in its own storage without heap indirection. Its fields MUST be placed in order of decreasing alignment, fields of equal alignment in declaration order, each at the next multiple of its alignment; the struct's alignment MUST be its largest field's (1 without fields) and its size the end of its last field rounded up to that alignment.

#### Scenario: Value struct definition
- **WHEN** `struct Point { val x: Double, val y: Double }` is declared and used in `fun main() { val p = Point(1.0, 2.0); println(p.y) }`
- **THEN** `prism dump-layout main.prism` shows `Point` with size 16 and alignment 8, `x` at offset 0 and `y` at offset 8, and `prism run main.prism` prints `2.0`

#### Scenario: Fields ordered by alignment
- **WHEN** `struct S { val a: Bool, val n: Int, val b: Bool }` is declared
- **THEN** `prism dump-layout` shows `S` with size 16, `n` at offset 0, `a` at 8 and `b` at 9

### Requirement: Enums Are Laid Out as a Tag and Payloads
An enum value MUST store its variant's index, in declaration order, in the smallest unsigned integer of 8, 16 or 32 bits that numbers every variant, at offset 0, followed by the variant's payload, laid out as a struct's fields are, starting after the tag. All variants MUST share the storage after the tag; the enum's size MUST be its largest variant's end rounded up to its alignment.

#### Scenario: Payload-free enum
- **WHEN** `enum Color { Red, Green, Blue }` is declared
- **THEN** `prism dump-layout` shows `Color` with size 1, alignment 1 and an 8-bit tag at offset 0

#### Scenario: Recursive enum with Heap indirection
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }` is declared
- **THEN** `prism dump-layout` shows a finite layout for `Expr`: size 24, an 8-bit tag at offset 0, the payload of `Literal` at 8, and the payloads of `Add` at 8 and 16

#### Scenario: A small payload follows the tag
- **WHEN** `enum Flag { On(Bool), Off }` is declared
- **THEN** `prism dump-layout` shows `Flag` with size 2, its tag at offset 0 and the payload of `On` at 1

### Requirement: Nullables Are Laid Out With a Tag or as a Null Pointer
A `T?` MUST be laid out as an 8-bit tag at offset 0, 0 for `null` and 1 otherwise, followed by `T` at the next multiple of its alignment, except that `(&T)?`, `(&mut T)?` and `Heap<T>?` MUST be a single pointer whose value is 0 exactly when it is `null`.

#### Scenario: A nullable integer
- **WHEN** a body holds a value of type `Int?`
- **THEN** `prism dump-layout` shows `Int?` with size 16, its tag at offset 0 and its payload at 8

#### Scenario: An optional borrow is one pointer
- **WHEN** `struct Point : Copy { val x: Int }` is declared and a function takes `a: (&Point)?` and `b: (&mut Point)?`
- **THEN** `prism dump-layout` shows both nullable types with the size and alignment of a pointer and no tag

### Requirement: Instances of Generic Types Are Laid Out by Substitution
A generic struct or enum used at concrete type arguments outside a generic body MUST be laid out as its declaration with the arguments substituted for its type parameters, and values of it MUST be compiled like those of any struct or enum.

#### Scenario: A generic struct at concrete types
- **WHEN** `struct Pair<A, B>(val a: A, val b: B)` is declared and `fun main() { val p = Pair(1, true); println(p.b) }` runs
- **THEN** `prism dump-layout main.prism` shows `Pair<Int, Bool>` with size 16, `a` at 0 and `b` at 8, and the program prints `true`

### Requirement: Layouts of Types From Modules Not Imported
A file MUST lay out every value it can hold, including values of private types and of types defined in modules it does not import, from its own declarations and the ABIs of its direct imports alone.

#### Scenario: A type reached through an imported struct
- **WHEN** `geo/core.prism` declares `public struct Vec : Copy { val x: Int }`, `geo/shapes.prism` imports `/geo/core` and declares `public struct Circle : Copy { val center: Vec, val r: Int }`, `public fun unit(): Circle = Circle(Vec(3), 1)` and `public fun norm(v: Vec): Int = v.x`, and `app/main.prism` imports only `/geo/shapes` and declares `fun main() { val c = unit(); val d = c; println(d.r); println(norm(c.center)) }`
- **THEN** `prism run app/main.prism` prints `1` and then `3`, and `prism dump-layout app/main.prism` shows `geo/shapes::Circle` with size 16

#### Scenario: A private field type
- **WHEN** `lib/a.prism` declares `struct Secret : Copy { val k: Int }`, `public struct Outer : Copy { val s: Secret, val n: Int }` and `public fun make(): Outer = Outer(Secret(1), 2)`, and `app/main.prism` imports `/lib/a` and declares `fun main() { val o = make(); println(o.n) }`
- **THEN** `prism run app/main.prism` prints `2`

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

### Requirement: Top-Level Values Have Storage and an Accessor
Each top-level `val` MUST have storage in its file's code unit and an accessor that initializes the storage on its first call, as the `execution` spec defines, and returns its address. A read of a top-level `val`, in its own file or another, MUST call the accessor through the function table.

#### Scenario: Reading an imported top-level value
- **WHEN** `lib/config.prism` declares `public val LIMIT: Int = 10`, and `app/main.prism` imports `/lib/config` and declares `fun main() { println(LIMIT + 1) }`
- **THEN** `prism run app/main.prism` prints `11`

### Requirement: Cranelift IR Dump
`prism dump-clif <file.prism>` MUST print, as deterministic text, the Cranelift IR of each function the backend generates for the file, in body order, followed by the file's drop, clone and equality glue in order of their names, each headed by its symbol. Like `prism dump-mir --elaborated`, it MUST compile the file alone, against its direct imports' ABIs. When the file has errors of any stage, including `E6001`, it MUST report them, print no IR, and exit with a failure status.

#### Scenario: Dumping a file's IR
- **WHEN** `prism dump-clif main.prism` runs twice on a file that `prism run` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: Dumping a file the backend rejects
- **WHEN** `prism dump-clif main.prism` runs on a file holding `fun f(g: () -> Int): Int = 1`
- **THEN** it reports `E6001`, prints no IR, and exits with a failure status

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
Each code unit MUST generate the drop, clone and equality glue of the types its bodies drop, clone, or compare through boxes or beyond the inline budget of equality, including types of other modules and the types those reach through boxes, and call it directly, not through the function table, so that a unit's code depends only on its own MIR.

#### Scenario: Drop glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct User { val name: String }` and `fun f(u: User) {}`
- **THEN** after the file's functions it prints the drop glue of `main::User`, headed `function main#drop(main::User)`, and `f` calls it on its argument directly

#### Scenario: Clone glue in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct User : Clone { val name: String, val id: Int }` and `fun f(u: &User): User = u.clone()`
- **THEN** after the file's functions it prints the clone glue of `main::User`, headed `function main#clone(main::User)`, which `f` calls

#### Scenario: Glue of an imported recursive type
- **WHEN** `lib/tree.prism` declares `public enum Tree { Leaf, Node(Heap<Tree>, Int, Heap<Tree>) }` and `public fun leaf(): Tree = Tree.Leaf`, and `app/main.prism` imports `/lib/tree` and declares `fun main() { val t = Tree.Node(Heap(leaf()), 1, Heap(leaf())); println(t == t) }`
- **THEN** `prism run --check-allocations app/main.prism` prints `true` and exits with status 0, and `prism dump-clif app/main.prism` prints `app/main#drop(lib/tree::Tree)`, `app/main#drop(Heap<lib/tree::Tree>)` and `app/main#equals(lib/tree::Tree)`

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

#### Scenario: A struct holding a heap value
- **WHEN** `struct Node { val next: Heap<Int>? }`, `fun size(n: &Node): Int = 1` and `fun main() { println(size(Node(Heap(1)))) }` are declared
- **THEN** `prism run --check-allocations main.prism` prints `1` and exits with status 0

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

### Requirement: Large Aggregates Are Compared by Equality Glue
Each `==` and `!=`, and each equality glue function, MUST expand inline at most 32 comparisons of the struct, enum and nullable parts it compares, counting one for each scalar, string, enum tag, nullable presence and box compared by glue that a part's expansion holds. A part in memory whose expansion does not fit in what remains MUST be compared by calling its equality glue, `<module>#equals(<type>)`, which expands that type's own parts under its own budget.

#### Scenario: A large struct is compared by equality glue
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct P : Copy { val a: Int, val b: Int, val c: Int, val d: Int }`, `struct Q : Copy { val a: P, val b: P, val c: P, val d: P }`, `struct R : Copy { val a: Q, val b: Q, val c: Q }` and `fun same(x: R, y: R): Bool = x == y`
- **THEN** `same` calls `main#equals(main::R)`, which compares the fields `a` and `b` inline and calls `main#equals(main::Q)` for `c`, and `main#equals(main::Q)` compares its sixteen `Int`s inline, calling no equality glue

### Requirement: Equality Glue Gives the Full Expansion's Result
Comparing through equality glue MUST give the same result as the full inline expansion: padding, inactive payloads and addresses are never compared. Generating code for equality MUST take time linear in the number of fields and payloads of the types compared.

#### Scenario: Equality of a large aggregate
- **WHEN** a program declares a struct of five structs, each holding two `Copy` structs of two `Int`s, a `String`, an `Int?` and an enum with a `Double` and a struct payload, and prints the comparisons of such values that are equal, that differ in the `Int?` of the first part, in the struct payload, the string, the presence of the `Int?` or the variant of the last part, and of a value holding `NaN` with itself
- **THEN** `prism run --check-allocations` prints `true` for the equal values and for `!=` of the values of different variants, `false` for the others, and exits with status 0

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

### Requirement: Aggregates Holding Function Values Are Rejected
A struct, enum or nullable holding a function value MUST be rejected with `E6001`, naming what it holds, until `closures` supports them.

#### Scenario: A struct holding a function value
- **WHEN** `struct Task { val run: () -> Int }`, `fun size(t: &Task): Int = 1` and `fun main() { println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at the declaration of `size`, naming function values and closures, and exits with status 1

#### Scenario: An enum holding a function value is rejected
- **WHEN** `enum Job { Run(() -> Int), Idle }` and `fun main() { val j = Job.Idle }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at `Job.Idle`, naming function values and closures, prints nothing to stdout and exits with status 1

### Requirement: A Template Is One Runtime Call
The backend MUST lower a template rvalue to one call of a runtime function receiving the destination and a table of the parts in order: text parts as the address, byte length and character count of the code unit's literal data, string parts as the address of the borrowed string, and scalar parts as their values. A template MUST NOT call any other string or conversion function of the runtime, and MUST NOT be rejected with `E6001`.

#### Scenario: A template calls the runtime once
- **WHEN** `fun label(n: Int, d: Double, s: &String): String = "Count: $n of $s at $d"` is declared and `prism dump-clif main.prism` runs
- **THEN** the body of `label` calls the runtime's template function once and calls no runtime concatenation or `toString()` function

### Requirement: A Template Allocates Its Result Once
The runtime MUST format a template's scalar parts itself, with the text their built-in `toString()` gives, and MUST build the result with exactly one allocation, of exactly its length in bytes, or with none when the result is empty.

#### Scenario: A template allocates its result once
- **WHEN** `fun label(n: Int, s: &String): String = "Count: $n, ${n * 2} of $s"` is compiled and called with `7` and a borrowed literal `"x"`, then its result is dropped, under the allocation check
- **THEN** the result is `Count: 7, 14 of x`, exactly one allocation is made, and none is live afterwards
- **WHEN** `fun empty(s: &String): String = "$s$s"` is called with a borrowed empty literal
- **THEN** the result is empty and no allocation is made

## Design Rationale

Decisions from `codegen-core` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Two crates** (D1). `crates/prism-codegen` holds everything that knows Cranelift: the support and entry checks (`supported.rs`, `entry.rs`), symbols (`symbols.rs`), representation and signatures (`representation.rs`), lowering of one body (`lower.rs`), one file's code unit (`unit.rs`), the function table and linker (`link.rs`), program execution (`program.rs`) and the IR dump (`dump.rs`). `crates/prism-runtime` holds the `extern "C"` functions generated code calls and depends on nothing, so Tier 2 can link it without Cranelift. Rejected: the runtime inside the codegen crate; a separate `prism-jit` crate before Tier 2 exists.
- **Cranelift pinned to `=0.136.2`, fixed settings** (D2). The exact pin keeps the generated IR, `prism dump-clif` and the timing baseline stable until a deliberate upgrade. The ISA is the host's with `opt_level=none` (`speed` until the compile-speed pass; see below), the verifier in debug builds only, frame pointers kept, inline stack probes, `use_colocated_libcalls=false`, and `is_pic` on x86-64 only (as `cranelift-jit` requires). Rejected: a version range.
- **One code unit per file, one JIT module per load** (D3). A unit is lowered from its file's elaborated MIR alone, so its code depends on the file, its direct imports' ABIs (through its own MIR), the compiler and the target, which keeps per-file object code cacheable and hot reload per file. The units of one load share one `JITModule`, finalized together; `hot-reload` will define a changed file's unit into a new module. Rejected: a module per file (separate executable pages and finalizations), one module with direct cross-file calls (ties code to callees' addresses).
- **Every call goes through the function table** (D4, D26). One pointer-sized slot per function, default generator and `val` accessor, in chunks that never move; code refers to a slot only as the imported data symbol `<symbol>#slot`, resolved by the JIT's symbol lookup, loads it and calls with `call_indirect`, also within a file, and nothing is inlined, so after `hot-reload` repoints a slot every later call, even from a frame of old code, reaches the new code. A slot no load fills holds `prism_rt_unlinked_function`. Runtime functions are called directly; the runtime is never reloaded. The table is owned by the linker and then the `Program`, not a `static`, so a test process can link several programs. Rejected: direct calls within a file, slot addresses as code constants (unrelocatable), a global table.
- **Readable symbols from declaration paths** (D5): `m::name`, `m::<receiver>.name` with the receiver written as the ABI writes its constructor (`lib/team.Team`), `<function>#default(p)`, `m::name#init`, `#get`, `#storage`, `#state` and `#slot`; `closures` will add `#closure<i>` and `monomorphization` type arguments. Tier 2 will escape them for object files. Rejected: a C++-style mangling now; bare receiver names.
- **Scalars natively, the host's C convention** (D6). `Int` is `i64`, `Double` `f64`, `Bool` `i8` (0 or 1, zero-extended at calls as Apple's arm64 convention requires), `Char` `i32`, a reference a pointer; `Unit` and `Nothing` have no representation and are left out of signatures. A caller derives the callee's signature from the call's parameter types and destination, so no ABI lookup is needed. Rejected: Cranelift's `tail` convention, `i32` booleans, a materialized `Unit`.
- **Locals are SSA variables unless borrowed** (D7). A borrowed local lives in a stack slot; every other one is a Cranelift `Variable`, so Cranelift builds SSA (drop flags and block-local temporaries excepted since the compile-speed pass; see below). Storage markers and `Read` generate nothing; `SwitchInt` is `brif` or a `Switch`. Rejected: every local in memory. (`codegen-aggregates` puts every struct, enum and tag-byte nullable in memory, `Range` included, and drops the `Range` special case; see below.)
- **References to scalars are supported now** (D8), because every top-level `val` read and every default generator already uses them in MIR. References to aggregates, field borrows and `(&T)?` came with `codegen-aggregates`. Rejected: special-casing `const &VAL`.
- **Unsupported constructs are a compile-time error over every compiled body** (D9). The check runs on each file's MIR before any code is generated and reports one `E6001` per body, naming a reference by its referent and the change that will support the construct; it covers every body, not only those reachable from `main`, which would need a whole-program call graph and would make a program run or fail depending on which functions an edit calls. Rejected: runtime-panicking stubs, compiler panics.
- **Data is never addressed PC-relatively** (D26). Slots, file names, panic details, value storage and the stack limit are non-colocated symbols, since `cranelift-jit`'s memory does not promise ±2 GiB reach.
- **`prism dump-clif`** (D23) prints each function's Cranelift IR, headed by its symbol, for debugging and for the scenarios about generated code; executable fixtures are the main tests, so there are no IR snapshots, which every Cranelift upgrade would churn.
- **Separate `code-generation` and `execution` specs** (D25). This spec holds the backend's contract, which later backend changes extend (`codegen-aggregates` adds layouts); `execution` holds the runtime semantics every tier must share.

Decisions from `codegen-aggregates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **What the backend compiles** (D1, D19). Every non-generic, non-closure body over scalars, `Unit`, structs, enums, nullables, `Range` and references to them, as long as no aggregate holds a `String`, a `Heap<T>` or a function value (`runtime-strings-and-drops` lifted the `String` restriction; see below). Concrete instances of generic types (`Pair<Int, Bool>`) are laid out by substituting their arguments, since the layout code is the same as for any struct; only generic *bodies* wait for `monomorphization`. `classify` looks into fields and payloads and names the first missing construct ("strings", "'Heap' values", "function values and closures"), so a struct is rejected for what it holds; a missing definition is an internal error, since the ABI carries every reachable one (see `compilation-and-abi`).
- **Structs inline, fields by decreasing alignment** (D5): equal alignments keep declaration order, so `struct S { val a: Bool, val n: Int, val b: Bool }` takes 16 bytes instead of C's 24, and the order is stable and documented so dumps and tests can pin it. Programs never see it: construction, evaluation and equality follow declaration order. Rejected: C order (padding with no FFI to justify it; a later C-layout attribute can opt in), Rust's unspecified order.
- **Enums are the smallest tag, then the payloads** (D6): a `u8`, `u16` or `u32` index at offset 0, each variant's payload laid out like a struct starting right after the tag (a `Bool` payload sits at 1), every variant sharing the storage. An enum without variants is uninhabited. Rejected: an `Int`-wide tag, a common payload offset, niche filling, a trailing tag.
- **Nullables are a tag byte, or the null pointer** (D7): `T?` is a `u8` tag (0 for `null`, matching MIR's `Discriminant`) then `T` at its alignment; `(&T)?`, `(&mut T)?` and `Heap<T>?` are one pointer, null being 0, so every optional borrow MIR produces costs one pointer. Rejected: one representation for all nullables, general niche filling now (a later change can add it with no observable effect).
- **Sizes** (D8): `Int`, `Double` and pointers 8, `Char` 4, `Bool` 1; `Unit` and fieldless structs 0 bytes (storage is at least one byte so every value has an address); `Heap<T>` is a pointer, so recursive enums have a layout (its values came with `heap-boxes`); `String` and function types have no layout until `runtime-strings-and-drops` and `closures` choose theirs (`String` now has its fixed 32-byte layout; see below).
- **Aggregates live in memory, scalars in variables** (D9). A struct, enum or tag-byte nullable local is a stack slot of its layout; an aggregate argument is used in place, and the return place is the caller's memory. Rejected: scalar replacement of unborrowed aggregates, an optimization with no observable effect that needs a second representation and cannot split overlapping payloads; Tier 2 gets it from LLVM.
- **Passed by address, returned through a caller address** (D10). An aggregate parameter is one pointer and an aggregate result a pointer parameter placed first, so a machine signature never depends on a layout and a function-table slot keeps its signature when a type gains a field. The caller passes a moved operand's own place (dead after the move), copies a `Copy` operand into a fresh slot (so a write through another argument cannot change it), and passes its destination as the result address only when the destination is a whole, unborrowed local no argument is rooted at, else a fresh slot copied afterwards. A layout change still recompiles every function touching the type, which the interface hashes find (see `compilation-and-abi`); what happens to live values is `hot-reload`'s. Rejected: `StructReturn` (its C registers are not needed between Prism functions), multi-value returns and per-field arguments (signatures would follow layouts), small aggregates in registers (a follow-up), callee-allocated results.
- **Places, copies and moves** (D11, D12). A place's address follows its projections (`Deref` loads, `Field` and a tag-byte `Payload` add offsets, a pointer-like `Payload` adds nothing); `Subtype` is lowered only where both sides share a representation. A copy and a move are the same bytes copied (`emit_small_memory_copy`, in its overlapping form when source and destination share a root or go through a dereference), and an `Aggregate` whose memory operands may overlap the destination copies them to temporaries first. Nothing is zeroed or flagged after a move.
- **Equality is expanded inline** (D13): fields in declaration order, enum tags then the current variant's payloads, nullables by presence then payload, references by referent, each scalar as its type compares, so padding and inactive payload bytes are never read and `NaN` and `-0.0` behave. Rejected: `memcmp` (reads padding, wrong for floats), per-type equality functions now (`runtime-strings-and-drops` kept equality inline and left them to `heap-boxes`; see below). Expansion is bounded by the type's size, since it stops at `Heap<T>` layers, where equality glue takes over (`heap-boxes`, below), and by an inline budget since the linear-time fix (below).
- **Drops of values that own nothing compile to nothing** (D14, D15): a `Drop` of a supported type is a jump, drop flags stay ordinary `Bool` locals, and `clone()` of such a value is a bitwise copy, so non-`Copy` aggregates of scalars run with their moves checked as they will be later. Rejected: rejecting every non-`Copy` aggregate until drop glue exists. (Types that own storage run drop and clone glue since `runtime-strings-and-drops`; see below.)
- **Top-level values build in place** (D17): `#storage` is sized by the layout and the accessor passes its address as `#init`'s result address.
- **`prism dump-layout`** (D18) prints the file's own structs and enums in declaration order, then every other struct, enum and nullable its compiled bodies hold (through fields, payloads and referents, not `Heap<T>`) by rendered name, with sizes, alignments, offsets and tags; a type without a layout is listed with what it waits for. `layout.snap` fixtures pin the layout rules, which `prism dump-clif` (no snapshots) could not. Rejected: layouts inside `dump-clif`, a `--layouts` flag on `dump-mir`.

Decisions from `runtime-strings-and-drops` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **A string is a pointer, two lengths and a capacity** (D2). 32 bytes, pointer-aligned: the UTF-8 text's first byte (never null; an empty text points at static data), the byte length, the character count and the capacity of the owned buffer, 0 when the value owns none. The runtime's `#[repr(C)] PrismString` (`crates/prism-runtime/src/string.rs`) and the layout table agree on it, so `String?` is 40 bytes by the nullable rule. The character count keeps `length` O(1) under character semantics (see `execution`) and tells ASCII text (count equal to byte length) for O(1) indexing; every string is built by the compiler or by a runtime function that knows its inputs' counts. Rejected: Rust's three words (`length` would scan), a capacity in the buffer's header, a small-string representation now (a branch on every read; a later optimization), reference counting or copy-on-write (hidden ARC, against the zero-cost memory safety invariant), a null-pointer niche for `String?` now (with the other niches later).
- **Literals are static data; ownership is a property of the value** (D3). A literal's bytes are emitted once per code unit, and evaluating it writes the four words without a call; capacity 0 means its drop frees nothing and its clone copies the words, since nothing can change or free static text. Every string the runtime builds is exactly sized, and nothing writes into a buffer yet, so no copy-on-write rule exists; an in-place operation must first copy text it does not own. Literal data must outlive every value pointing at it, so `hot-reload` must keep old code modules mapped. Rejected: allocating per evaluation, a separate `StaticString` type.
- **String operations are runtime functions** (D7). Generated code inlines only building a literal and reading `length` (a load at offset 16); concatenation, equality, indexing (with its bounds check and location), clone, drop, printing and `toString()` of each scalar are direct calls of `prism_rt_*` functions taking strings by address and writing results to a destination address. Equality compares byte lengths then bytes, since UTF-8 is canonical; strings may contain `\0`. Rejected: string loops generated in Cranelift (more IR per use and per tier), null termination.
- **The allocator is the runtime over Rust's global allocator, with sized frees** (D8). `prism_rt_allocate(size, alignment)` and `prism_rt_free(pointer, size, alignment)` over `std::alloc`, every allocation of a program going through them, so a size-class allocator can replace the global one without changing callers and the allocation check can verify each free. A failed allocation or an impossible size is "out of memory". A zero-sized allocation is a dangling aligned pointer, never allocated or tracked (D22), which `heap-boxes` uses for boxes of zero-sized values. The allocation map has its own lock, so reporting an invalid free, which flushes the output, never runs with the map locked (D22). Rejected: an arena freed at exit (a long-running UI would grow without bound), the C library's `malloc` from generated code, Prism's own allocator now, counting allocations in normal runs.
- **Drop glue is generated per code unit, per type** (D9). A type that owns no storage has none (its `Drop` is a jump); a `String` is dropped by `prism_rt_string_drop`; a struct drops its owning fields in declaration order (Rust's order), an enum switches on its tag and drops the current variant's owning payloads, a nullable tests its tag, each part through its own glue, as `<unit module>#drop(<type>)` functions with local linkage, memoized in the unit's `GlueTable` (`crates/prism-codegen/src/glue.rs`), lowered after its bodies, and called directly through non-colocated references. Drop flags need nothing new: they are MIR `Bool` locals tested before the `Drop`. A unit's code then depends only on its own MIR, instances of generic types get glue where they are used, and hot reload stays sound (a unit is recompiled when a type it handles changes layout, and old frames keep calling old glue). Glue symbols qualify every struct and enum by its module, also in type arguments. Glue does not check the stack, since nothing recursive can be dropped yet; `heap-boxes` must add the check or drop long chains iteratively. Rejected: glue inlined at each drop (code grows with the type, and recursive types cannot be expanded), glue in the defining file's unit through the function table (an indirect call per drop, and no file defines `Pair<String, Int>`'s), a runtime interpreter of layout descriptions, anonymous functions (`prism dump-clif` could not name them). Duplicate glue across units costs a few instructions per type; Tier 2 can merge it.
- **Clone glue follows drop glue** (D10). `<unit module>#clone(<type>)` takes the destination's and the source's addresses, copies the whole value bitwise (every part that owns nothing, the tag and the padding) and then replaces each owning part of the current variant or payload, in declaration order, by a clone made by its own glue; a `String` is cloned by `prism_rt_string_clone` (a copy when it owns no buffer, a new exactly sized one otherwise). Glue exists only for `Clone` types, runs no user code, and shares no owned memory with the source.
- **Equality stays inline; strings compare in the runtime** (D11). Without `Heap<T>` no type reaches itself, so inline expansion still terminates; a `String` part calls `prism_rt_string_equals`. `heap-boxes` turned the comparison of boxed aggregates into `#equals` glue functions of the same kind (below).
- **Strings in the calling convention** (D16). `String` is `Representation::Memory`, so every rule for aggregates applies: a stack slot per local, parameters by the address of caller-owned memory (a literal operand is first built in a slot), results through the caller's result address, a move a 32-byte copy that leaves the source untouched. A top-level `String`'s `#storage` is 32 bytes and its accessor registers it with the runtime once initialized (see `execution`).
- **What the backend still rejects** (D18). Strings and every type holding them are supported; `classify` names, in field order, function values and closures (`closures`), calls of generic functions (`monomorphization`), composable functions (`compose-transform`) and values of unknown type, and the fixtures that pinned strings as unsupported pinned `Heap<T>` until `heap-boxes` moved them to function values.

Decisions from `heap-boxes` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **A box is a pointer to an exactly sized allocation** (D3): non-null, `T`-aligned, `size_of(T)` bytes from the runtime allocator, freed with the same size and alignment. The argument is evaluated before allocating, so `Heap(make())` runs `make` before the allocation can fail ("out of memory", no location). A zero-sized `T` allocates nothing: the box is the address equal to `T`'s alignment, as Rust's `NonNull::dangling`. `Heap<T>?` stays one pointer, null being 0. Rejected: a header word (nothing reads it, and a count would be hidden reference counting, which the zero-cost memory safety invariant forbids); one byte for zero-sized contents; placement construction (it reorders "out of memory" before the argument's effects; a later optimization).
- **Box glue drops the contents, then the allocation** (D12): `#drop(Heap<T>)` runs `T`'s drop glue then frees, `#clone(Heap<T>)` allocates then clones or copies the contents into the new box, and `unbox` and construction are lowered inline. Aggregates call the box's glue for box parts, a `Heap<T>?` tests its pointer for 0, and a recursive type's glue is recursive.
- **Boxed aggregates compare by equality glue** (D13). `==` stays structural and never compares addresses; inline expansion stops at box layers, where the contents of a struct, enum or nullable of one are compared by `<module>#equals(<type>)`, local to the unit like drop and clone glue (the expansion moved to `equality.rs`, shared by bodies and glue). Scalars, strings and nullable scalars in a box compare inline. There is no pointer-equality shortcut, since a boxed `NaN` is unequal to itself. Rejected: equality glue for every aggregate; glue only for types that reach themselves; a pointer shortcut for types without `Double`.
- **Equality compiles in linear time** (plain fix after `heap-boxes`; autonomous). Expanding `==` fully inline made code generation super-linear: generating the code of a program comparing two values of a struct of 4096 `Int` leaves twice took 5.8 s in a debug build and 197 ms in a release build. Cranelift's optimizer spends time proportional to the dominator tree's depth on each new pure instruction, and the expansion's branch after every scalar made that depth the number of scalars; a chain of `band`s instead is placed by the optimizer at its only use, the end, keeping every result live across the remaining calls, which made register allocation quadratic. So each `==` and each equality glue function expands at most 32 comparisons inline and compares the struct, enum and nullable parts beyond that through their `#equals` glue, one function per type per unit, which bounds both a use site's code and a function's branches; the parts' results are joined without branching, an enum's tag and a nullable's presence being the only branches; and the conjunction is written to a byte of the frame before each call, before each branch and after every 8 results, unless the whole comparison has at most 8 results and no call, which keeps a `Point` or an `Int?` comparison in registers. Parts are read at offsets of the loads rather than through address arithmetic. Compile time is linear in the fields and payloads of the compared types, plus 32 comparisons per glue function: that program takes 59 ms in a debug build and 14 ms in a release build, and `crates/prism-cli/tests/equality_timing.rs` (ignored by default) reports the growth for nested structs, wide structs of `Int?` and of `String` and nested enums. Equality results are unchanged; a part is compared even after an earlier part differed, which costs nothing observable since equality runs no user code. Rejected: glue for every aggregate (a call for every `Point == Point`); a static size threshold per type (a type with thousands of fields would still be one deep function); keeping the branch after every part (the depth again); `opt_level=none` for such functions (one ISA serves the whole JIT module).
- **Glue checks the stack** (D14). Every drop, clone and equality glue function checks the stack on entry like a compiled body, so a structure deeper than the 8 MiB stack is the runtime error "stack overflow", written without a location since glue has no declaration, instead of a crash. A linked list of a few hundred thousand nodes overflows on drop, as Rust's `Box` lists do; iterative drop of box chains is a follow-up that changes no program's meaning. Rejected: iterative drop now; no stack check (a segmentation fault); passing a location to every glue call.

Decisions from the compile-speed pass (plain fixes after `heap-boxes`, made autonomously on 2026-10-18 and logged in `docs/decisions-for-review.md`; `crates/prism-cli/tests/compile_timing.rs`, ignored by default, times every stage on generated programs):

- **Drop flags live in stack slots** (amends D7 of `codegen-core`). A drop flag is live from its value's initialization to the end of its scope, and Cranelift's SSA construction visits every merge between a variable's definition and its use once per variable, so a body with many conditionally moved values (4096 values each moved under an `if`) spent 4.1 s lowering and 13.9 s in Cranelift in a release build, its IR holding millions of aliases. A drop flag is now a one-byte stack slot, written where MIR assigns it and loaded where a drop tests it, and that body lowers in 19 ms. Every other unborrowed scalar stays a variable. Rejected: a stack slot for every scalar live across many merges (a per-local analysis for the same effect); keeping flags in variables and capping a body's branches (no program should be refused for its shape).
- **Tier 1 compiles with `opt_level=none`** (amends D2 of `codegen-core`). Tier 1 is the development tier, where compile time matters most, and Cranelift's optimizer cost more than it saved: its egraph pass grows with the depth of the dominator tree, so a body of 4096 conditionally moved values spent 1.1 s in Cranelift with `speed` and 45 ms with `none`, a `when` of 8192 patterns 440 ms and 234 ms, 4096 live borrows 582 ms and 289 ms, and code generation of `examples/run/main.prism` went from 2.4 ms to 1.8 ms (release builds). Run time barely moved: recursive calls, trial division and string building ran in the same time, and a loop rebuilding a `Copy` struct 300 million times took 0.41 s instead of 0.33 s, since every local already lives in a variable or a stack slot as before and calls go through the function table either way. Tier 2 (LLVM) is the optimizing tier. Rejected: `speed` with a per-function size threshold (one ISA serves a JIT module, so it needs a second ISA and compiling outside the module, for a gain only large bodies would see, which are the ones `speed` makes slow); `speed_and_size` (the same egraph pass).
- **A straight-line run of MIR blocks is one Cranelift block**. A MIR block that is the only successor of its only predecessor, through a `Goto`, a returning `Call` or a `Drop`, is lowered after it in the same Cranelift block, so a call no longer ends one. Cranelift's SSA construction walks back through single-predecessor blocks to find a variable's value and records it in each, so a body of 4096 borrows used after 4096 calls spent 88 ms lowering and 288 ms in Cranelift; it now takes 5 ms and 48 ms. Generated code is otherwise the same, with fewer jumps.
- **Large functions use the single-pass register allocator**. Cranelift's default, backtracking allocator grows faster than the function: with the depth of its dominator tree (a body of thousands of sequential branches) and with the values live at once (a call of 4096 arguments took 120 ms). A function of more than `LARGE_FUNCTION_INSTRUCTIONS` (2048) Cranelift instructions, or with a call of more than `LARGE_CALL_ARGUMENTS` (256) arguments, is compiled with `regalloc_algorithm=single_pass` instead, by a second ISA outside the JIT module's own compilation, and defined from its bytes and relocations (`crates/prism-codegen/src/link.rs`), so compiling it stays linear: that call compiles in 1.2 ms, a `when` of 8192 patterns in 37 ms instead of 237 ms, 4096 live borrows in 11 ms instead of 48 ms. Its code runs about half as fast (twice as slow on recursive calls, trial division and a struct-building loop when every function took the single-pass allocator), which only such large, typically generated, functions pay; every function of the reference programs is far below the threshold. Rejected: the single-pass allocator for every function (Tier 1 code twice as slow everywhere); no threshold (seconds for one generated function in a debug build); a threshold on blocks alone (a call of thousands of arguments has four).
- **Block-local temporaries are plain values** (amends D7 of `codegen-core`). A scalar temporary that a statement wholly assigns once and that is mentioned otherwise only after it, in the same MIR block, as the comparisons and payload reads of a `when`'s tests are, keeps its Cranelift value instead of being a variable. Cranelift's SSA construction keeps each variable's value per block in a map sized by the block's number, so a variable defined in a late block of a large body cost that body's size: a `when` of 8192 patterns spent 68 ms lowering, now 20 ms. A temporary read in a later block, written by a call or assigned twice stays a variable.

Decisions from `string-templates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **One runtime call, one allocation** (D12). `prism_rt_string_template` takes the destination, a table of 32-byte parts (a kind word and three payload words: static text with its byte length and character count, a string's address, or a scalar's value) and the count. It measures every part (digit counting for `Int`, each `Double` formatted once into scratch memory that is not a program allocation), allocates exactly the result's length once through `prism_rt_allocate`, or nothing for an empty result, then writes the parts. Rejected: concatenation, `toString()` first, a growable buffer, and an array of pre-converted strings, which all allocate more.
- **The part table is a stack slot** (D13). The backend fills one 8-byte-aligned stack slot per template with ordinary stores (text entries from the unit's deduplicated literal data, `Bool` and `Char` zero-extended to a word) and makes one three-argument call, so a long template never makes its function large through many call arguments; only its stores count towards the instruction threshold, linearly. A table in read-only data with only the value entries written at run time is a later optimization, needing data relocations.
