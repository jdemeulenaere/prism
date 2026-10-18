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
Every compiled function MUST have a symbol derived from its declaration's identity alone: `<module>::<name>` for a function, `<module>::<receiver>.<name>` for an extension, where the receiver is written as the ABI writes its type constructor, `<function symbol>#default(<parameter>)` for a default value's generator, and `<module>::<name>#init` and `<module>::<name>#get` for a top-level `val`'s initializer and accessor. Symbols MUST be unique in a program.

#### Scenario: Symbols of a file
- **WHEN** `lib/math.prism`, in package `lib`, declares `public fun Int.double(): Int = this * 2`, `public fun scale(a: Int, b: Int = 2): Int = a * b` and `public val BASE: Int = 3`
- **THEN** `prism dump-clif lib/math.prism` prints the functions `lib/math::Int.double`, `lib/math::scale`, `lib/math::scale#default(b)`, `lib/math::BASE#init` and `lib/math::BASE#get`

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
A parameter whose type is a struct, an enum or a nullable laid out with a tag MUST be passed as the address of memory the caller owns for the duration of the call, and such a result MUST be written to memory whose address the caller passes before the other arguments. A function's machine signature MUST NOT depend on the layout of any type.

#### Scenario: Signature of a function over structs
- **WHEN** `struct Point : Copy { val x: Int, val y: Int }` and `fun f(p: Point, n: Int): Point = p` are declared and `prism dump-clif main.prism` runs
- **THEN** `f` takes three machine values, the result's address, the address of `p` and the 64-bit `n`, and returns nothing

#### Scenario: A field added keeps the signature
- **WHEN** `Point` of that file gains a field `val z: Int`
- **THEN** `prism dump-clif main.prism` shows the same signature for `f`

### Requirement: Drops of Values That Own Nothing Do Nothing
A value whose type owns no storage (no `String`, `Heap<T>` or function value in it, through any field, payload or nullable) MUST be dropped without running any code. A struct or enum that owns storage MUST be rejected with `E6001` naming what it holds, until drop glue exists.

#### Scenario: A non-Copy struct is moved and dropped
- **WHEN** `struct Counter { var n: Int }`, `fun take(c: Counter): Int = c.n` and `fun flag(): Bool = false` are declared and `fun main() { val c = Counter(4); if (flag()) println(take(c)); println(1) }` runs
- **THEN** the program prints `1` and exits with status 0

