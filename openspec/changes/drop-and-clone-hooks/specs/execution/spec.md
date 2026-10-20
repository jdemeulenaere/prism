## ADDED Requirements

### Requirement: Drop Hooks Run Before the Value's Parts
Dropping a value whose type has a drop hook (`ownership`) MUST call the hook with a mutable borrow of the value, and then drop the value's parts as its type's drop does: a struct's fields in declaration order, an enum's payloads of its current variant in order. Hooks MUST run where the drops of the elaborated MIR run: locals in reverse order of creation where their scope ends, temporaries at the end of their statement.

#### Scenario: Locals run their hooks in reverse order
- **WHEN** `struct Noisy { val name: String }` and `operator mut fun Noisy.drop() { println("drop " + name) }` are declared and `fun main() { val a = Noisy("a"); val b = Noisy("b"); println("end") }` runs with `--check-allocations`
- **THEN** it prints `end`, `drop b` and `drop a`, and exits with status 0

#### Scenario: A temporary runs its hook at the end of its statement
- **WHEN** with that `Noisy`, `fun main() { println(Noisy("t").name.length); println("next") }` runs with `--check-allocations`
- **THEN** it prints `1`, `drop t` and `next`

#### Scenario: A hook runs before the hooks of its fields
- **WHEN** with that `Noisy`, `struct Pair { val first: Noisy, val second: Noisy }` and `operator mut fun Pair.drop() { println("drop pair of " + first.name) }` are declared and `fun main() { val p = Pair(Noisy("1"), Noisy("2")) }` runs with `--check-allocations`
- **THEN** it prints `drop pair of 1`, `drop 1` and `drop 2`

#### Scenario: An enum's hook runs before its payloads
- **WHEN** with that `Noisy`, `enum Slot { Full(Noisy), Empty }` and `operator mut fun Slot.drop() { println("drop slot") }` are declared and `fun main() { val a = Slot.Full(Noisy("x")); val b = Slot.Empty }` runs with `--check-allocations`
- **THEN** it prints `drop slot`, `drop slot` and `drop x`

#### Scenario: A hook may change its value before its parts are dropped
- **WHEN** with that `Noisy`, `struct Guard { var inner: Noisy }` and `operator mut fun Guard.drop() { inner = Noisy("replaced") }` are declared and `fun main() { val g = Guard(Noisy("first")) }` runs with `--check-allocations`
- **THEN** it prints `drop first` and `drop replaced`, and exits with status 0

### Requirement: Moved Values Run No Drop Hook
A value moved out of its owner MUST NOT run its hook where the owner's scope ends; it MUST run it where its last owner drops it. A value moved on some paths only MUST run its hook at its scope end exactly on the paths where it was not moved. An assignment MUST run the old value's hook before the new value is written, and only when an old value is present.

#### Scenario: A value moved into a function runs its hook there
- **WHEN** with the `Noisy` of "Locals run their hooks in reverse order", `fun consume(n: Noisy) { println("consume " + n.name) }` is declared and `fun main() { val n = Noisy("a"); consume(n); println("after") }` runs with `--check-allocations`
- **THEN** it prints `consume a`, `drop a` and `after`

#### Scenario: A conditionally moved value runs its hook only if present
- **WHEN** with that `Noisy` and `consume`, `fun f(c: Bool) { val n = Noisy("x"); if (c) consume(n); println("end f") }` is declared and `fun main() { f(true); f(false) }` runs with `--check-allocations`
- **THEN** it prints `consume x`, `drop x`, `end f`, `end f` and `drop x`

#### Scenario: Assigning runs the old value's hook
- **WHEN** with that `Noisy` and `consume`, `fun main() { var n = Noisy("a"); n = Noisy("b"); consume(n); n = Noisy("c"); println("end") }` runs with `--check-allocations`
- **THEN** it prints `drop a`, `consume b`, `drop b`, `end` and `drop c`: assigning `c` over the moved `n` runs no hook

#### Scenario: A returned value runs its hook in the caller
- **WHEN** with that `Noisy`, `fun make(): Noisy { val n = Noisy("m"); println("made"); return n }` is declared and `fun main() { val m = make(); println("got") }` runs with `--check-allocations`
- **THEN** it prints `made`, `got` and `drop m`

