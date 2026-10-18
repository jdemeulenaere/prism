# Code Generation Delta

## ADDED Requirements

### Requirement: Composables Take the Composer and Change Words
The machine signature of a composable function, and the signature of the code of a composable closure, MUST be that of its declared parameters followed by the composer and its change words, each a 64-bit integer. A call of a composable function value MUST pass the environment, the arguments, the composer and the change words, in that order.

#### Scenario: Signature of a composable
- **WHEN** `compose fun Label(n: Int) {}` is declared and `prism dump-clif main.prism` runs
- **THEN** `main::Label` takes three 64-bit integers: `n`, the composer and one change word

#### Scenario: A call of a composable function value
- **WHEN** `compose fun Column(content: compose () -> Unit) { content() }` is declared and `prism dump-clif main.prism` runs
- **THEN** the call of `content` in `main::Column` passes the environment, then `Column`'s composer and a change word

### Requirement: Composer Operations Call the Compose Runtime
Each composer operation of composed MIR MUST be a direct call of the compose runtime entry point that implements it, by symbol, with the machine signature that the runtime's table lists. A value given to a slot MUST be passed by address and moved into the runtime.

#### Scenario: A restart group in the IR dump
- **WHEN** `compose fun Badge(n: Int) { println(n) }` is declared and `prism dump-clif main.prism` runs
- **THEN** `main::Badge` calls `prism_compose_start_restart_group` first and `prism_compose_end_restart_group` before each return

### Requirement: Slot Type Records
Each code unit MUST define one slot type record per type that its composed MIR keeps or remembers, named `<unit>#slot-type(<type>)`: the type's 256-bit type fingerprint, its size and alignment, the address of its drop glue or null when it needs no drop, the address of its equality glue, and its name as the slot dump prints it.

#### Scenario: Records of a unit
- **WHEN** `struct Point : Copy { val x: Int }`, `compose fun Badge(n: Int) {}` and `compose fun P(n: Int) { val p = remember { Point(n) } }` are declared and `prism dump-clif main.prism` runs
- **THEN** it defines the data `main#slot-type(Int)` and `main#slot-type(main::Point)`, once each

### Requirement: Kept Values Are Compared by Equality Glue
Every type with a slot type record MUST have equality glue taking the two values' addresses and returning a `Bool`, scalars included, that gives `==` of the type. The equality glue of a function type MUST compare two values as equal exactly when they have the same descriptor, its environment equality entry is not null, and the entry finds their environments equal.

#### Scenario: A kept value that is never equal to itself
- **WHEN** `struct D : Copy { val v: Double }`, `compose fun Show(d: D) { println("show") }`, `fun screen(v: Double): compose () -> Unit = { Show(D(v)) }` and `fun main() { val nan = 0.0 / 0.0; val c = composition(screen(nan)); c.setContent(screen(nan)); c.setContent(screen(1.0)); c.setContent(screen(1.0)) }` run
- **THEN** `show` is printed three times: on both passes with `NaN`, and on the first pass with `1.0`

### Requirement: Keepable Closures Compare and Duplicate Their Environments
A closure that captures nothing or owns only `Copy` captures MUST have an environment equality entry, comparing two environments' captures with `==` in capture order. When such a closure's environment is allocated, it MUST also have an environment duplicate entry returning a new allocation holding a copy of the captures. Every other closure MUST have neither.

#### Scenario: Entries of closures
- **WHEN** `fun keep(f: escaping () -> Unit) {}` and `fun main() { val a = 1; val b = 2; keep { println(a + b) }; val s = "x" + "y"; keep { println(s) } }` are declared and `prism dump-clif main.prism` runs
- **THEN** it prints `main::main#closure0#equals-environment` and `main::main#closure0#duplicate-environment`, and neither for `main::main#closure1`

### Requirement: Intrinsic Structs Are Laid Out by the Compiler
`Composition` and `RecomposeScope` MUST each be one 64-bit word holding a runtime handle. The drop glue of `Composition` MUST dispose the composition it names, and `==` of either MUST compare the handles.

#### Scenario: Layout of a composition
- **WHEN** `prism dump-layout main.prism` runs on a file declaring `struct Screen { val c: Composition, val s: RecomposeScope }` and `fun f(s: Screen) {}`
- **THEN** it shows `Screen` with size 16 and alignment 8, and `prism/compose/runtime::Composition` and `prism/compose/runtime::RecomposeScope` with size 8

## MODIFIED Requirements

