# Code Generation Specification Delta

## ADDED Requirements

### Requirement: Function Values Are a Descriptor and an Environment
A value of a function type, composable or not, MUST be 16 bytes with the alignment of a pointer: the address of its closure's descriptor at offset 0, never null, and the address of its environment at offset 8, null when the closure captures nothing. It MUST be held, passed and returned in memory like a struct, and a nullable function value MUST be laid out with a tag byte.

#### Scenario: Layout of a struct holding a function value
- **WHEN** `prism dump-layout main.prism` runs on a file declaring `struct Task { val run: () -> Unit, val id: Int }` and `fun later(f: (() -> Unit)?) {}`
- **THEN** it shows `Task` with size 24 and alignment 8, `run` at offset 0 and `id` at 16, and `(() -> Unit)?` with size 24, its tag at offset 0 and its payload at 8

#### Scenario: Signature of a function taking a function value
- **WHEN** `fun call(f: () -> Int): Int = f()` is declared and `prism dump-clif main.prism` runs
- **THEN** `call` takes one machine value, the address of `f`, and returns a 64-bit integer

### Requirement: Each Closure Has a Descriptor
Each compiled closure body MUST have one descriptor, static data of its code unit named `<closure symbol>#descriptor`, holding the address of the function-table slot of the closure's code and the environment's drop entry, which MUST be null for a closure that captures by borrow. Every value the closure builds MUST point at that descriptor.

#### Scenario: Building a closure names its descriptor
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `prism dump-clif main.prism` runs on a file holding `fun main() { val a = 1; println(run { a + 1 }) }`
- **THEN** the body of `main` writes the address of the data symbol `main::main#closure0#descriptor` into the closure value it passes to `run`

### Requirement: Closure Code Is a Function With Its Own Slot
Each closure body outside a generic body MUST be compiled as a function of its file's code unit with its own function-table slot, named `<parent symbol>#closure<i>` after the body it is lowered in and its index there. Its signature MUST be its body's, with the environment's address after the result address and before the parameters.

#### Scenario: Closure functions in the IR dump
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `prism dump-clif main.prism` runs on a file holding `fun main() { val a = 1; println(run { a + 1 }) }`
- **THEN** it prints the functions `main::run`, `main::main` and `main::main#closure0`, the last taking one pointer and returning a 64-bit integer, and `prism run main.prism` prints `2`

#### Scenario: Symbols of nested and default closures
- **WHEN** `fun run(f: () -> Int): Int = f()` and `fun pick(n: Int, f: () -> Int = { 3 }): Int = n + f()` are declared and `prism dump-clif main.prism` runs on a file holding `fun main() { println(run { run { 1 } }); println(pick(1)) }`
- **THEN** it prints the functions `main::main#closure0`, `main::main#closure0#closure0` and `main::pick#default(f)#closure0`, and `prism run main.prism` prints `1` and then `4`

### Requirement: Calls of Function Values Go Through the Descriptor's Slot
A call of a function value MUST load the slot address from the value's descriptor and the code from that slot, and call it indirectly with the value's environment address, so that a repointed slot is followed by every value built before. No call of a function value MUST be inlined.

#### Scenario: A call of a function value in the IR dump
- **WHEN** `fun call(f: () -> Int): Int = f()` is declared and `prism dump-clif main.prism` runs
- **THEN** the body of `call` loads the descriptor from `f`, the slot address from the descriptor and the code from the slot, and calls it indirectly passing the environment address loaded from `f`

#### Scenario: A repointed closure slot is followed
- **WHEN** a program declares `fun run(f: () -> Int): Int = f()` and `fun main() { println(run { 1 }); println(run { 2 }) }`, and before `main` runs the slot of `main::main#closure0` is pointed at the code of `main::main#closure1`
- **THEN** the program prints `2` twice

### Requirement: Environments of Borrowing Closures Hold Pointers in the Creating Frame
The environment of a closure that captures by borrow MUST hold the address of each captured place, one pointer per capture in capture order, in stack storage of the function that builds the closure. Building or calling such a closure MUST NOT allocate.

#### Scenario: A writing closure allocates nothing
- **WHEN** `fun repeat(n: Int, f: () -> Unit) { var i = 0; while (i < n) { f(); i += 1 } }` is declared and `prism dump-clif main.prism` runs on a file holding `fun main() { var count = 0; repeat(3) { count += 1 }; println(count) }`
- **THEN** no function of the file calls the runtime's allocation function, and `prism run main.prism` prints `3`