### Requirement: Drop Hooks Run Wherever Values Are Dropped
A value with a drop hook MUST run it exactly once when it is dropped as a field or payload of a value without a hook, as a nullable's payload, a list element (in index order when the list is dropped, and when an element is overwritten), a box's contents, a capture a closure owns, or a type-parameter value of a generic instance. Unboxing or removing an element MUST move the value without running its hook.

#### Scenario: List elements run their hooks
- **WHEN** with the `Noisy` of "Locals run their hooks in reverse order", `fun main() { var xs = listOf(Noisy("1"), Noisy("2"), Noisy("3")); xs[0] = Noisy("4"); val second = xs.removeAt(1); println("end") }` runs with `--check-allocations`
- **THEN** it prints `drop 1`, `end`, `drop 2`, `drop 4` and `drop 3`

#### Scenario: Box contents and fields run their hooks
- **WHEN** with that `Noisy`, `struct Holder { val n: Noisy, val tag: Int }` is declared and `fun main() { val h = Heap(Noisy("boxed")); val o: Noisy? = Noisy("maybe"); val none: Noisy? = null; val held = Holder(Noisy("held"), 1); val out = Heap(Noisy("out")).unbox(); println("end") }` runs with `--check-allocations`
- **THEN** it prints `end`, `drop out`, `drop held`, `drop maybe` and `drop boxed`

#### Scenario: A closure's owned capture runs its hook with the closure
- **WHEN** with that `Noisy`, `fun keep(): () -> Unit { val n = Noisy("c"); return { println(n.name) } }` is declared and `fun main() { val f = keep(); f(); println("end") }` runs with `--check-allocations`
- **THEN** it prints `c`, `end` and `drop c`

#### Scenario: An instance runs the hook of its type argument
- **WHEN** with that `Noisy`, `fun <T> discard(x: T) {}` and `fun <T> pick(a: T, b: T, first: Bool): T = if (first) a else b` are declared and `fun main() { discard(Noisy("g")); val p = pick(Noisy("a"), Noisy("b"), true); println("end") }` runs with `--check-allocations`
- **THEN** it prints `drop g`, `drop b`, `end` and `drop a`

### Requirement: A Closure Keeps Only What It Captures by Value
A closure MUST keep, and drop with its environment, only the values it captures by value. A binding of which an owning closure copies only `Copy` paths MUST stay owned by its scope, and MUST run its hook where that scope ends, even while the closure lives on.

#### Scenario: A binding whose field a closure copies is dropped by its scope
- **WHEN** `struct Ticket { val id: Int }` and `operator mut fun Ticket.drop() { println("drop ticket") }` are declared, `fun make(t: Ticket): () -> Int = { t.id }` is declared and `fun main() { val f = make(Ticket(7)); println("made"); println(f()) }` runs with `--check-allocations`
- **THEN** it prints `drop ticket`, `made` and `7`: the closure copied `t.id`, so `t` was dropped when `make` returned

### Requirement: Clone Hooks Run Wherever Values Are Cloned
`x.clone()` of a type with a clone hook MUST call the hook with a shared borrow of `x` and give its result. Cloning a value that holds such a type as a field, payload, nullable payload, list element or box contents MUST clone that part by its hook, in the order the parts are cloned, and an instance's `clone()` of a type-parameter value MUST call the type argument's hook.

#### Scenario: Cloning through a hook
- **WHEN** `struct Ticket : Clone { val id: Int }` and `operator fun Ticket.clone(): Ticket { println("clone " + id.toString()); return Ticket(id + 100) }` are declared and `fun main() { val a = Ticket(1); val b = a.clone(); println(b.id); println(a.id) }` runs with `--check-allocations`
- **THEN** it prints `clone 1`, `101` and `1`

#### Scenario: Parts are cloned by their hooks
- **WHEN** with that `Ticket`, `struct Pass : Clone { val ticket: Ticket, val owner: String }` and `fun <T : Clone> copyOf(x: &T): T = x.clone()` are declared and `fun main() { val p = Pass(Ticket(2), "ada").clone(); println(p.ticket.id); val xs = listOf(Ticket(3), Ticket(4)).clone(); println(xs[1].id); val h = Heap(Ticket(5)).clone(); println(h.id); println(copyOf(&Ticket(6)).id) }` runs with `--check-allocations`
- **THEN** it prints `clone 2`, `102`, `clone 3`, `clone 4`, `104`, `clone 5`, `105`, `clone 6` and `106`

