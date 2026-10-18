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

### Requirement: Top-Level Values Have Storage and an Accessor
Each top-level `val` MUST have storage in its file's code unit and an accessor that initializes the storage on its first call, as the `execution` spec defines, and returns its address. A read of a top-level `val`, in its own file or another, MUST call the accessor through the function table.

#### Scenario: Reading an imported top-level value
- **WHEN** `lib/config.prism` declares `public val LIMIT: Int = 10`, and `app/main.prism` imports `/lib/config` and declares `fun main() { println(LIMIT + 1) }`
- **THEN** `prism run app/main.prism` prints `11`

### Requirement: Unsupported Constructs Are Rejected Before Code Generation
Before generating any code, the backend MUST check every body it compiles and report `E6001` (`UnsupportedByBackend`) for the first construct of each body that it does not support, naming the construct. An unsupported parameter or return type MUST be reported at the declaration, anything else at the first statement or terminator using it, in block order. A rejected program MUST NOT run. `prism check` MUST NOT report `E6001`.

#### Scenario: A struct is rejected
- **WHEN** `struct P { val x: Int }` and `fun main() { val p = P(1); println(p.x) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at `P(1)`, naming structs, prints nothing to stdout and exits with status 1

#### Scenario: Unsupported constructs in several forms
- **WHEN** a program's `main` runs `println("hi")`, or `val n: Int? = null`, or `val f = { 1 }`, or `println(id(1))` for `fun <T> id(x: T): T = x`, or the program declares `compose fun Label() {}` or `fun first(s: String): Int = 1`
- **THEN** `prism run` reports `E6001` naming strings at `"hi"`, nullable values at `null`, closures at `{ 1 }`, calls of generic functions at `id(1)`, composable functions at `Label`, and strings at the declaration of `first`, and runs nothing

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

## Design Rationale

Decisions from `codegen-core` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Two crates** (D1). `crates/prism-codegen` holds everything that knows Cranelift: the support and entry checks (`supported.rs`, `entry.rs`), symbols (`symbols.rs`), representation and signatures (`representation.rs`), lowering of one body (`lower.rs`), one file's code unit (`unit.rs`), the function table and linker (`link.rs`), program execution (`program.rs`) and the IR dump (`dump.rs`). `crates/prism-runtime` holds the `extern "C"` functions generated code calls and depends on nothing, so Tier 2 can link it without Cranelift. Rejected: the runtime inside the codegen crate; a separate `prism-jit` crate before Tier 2 exists.
- **Cranelift pinned to `=0.136.2`, fixed settings** (D2). The exact pin keeps the generated IR, `prism dump-clif` and the timing baseline stable until a deliberate upgrade. The ISA is the host's with `opt_level=speed`, the verifier in debug builds only, frame pointers kept, inline stack probes, `use_colocated_libcalls=false`, and `is_pic` on x86-64 only (as `cranelift-jit` requires). Rejected: `opt_level=none` (a small compile-time saving; revisit from the timing numbers), a version range.
- **One code unit per file, one JIT module per load** (D3). A unit is lowered from its file's elaborated MIR alone, so its code depends on the file, its direct imports' ABIs (through its own MIR), the compiler and the target, which keeps per-file object code cacheable and hot reload per file. The units of one load share one `JITModule`, finalized together; `hot-reload` will define a changed file's unit into a new module. Rejected: a module per file (separate executable pages and finalizations), one module with direct cross-file calls (ties code to callees' addresses).
- **Every call goes through the function table** (D4, D26). One pointer-sized slot per function, default generator and `val` accessor, in chunks that never move; code refers to a slot only as the imported data symbol `<symbol>#slot`, resolved by the JIT's symbol lookup, loads it and calls with `call_indirect`, also within a file, and nothing is inlined, so after `hot-reload` repoints a slot every later call, even from a frame of old code, reaches the new code. A slot no load fills holds `prism_rt_unlinked_function`. Runtime functions are called directly; the runtime is never reloaded. The table is owned by the linker and then the `Program`, not a `static`, so a test process can link several programs. Rejected: direct calls within a file, slot addresses as code constants (unrelocatable), a global table.
- **Readable symbols from declaration paths** (D5): `m::name`, `m::<receiver>.name` with the receiver written as the ABI writes its constructor (`lib/team.Team`), `<function>#default(p)`, `m::name#init`, `#get`, `#storage`, `#state` and `#slot`; `closures` will add `#closure<i>` and `monomorphization` type arguments. Tier 2 will escape them for object files. Rejected: a C++-style mangling now; bare receiver names.
- **Scalars natively, the host's C convention** (D6). `Int` is `i64`, `Double` `f64`, `Bool` `i8` (0 or 1, zero-extended at calls as Apple's arm64 convention requires), `Char` `i32`, a reference a pointer; `Unit` and `Nothing` have no representation and are left out of signatures. A caller derives the callee's signature from the call's parameter types and destination, so no ABI lookup is needed. Rejected: Cranelift's `tail` convention, `i32` booleans, a materialized `Unit`.
- **Locals are SSA variables unless borrowed** (D7). A borrowed local lives in a stack slot; every other one is a Cranelift `Variable`, so Cranelift builds SSA. A `Range` local is three variables (scalar replacement, which `codegen-aggregates` can generalize). Storage markers and `Read` generate nothing; `SwitchInt` is `brif` or a `Switch`. Rejected: every local in memory.
- **References to scalars are supported now** (D8), because every top-level `val` read and every default generator already uses them in MIR. References to aggregates, field borrows and `(&T)?` stay with `codegen-aggregates`. Rejected: special-casing `const &VAL`.
- **Unsupported constructs are a compile-time error over every compiled body** (D9). The check runs on each file's MIR before any code is generated and reports one `E6001` per body, naming a reference by its referent and the change that will support the construct; it covers every body, not only those reachable from `main`, which would need a whole-program call graph and would make a program run or fail depending on which functions an edit calls. Rejected: runtime-panicking stubs, compiler panics.
- **Data is never addressed PC-relatively** (D26). Slots, file names, panic details, value storage and the stack limit are non-colocated symbols, since `cranelift-jit`'s memory does not promise ±2 GiB reach.
- **`prism dump-clif`** (D23) prints each function's Cranelift IR, headed by its symbol, for debugging and for the scenarios about generated code; executable fixtures are the main tests, so there are no IR snapshots, which every Cranelift upgrade would churn.
- **Separate `code-generation` and `execution` specs** (D25). This spec holds the backend's contract, which later backend changes extend (`codegen-aggregates` adds layouts); `execution` holds the runtime semantics every tier must share.