#### Scenario: A struct holding a string
- **WHEN** `struct User { val name: String }`, `fun age(u: &User): Int = 1` and `fun main() { println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at the declaration of `age`, naming strings, and exits with status 1

### Requirement: Top-Level Values Have Storage and an Accessor
Each top-level `val` MUST have storage in its file's code unit and an accessor that initializes the storage on its first call, as the `execution` spec defines, and returns its address. A read of a top-level `val`, in its own file or another, MUST call the accessor through the function table.

#### Scenario: Reading an imported top-level value
- **WHEN** `lib/config.prism` declares `public val LIMIT: Int = 10`, and `app/main.prism` imports `/lib/config` and declares `fun main() { println(LIMIT + 1) }`
- **THEN** `prism run app/main.prism` prints `11`

### Requirement: Unsupported Constructs Are Rejected Before Code Generation
Before generating any code, the backend MUST check every body it compiles and report `E6001` (`UnsupportedByBackend`) for the first construct of each body that it does not support, naming the construct. An unsupported parameter or return type MUST be reported at the declaration, anything else at the first statement or terminator using it, in block order. A rejected program MUST NOT run. `prism check` MUST NOT report `E6001`.

#### Scenario: A struct is rejected
- **WHEN** `struct P { val name: String }` and `fun main() { val p = P("a"); println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at `P("a")`, naming strings, prints nothing to stdout and exits with status 1

#### Scenario: An enum holding a heap value is rejected
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun main() { val e = Expr.Literal(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at `Expr.Literal(1)`, naming `'Heap' values`, prints nothing to stdout and exits with status 1

#### Scenario: Unsupported constructs in several forms
- **WHEN** a program's `main` runs `println("hi")`, or `val h: Heap<Int>? = null`, or `val f = { 1 }`, or `println(id(1))` for `fun <T> id(x: T): T = x`, or the program declares `compose fun Label() {}` or `fun first(s: String): Int = 1`
- **THEN** `prism run` reports `E6001` naming strings at `"hi"`, `'Heap' values` at `null`, closures at `{ 1 }`, calls of generic functions at `id(1)`, composable functions at `Label`, and strings at the declaration of `first`, and runs nothing

#### Scenario: One error per body
- **WHEN** `fun f(): Int { val a = "x"; val b = "y"; return 1 }` and `fun g(): Int { val c = "z"; return 2 }` are declared with a `main` that calls neither
- **THEN** `prism run` reports exactly two `E6001`, one at `"x"` in `f` and one at `"z"` in `g`

### Requirement: Cranelift IR Dump
`prism dump-clif <file.prism>` MUST print, as deterministic text, the Cranelift IR of each function the backend generates for the file, in body order, each headed by its symbol. Like `prism dump-mir --elaborated`, it MUST compile the file alone, against its direct imports' ABIs. When the file has errors of any stage, including `E6001`, it MUST report them, print no IR, and exit with a failure status.

#### Scenario: Dumping a file's IR
- **WHEN** `prism dump-clif main.prism` runs twice on a file that `prism run` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: Dumping a file the backend rejects
- **WHEN** `prism dump-clif main.prism` runs on a file holding `fun f(s: String): Int = 1`
- **THEN** it reports `E6001`, prints no IR, and exits with a failure status

### Requirement: Layout Dump
`prism dump-layout <file.prism>` MUST print, as deterministic text, the layout of each struct and enum the file declares, in declaration order, then of every other struct, enum and nullable type that a compiled body of the file holds, by rendered name: size, alignment, and the offset of every field, tag and payload. A type without a layout yet MUST be listed with the construct it waits for. With type or borrow errors it MUST report them, print nothing and exit with a failure status.

#### Scenario: Dumping a file's layouts
- **WHEN** `prism dump-layout main.prism` runs twice on a file that `prism check` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: A type without a layout yet
- **WHEN** `prism dump-layout main.prism` runs on a file declaring `struct User { val name: String }` and `struct Box<T>(val v: T)`
- **THEN** it lists `User` as having no layout yet because of strings, and `Box` as laid out per instance, and exits with status 0

#### Scenario: Dumping a file with errors
- **WHEN** `prism dump-layout main.prism` runs on `struct P { val x: Int }` and `fun f(): Int = nope`
- **THEN** it reports `E4001`, prints no layout, and exits with a failure status

## Design Rationale

Decisions from `codegen-core` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Two crates** (D1). `crates/prism-codegen` holds everything that knows Cranelift: the support and entry checks (`supported.rs`, `entry.rs`), symbols (`symbols.rs`), representation and signatures (`representation.rs`), lowering of one body (`lower.rs`), one file's code unit (`unit.rs`), the function table and linker (`link.rs`), program execution (`program.rs`) and the IR dump (`dump.rs`). `crates/prism-runtime` holds the `extern "C"` functions generated code calls and depends on nothing, so Tier 2 can link it without Cranelift. Rejected: the runtime inside the codegen crate; a separate `prism-jit` crate before Tier 2 exists.
- **Cranelift pinned to `=0.136.2`, fixed settings** (D2). The exact pin keeps the generated IR, `prism dump-clif` and the timing baseline stable until a deliberate upgrade. The ISA is the host's with `opt_level=speed`, the verifier in debug builds only, frame pointers kept, inline stack probes, `use_colocated_libcalls=false`, and `is_pic` on x86-64 only (as `cranelift-jit` requires). Rejected: `opt_level=none` (a small compile-time saving; revisit from the timing numbers), a version range.
- **One code unit per file, one JIT module per load** (D3). A unit is lowered from its file's elaborated MIR alone, so its code depends on the file, its direct imports' ABIs (through its own MIR), the compiler and the target, which keeps per-file object code cacheable and hot reload per file. The units of one load share one `JITModule`, finalized together; `hot-reload` will define a changed file's unit into a new module. Rejected: a module per file (separate executable pages and finalizations), one module with direct cross-file calls (ties code to callees' addresses).
- **Every call goes through the function table** (D4, D26). One pointer-sized slot per function, default generator and `val` accessor, in chunks that never move; code refers to a slot only as the imported data symbol `<symbol>#slot`, resolved by the JIT's symbol lookup, loads it and calls with `call_indirect`, also within a file, and nothing is inlined, so after `hot-reload` repoints a slot every later call, even from a frame of old code, reaches the new code. A slot no load fills holds `prism_rt_unlinked_function`. Runtime functions are called directly; the runtime is never reloaded. The table is owned by the linker and then the `Program`, not a `static`, so a test process can link several programs. Rejected: direct calls within a file, slot addresses as code constants (unrelocatable), a global table.
- **Readable symbols from declaration paths** (D5): `m::name`, `m::<receiver>.name` with the receiver written as the ABI writes its constructor (`lib/team.Team`), `<function>#default(p)`, `m::name#init`, `#get`, `#storage`, `#state` and `#slot`; `closures` will add `#closure<i>` and `monomorphization` type arguments. Tier 2 will escape them for object files. Rejected: a C++-style mangling now; bare receiver names.
- **Scalars natively, the host's C convention** (D6). `Int` is `i64`, `Double` `f64`, `Bool` `i8` (0 or 1, zero-extended at calls as Apple's arm64 convention requires), `Char` `i32`, a reference a pointer; `Unit` and `Nothing` have no representation and are left out of signatures. A caller derives the callee's signature from the call's parameter types and destination, so no ABI lookup is needed. Rejected: Cranelift's `tail` convention, `i32` booleans, a materialized `Unit`.
- **Locals are SSA variables unless borrowed** (D7). A borrowed local lives in a stack slot; every other one is a Cranelift `Variable`, so Cranelift builds SSA. Storage markers and `Read` generate nothing; `SwitchInt` is `brif` or a `Switch`. Rejected: every local in memory. (`codegen-aggregates` puts every struct, enum and tag-byte nullable in memory, `Range` included, and drops the `Range` special case; see below.)
- **References to scalars are supported now** (D8), because every top-level `val` read and every default generator already uses them in MIR. References to aggregates, field borrows and `(&T)?` came with `codegen-aggregates`. Rejected: special-casing `const &VAL`.
- **Unsupported constructs are a compile-time error over every compiled body** (D9). The check runs on each file's MIR before any code is generated and reports one `E6001` per body, naming a reference by its referent and the change that will support the construct; it covers every body, not only those reachable from `main`, which would need a whole-program call graph and would make a program run or fail depending on which functions an edit calls. Rejected: runtime-panicking stubs, compiler panics.
- **Data is never addressed PC-relatively** (D26). Slots, file names, panic details, value storage and the stack limit are non-colocated symbols, since `cranelift-jit`'s memory does not promise ±2 GiB reach.
- **`prism dump-clif`** (D23) prints each function's Cranelift IR, headed by its symbol, for debugging and for the scenarios about generated code; executable fixtures are the main tests, so there are no IR snapshots, which every Cranelift upgrade would churn.
- **Separate `code-generation` and `execution` specs** (D25). This spec holds the backend's contract, which later backend changes extend (`codegen-aggregates` adds layouts); `execution` holds the runtime semantics every tier must share.

Decisions from `codegen-aggregates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **What the backend compiles** (D1, D19). Every non-generic, non-closure body over scalars, `Unit`, structs, enums, nullables, `Range` and references to them, as long as no aggregate holds a `String`, a `Heap<T>` or a function value. Concrete instances of generic types (`Pair<Int, Bool>`) are laid out by substituting their arguments, since the layout code is the same as for any struct; only generic *bodies* wait for `monomorphization`. `classify` looks into fields and payloads and names the first missing construct ("strings", "'Heap' values", "function values and closures"), so a struct is rejected for what it holds; a missing definition is an internal error, since the ABI carries every reachable one (see `compilation-and-abi`).
- **Structs inline, fields by decreasing alignment** (D5): equal alignments keep declaration order, so `struct S { val a: Bool, val n: Int, val b: Bool }` takes 16 bytes instead of C's 24, and the order is stable and documented so dumps and tests can pin it. Programs never see it: construction, evaluation and equality follow declaration order. Rejected: C order (padding with no FFI to justify it; a later C-layout attribute can opt in), Rust's unspecified order.
- **Enums are the smallest tag, then the payloads** (D6): a `u8`, `u16` or `u32` index at offset 0, each variant's payload laid out like a struct starting right after the tag (a `Bool` payload sits at 1), every variant sharing the storage. An enum without variants is uninhabited. Rejected: an `Int`-wide tag, a common payload offset, niche filling, a trailing tag.
- **Nullables are a tag byte, or the null pointer** (D7): `T?` is a `u8` tag (0 for `null`, matching MIR's `Discriminant`) then `T` at its alignment; `(&T)?`, `(&mut T)?` and `Heap<T>?` are one pointer, null being 0, so every optional borrow MIR produces costs one pointer. Rejected: one representation for all nullables, general niche filling now (a later change can add it with no observable effect).
- **Sizes** (D8): `Int`, `Double` and pointers 8, `Char` 4, `Bool` 1; `Unit` and fieldless structs 0 bytes (storage is at least one byte so every value has an address); `Heap<T>` is a pointer now so recursive enums have a layout, though its values stay `E6001`; `String` and function types have no layout until `runtime-strings-and-drops` and `closures` choose theirs.
- **Aggregates live in memory, scalars in variables** (D9). A struct, enum or tag-byte nullable local is a stack slot of its layout; an aggregate argument is used in place, and the return place is the caller's memory. Rejected: scalar replacement of unborrowed aggregates, an optimization with no observable effect that needs a second representation and cannot split overlapping payloads; Tier 2 gets it from LLVM.
- **Passed by address, returned through a caller address** (D10). An aggregate parameter is one pointer and an aggregate result a pointer parameter placed first, so a machine signature never depends on a layout and a function-table slot keeps its signature when a type gains a field. The caller passes a moved operand's own place (dead after the move), copies a `Copy` operand into a fresh slot (so a write through another argument cannot change it), and passes its destination as the result address only when the destination is a whole, unborrowed local no argument is rooted at, else a fresh slot copied afterwards. A layout change still recompiles every function touching the type, which the interface hashes find (see `compilation-and-abi`); what happens to live values is `hot-reload`'s. Rejected: `StructReturn` (its C registers are not needed between Prism functions), multi-value returns and per-field arguments (signatures would follow layouts), small aggregates in registers (a follow-up), callee-allocated results.
- **Places, copies and moves** (D11, D12). A place's address follows its projections (`Deref` loads, `Field` and a tag-byte `Payload` add offsets, a pointer-like `Payload` adds nothing); `Subtype` is lowered only where both sides share a representation. A copy and a move are the same bytes copied (`emit_small_memory_copy`, in its overlapping form when source and destination share a root or go through a dereference), and an `Aggregate` whose memory operands may overlap the destination copies them to temporaries first. Nothing is zeroed or flagged after a move.
- **Equality is expanded inline** (D13): fields in declaration order, enum tags then the current variant's payloads, nullables by presence then payload, references by referent, each scalar as its type compares, so padding and inactive payload bytes are never read and `NaN` and `-0.0` behave. Rejected: `memcmp` (reads padding, wrong for floats), per-type equality functions now, which `runtime-strings-and-drops` may introduce with its glue functions. Expansion is bounded by the type's size, since a type reaching itself needs `Heap<T>`, whose values are rejected.
- **Drops of values that own nothing compile to nothing** (D14, D15): a `Drop` of a supported type is a jump, drop flags stay ordinary `Bool` locals, and `clone()` of such a value is a bitwise copy, so non-`Copy` aggregates of scalars run with their moves checked as they will be later. Rejected: rejecting every non-`Copy` aggregate until drop glue exists.
- **Top-level values build in place** (D17): `#storage` is sized by the layout and the accessor passes its address as `#init`'s result address.
- **`prism dump-layout`** (D18) prints the file's own structs and enums in declaration order, then every other struct, enum and nullable its compiled bodies hold (through fields, payloads and referents, not `Heap<T>`) by rendered name, with sizes, alignments, offsets and tags; a type without a layout is listed with what it waits for. `layout.snap` fixtures pin the layout rules, which `prism dump-clif` (no snapshots) could not. Rejected: layouts inside `dump-clif`, a `--layouts` flag on `dump-mir`.