#### Scenario: A closure built in an earlier iteration still runs
- **WHEN** `fun main() { val a = 10; var keep: () -> Int = { 0 }; var i = 0; while (i < 3) { val g = { a + 1 }; if (i == 0) keep = g; i += 1 }; println(keep()) }` runs with `--check-allocations`
- **THEN** it prints `11`, reports no live allocation and exits with status 0

### Requirement: Dropping a Function Value Runs Its Drop Entry
Dropping a function value, alone or as a part of a struct, enum or nullable, MUST run the drop glue of its function type, `<module>#drop(<function type>)`, which MUST call the descriptor's drop entry with the environment's address when the entry is not null and do nothing otherwise.

#### Scenario: Drop glue of a function type in the IR dump
- **WHEN** `prism dump-clif main.prism` runs on a file declaring `struct Task { val run: () -> Int, val name: String }` and `fun f(t: Task) {}`
- **THEN** after the file's functions it prints `function main#drop(() -> Int)` and `function main#drop(main::Task)`, which calls it for `run`

#### Scenario: Dropping a struct holding a function value
- **WHEN** `struct Task { val run: () -> Int, val name: String }` is declared and `fun main() { val t = Task({ 7 }, "a" + "b"); println(t.run()); println(t.name) }` runs with `--check-allocations`
- **THEN** it prints `7` and then `ab`, reports no live allocation and exits with status 0

### Requirement: Closures of Instances Are Compiled in the Instance's Unit
The closures lowered inside a generic body MUST be compiled as part of each instance of that body, in the instance's code unit, each with its own slot and named `<instance symbol>#closure<i>`.

#### Scenario: A closure in an instance
- **WHEN** `fun run(g: () -> Int): Int = g()` and `fun <T : Copy> count(x: T, f: (T) -> Int): Int = run { f(x) + 1 }` are declared and `prism dump-clif --instances main.prism` runs on a file holding `fun main() { println(count(2) { n -> n * 10 }) }`
- **THEN** it prints the functions `main::count<Int>` and `main::count<Int>#closure0`, and `prism run main.prism` prints `21`

## MODIFIED Requirements

### Requirement: Tier 1 Compiles Elaborated MIR in Memory
`prism run` MUST compile the elaborated MIR of every file of the program into native code in the memory of the `prism` process, without invoking any external program such as a linker or an assembler. Every body MUST be compiled, closure bodies included, except generic bodies and the closures inside them, which are compiled only as the instances the program calls.

#### Scenario: Running a scalar program
- **WHEN** `fun square(x: Int): Int = x * x` and `fun main() { println(square(7)) }` are declared in `main.prism` and `prism run main.prism` runs
- **THEN** it prints `49` and exits with status 0, and no other process is started

#### Scenario: An uncalled generic function is not compiled
- **WHEN** `fun <T> keep(x: T): T = x` and `fun main() { println(1) }` are declared and `prism run main.prism` runs
- **THEN** it prints `1` and exits with status 0

#### Scenario: A called generic function is compiled per instance
- **WHEN** `fun <T> keep(x: T): T = x` and `fun main() { println(keep(1)); println(keep(true)) }` are declared and `prism run main.prism` runs
- **THEN** it prints `1` and `true` and exits with status 0

#### Scenario: A closure is compiled
- **WHEN** `fun twice(f: (Int) -> Int, x: Int): Int = f(f(x))` and `fun main() { println(twice({ n -> n * 3 }, 2)) }` are declared and `prism run main.prism` runs
- **THEN** it prints `18` and exits with status 0

### Requirement: Constructs the Backend Lacks Are Rejected Before Code Generation
Before generating any code, the backend MUST check every body it compiles, each instance included, and report `E6001` (`UnsupportedByBackend`) for the first construct of each body that it does not support, naming the construct. An unsupported parameter or return type MUST be reported at the declaration, anything else at the first statement or terminator using it, in block order; for an instance, at that location in the generic's source, once however many instances share it. A rejected program MUST NOT run. `prism check` MUST NOT report `E6001`.