### Requirement: Drop Hooks Release Runtime Resources
A type whose value holds a handle to a runtime resource MUST be able to release it in its drop hook and acquire a new one in its clone hook, so that under `--check-allocations` a program using it reports no live resource, whether its values end in locals, lists, boxes, fields or clones.

#### Scenario: A session type disposes its composition
- **WHEN** a file with `import /prism/composer` declares `struct Session : Clone { val id: Int }`, `fun open(): Session = Session(startComposition { k -> })`, `operator mut fun Session.drop() { disposeComposition(id) }` and `operator fun Session.clone(): Session = open()`, and `fun main() { val a = open(); val b = a.clone(); val all = listOf(open(), open()); val boxed = Heap(open()); println("done") }` runs with `--check-allocations`
- **THEN** it prints `done`, writes nothing to stderr and exits with status 0

#### Scenario: Without the hook the session leaks its composition
- **WHEN** the same file declares no drop or clone hook and no `: Clone` marker, and `fun main() { val a = open(); println("done") }` runs with `--check-allocations`
- **THEN** it prints `done`, stderr is exactly `allocation check: compositions still live at exit: 1` and a newline, and the exit status is 102

### Requirement: A Runtime Error in a Hook Ends the Program
A runtime error raised in a hook MUST end the program as any runtime error does (status 101, located in the hook), and no further drop or hook MUST run. Ending the program with `exitProcess` MUST run no hook.

#### Scenario: A hook that divides by zero
- **WHEN** with the `Noisy` of "Locals run their hooks in reverse order", `fun zero(): Int = 0`, `struct Bomb { val n: Int }` and `operator mut fun Bomb.drop() { println(n / zero()) }` are declared and `fun main() { val a = Noisy("a"); val b = Bomb(1); println("end") }` runs
- **THEN** stdout is `end`, stderr holds `panic: division by zero` located at `n / zero()` in the hook, and the exit status is 101: `drop a` is never printed

#### Scenario: Exiting runs no hook
- **WHEN** with that `Noisy`, `fun main() { val n = Noisy("n"); println("bye"); exitProcess(3) }` runs with `import /prism/system`
- **THEN** stdout is `bye` and the exit status is 3

### Requirement: The Allocation Check Drops Top-Level Values
Under `--check-allocations`, when `main` returns, the check MUST drop each initialized top-level value in reverse order of initialization, running its hooks, before it reports, and drop in turn any value initialized meanwhile. Reading a top-level value it dropped MUST raise the runtime error `top-level value '<name>' read after the allocation check dropped it`. Without the option, no top-level value is dropped and no hook of one runs.

#### Scenario: A top-level value's hook runs under the check
- **WHEN** with the `Noisy` of "Locals run their hooks in reverse order", `val GLOBAL: Noisy = Noisy("global")` and `fun main() { println(GLOBAL.name) }` run with `--check-allocations`
- **THEN** it prints `global` and `drop global`, and exits with status 0

#### Scenario: Without the check a top-level value runs no hook
- **WHEN** the same program runs without `--check-allocations`
- **THEN** it prints only `global` and exits with status 0

#### Scenario: A hook reading a top-level value the check dropped
- **WHEN** with that `Noisy`, `struct Reader { val n: Int }`, `operator mut fun Reader.drop() { println(A.name) }`, `val B: Reader = Reader(1)`, `val A: Noisy = Noisy("a")` and `fun main() { println(B.n); println(A.name) }` run with `--check-allocations`
- **THEN** stdout holds `1`, `a` and `drop a`, stderr holds `panic: top-level value 'A' read after the allocation check dropped it` located at `A.name` in the hook, and the exit status is 101

## MODIFIED Requirements

### Requirement: Values Are Freed Where Their Owner's Scope Ends
Each drop that the elaborated MIR runs MUST free, at that point, the memory the dropped value owns, deterministically and without a garbage collector or reference counting, so that every allocation a value owns is freed exactly once: where its owner's scope ends, where it is overwritten by an assignment or a write through a reference, or by the code it was moved to. A top-level value MUST never be dropped while the program runs; only the allocation check drops top-level values, after `main` returns.