### Requirement: Constructs the Backend Lacks Are Rejected Before Code Generation
Before generating any code, the backend MUST check every body it compiles and report `E6001` (`UnsupportedByBackend`) for the first construct of each body that it does not support, naming the construct. An unsupported parameter or return type MUST be reported at the declaration, anything else at the first statement or terminator using it, in block order. A rejected program MUST NOT run. `prism check` MUST NOT report `E6001`.

#### Scenario: A struct is rejected
- **WHEN** `struct P { val w: Widget }`, with no declaration of `Widget`, and `fun use(p: P): Int = 1` and `fun main() { println(1) }` are declared
- **THEN** `prism check main.prism` passes, and `prism run main.prism` reports `E6001` at the declaration of `use`, naming values of unknown type `Widget`, prints nothing to stdout and exits with status 1

#### Scenario: A recursive enum runs
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }` and `fun main() { val e = Expr.Add(Heap(Expr.Literal(1)), Heap(Expr.Literal(2))); println(1) }` are declared
- **THEN** `prism run --check-allocations main.prism` prints `1`, writes nothing to stderr and exits with status 0

#### Scenario: Unsupported constructs in several forms
- **WHEN** a program declares `fun first(w: Widget): Int = 1`, or `fun second(): Int { val g: Gadget? = null; return 1 }`, with no declaration of `Widget` or `Gadget`
- **THEN** `prism run` reports `E6001` naming values of unknown type `Widget` at the declaration of `first`, and `Gadget` at the statement assigning `null`, and runs nothing

#### Scenario: Composables are compiled
- **WHEN** a program's `main` runs `val c: compose () -> Unit = {}`, or the program declares `compose fun Label() {}` and `fun main() { println(1) }`
- **THEN** `prism run` reports no `E6001`, and the second form prints `1`

#### Scenario: One error per body
- **WHEN** `fun f(a: Widget, b: Gadget): Int = 1` and `fun g(c: Widget): Int = 2` are declared, with no declaration of `Widget` or `Gadget`, with a `main` that calls neither
- **THEN** `prism run` reports exactly two `E6001`, one at the declaration of `f`, naming `Widget`, and one at the declaration of `g`

### Requirement: Cranelift IR Dump
`prism dump-clif <file.prism>` MUST print, as deterministic text, the Cranelift IR of each function the backend generates for the file, in body order, followed by the file's glue in order of names, each headed by its symbol. Like `prism dump-mir --composed`, it MUST compile the file alone, against its direct imports' ABIs, so it prints no instance. When the file has errors of any stage, including `E6001`, it MUST report them, print no IR, and exit with a failure status.

#### Scenario: Dumping a file's IR
- **WHEN** `prism dump-clif main.prism` runs twice on a file that `prism run` accepts
- **THEN** it prints the same text both times and exits with status 0

#### Scenario: Dumping a file the backend rejects
- **WHEN** `prism dump-clif main.prism` runs on a file holding `fun first(w: Widget): Int = 1`, with no declaration of `Widget`
- **THEN** it reports `E6001`, prints no IR, and exits with a failure status

### Requirement: The Backend Checks Each Instance
The backend MUST check each instance it compiles as it checks every other body, closures of the instance included, and MUST report an unsupported construct of an instance at its location in the generic's source, once however many instances share it.

#### Scenario: An unsupported construct in a generic body
- **WHEN** `lib/run.prism` declares `public fun <T> later(x: T): Int { val g: Widget? = null; return 1 }`, with no declaration of `Widget`, and `app/main.prism` imports `/lib/run` and declares `fun main() { println(later(1)); println(later(true)) }`
- **THEN** `prism check app/main.prism` passes, and `prism run app/main.prism` reports exactly one `E6001`, at `null` in `lib/run.prism`, naming values of unknown type `Widget`, and runs nothing

### Requirement: Each Closure Has a Descriptor
Each compiled closure body MUST have one descriptor, static data of its code unit named `<closure symbol>#descriptor`, of four words: the address of the function-table slot of the closure's code; the environment's drop entry, the address of its `#drop-environment` function when its environment is allocated and null when the closure only borrows or its environment is held inline; and its environment equality and duplicate entries ("Keepable Closures Compare and Duplicate Their Environments"), or null. Every value the closure builds MUST point at that descriptor.

#### Scenario: Building a closure names its descriptor
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `prism dump-clif main.prism` runs on a file holding `fun main() { val a = 1; println(run { a + 1 }) }`
- **THEN** the body of `main` writes the address of the data symbol `main::main#closure0#descriptor` into the closure value it passes to `run`