#### Scenario: A struct holding a composable closure is rejected
- **WHEN** `compose fun Label() {}`, `struct P { val content: compose () -> Unit }` and `fun main() { val p = P({ Label() }); println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at `{ Label() }`, naming composable functions, with a note naming the `compose-transform` change, prints nothing to stdout and exits with status 1

#### Scenario: A recursive enum runs
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun main() { val e = Expr.Add(Heap(Expr.Literal(1)), Heap(Expr.Literal(2))); println(1) }` are declared
- **THEN** `prism run --check-allocations main.prism` prints `1`, writes nothing to stderr and exits with status 0

#### Scenario: Unsupported constructs in several forms
- **WHEN** a program's `main` runs `val c: compose () -> Unit = {}`, or the program declares `compose fun Label() {}` or `fun first(w: Widget): Int = 1` with no declaration of `Widget`
- **THEN** `prism run` reports `E6001` naming composable functions at `{}` and at `Label`, and values of unknown type `Widget` at the declaration of `first`, and runs nothing

#### Scenario: One error per body
- **WHEN** `fun f(a: Widget, b: Gadget): Int = 1` and `fun g(c: Widget): Int = 2` are declared, with no declaration of `Widget` or `Gadget`, with a `main` that calls neither
- **THEN** `prism run` reports exactly two `E6001`, one at the declaration of `f`, naming `Widget`, and one at the declaration of `g`

#### Scenario: An unsupported construct in a generic body
- **WHEN** `lib/run.prism` declares `public fun <T> later(x: T): Int { val f: compose () -> Unit = {}; return 1 }`, and `app/main.prism` imports `/lib/run` and declares `fun main() { println(later(1)); println(later(true)) }`
- **THEN** `prism check app/main.prism` passes, and `prism run app/main.prism` reports exactly one `E6001`, at `{}` in `lib/run.prism`, naming composable functions, and runs nothing

### Requirement: Cranelift IR Dump
`prism dump-clif <file.prism>` MUST print, as deterministic text, the Cranelift IR of each function the backend generates for the file, in body order, each headed by its symbol, followed by the file's drop, clone and equality glue, each headed by `<module>#drop(<type>)`, `<module>#clone(<type>)` or `<module>#equals(<type>)`, in order of their names. Like `prism dump-mir --elaborated`, it MUST compile the file alone, against its direct imports' ABIs, so it prints no instance: a call of an instance appears as a load from its slot. When the file has errors of any stage, including `E6001`, it MUST report them, print no IR, and exit with a failure status.

#### Scenario: Dumping a file's IR
- **WHEN** `prism dump-clif main.prism` runs twice on a file that `prism run` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: Dumping a file the backend rejects
- **WHEN** `prism dump-clif main.prism` runs on a file holding `compose fun Label() {}`
- **THEN** it reports `E6001`, prints no IR, and exits with a failure status

### Requirement: Layout Dump
`prism dump-layout <file.prism>` MUST print, as deterministic text, the layout of each struct and enum the file declares, in declaration order, then of every other struct, enum and nullable type that a compiled body of the file holds, by rendered name: size, alignment, and the offset of every field, tag and payload. A type without a layout yet MUST be listed with the construct it waits for. With type or borrow errors it MUST report them, print nothing and exit with a failure status.

#### Scenario: Dumping a file's layouts
- **WHEN** `prism dump-layout main.prism` runs twice on a file that `prism check` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: A type without a layout yet
- **WHEN** `prism dump-layout main.prism` runs on a file declaring `struct Task { val run: () -> Unit }`, `struct Box<T>(val v: T)` and `struct Odd { val w: Widget }`, with no declaration of `Widget`
- **THEN** it lists `Task` with size 16 and `run` at offset 0, `Box` as laid out per instance, and `Odd` as having no layout because of values of unknown type `Widget`, and exits with status 0

#### Scenario: Dumping a file with errors
- **WHEN** `prism dump-layout main.prism` runs on `struct P { val x: Int }` and `fun f(): Int = nope`
- **THEN** it reports `E4001`, prints no layout, and exits with a failure status

## REMOVED Requirements

### Requirement: Aggregates Holding Heap or Function Values Are Rejected
**Reason**: `heap-boxes` makes boxes run and this change makes function values run, so no struct, enum or nullable is rejected for what it holds, except a value of unknown type.
**Migration**: "Function Values Are a Descriptor and an Environment" and "Dropping a Function Value Runs Its Drop Entry" lay out and drop aggregates holding function values; `heap-boxes` does the same for boxes. Whatever narrower requirement `heap-boxes` leaves in this one's place (naming function values only) is removed with it.