#### Scenario: Deterministic scope exit cleanup
- **WHEN** `fun main() { var total = 0; for (i in 0..<1000) { val s = i.toString() + "x"; total += s.length }; println(total) }` runs with `--check-allocations`
- **THEN** it prints `3890`, writes nothing to stderr and exits with status 0

#### Scenario: A conditionally moved value is dropped only if present
- **WHEN** `fun take(s: String): Int = s.length` and `fun f(c: Bool) { val s = 1.toString(); if (c) println(take(s)) }` are declared and `fun main() { f(true); f(false) }` runs with `--check-allocations`
- **THEN** it prints `1`, writes nothing to stderr and exits with status 0

#### Scenario: Writing through a reference drops the old referent
- **WHEN** `fun set(r: &mut String, v: String) { *r = v }` is declared and `fun main() { var s = 1.toString(); set(&mut s, 2.toString()); s = 3.toString(); println(s) }` runs with `--check-allocations`
- **THEN** it prints `3`, writes nothing to stderr and exits with status 0

#### Scenario: Argument temporary dropped at the end of the statement
- **WHEN** `fun make(): String = 5.toString()` and `fun greet(n: &String): Int = n.length` are declared and `fun main() { println(greet(make())); println(greet(make())) }` runs with `--check-allocations`
- **THEN** it prints `1` twice, writes nothing to stderr and exits with status 0

#### Scenario: Top-level values are kept
- **WHEN** `val NAME: String = 4.toString() + "2"` is declared and `fun main() { println(NAME); println(NAME.length) }` runs with `--check-allocations`
- **THEN** it prints `42` and then `2`, writes nothing to stderr and exits with status 0

### Requirement: Cloning Values That Own Storage
`x.clone()` of a value that owns storage MUST give a value that shares no owned memory with it, so that changing or dropping either leaves the other unchanged. When neither its type nor any of its parts has a clone hook (`ownership`), the clone MUST be equal to `x` and MUST run no user code; otherwise the hooks build those parts.

#### Scenario: String clone is independent
- **WHEN** `fun main() { var a = 1.toString(); val b = a.clone(); a = 2.toString(); println(a); println(b) }` runs with `--check-allocations`
- **THEN** it prints `2` and then `1`, writes nothing to stderr and exits with status 0

#### Scenario: Struct clone clones each field
- **WHEN** `struct User : Clone { var name: String, val nick: String? }` is declared and `fun main() { var a = User(1.toString(), 2.toString()); val b = a.clone(); a.name = 3.toString(); println(a.name); println(b.name); println(b.nick == a.nick) }` runs with `--check-allocations`
- **THEN** it prints `3`, `1` and `true`, writes nothing to stderr and exits with status 0

### Requirement: Instances Drop and Clone Type-Parameter Values
In an instance, a value of a type parameter's type MUST be dropped, moved and copied as a value of the type argument is: dropping it runs the argument's drop glue and hooks, a move transfers its storage, and a `clone()` under a `Clone` bound builds a value that shares no storage with the original, through the argument's clone hooks if it has any. A value whose argument owns nothing and has no drop hook in it MUST drop without running any code.

#### Scenario: Dropping and cloning strings through generic code
- **WHEN** `struct Pair<A, B>(val a: A, val b: B)`, `fun <T> pick(a: T, b: T, first: Bool): T = if (first) a else b`, `fun <T> discard(x: T) {}` and `fun <T : Clone> both(x: &T): Pair<T, T> = Pair(x.clone(), x.clone())` are declared and `fun main() { println(pick("a", "b", false)); discard("c" + "d"); val p = both(&("e" + "f")); println(p.a + p.b) }` runs under `prism run --check-allocations`
- **THEN** the program prints `b` and `efef`, reports no live allocation and no double free, and exits with status 0

#### Scenario: Generic code over structs that own strings
- **WHEN** `struct User : Clone { val name: String }`, `struct Box<T>(val v: T)` and `fun <T> unwrap(b: Box<T>): T { val (v) = b; return v }` are declared and `fun main() { val u = unwrap(Box(User("ada"))); println(u.name); val c = unwrap(Box(Box(u.clone()))); println(c.v.name) }` runs under `prism run --check-allocations`
- **THEN** the program prints `ada` twice, reports no live allocation and exits with status 0
