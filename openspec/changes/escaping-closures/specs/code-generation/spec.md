# Code Generation Specification Delta

## ADDED Requirements

### Requirement: Owned Environments Are One Allocation
A closure that owns at least one capture, unless its environment is held inline, MUST allocate its environment once where it is built, with exactly the size and alignment of the environment's layout: each owned capture by value and each borrowed capture as a pointer, laid out like a struct's fields. Calling the closure MUST NOT allocate.

#### Scenario: An owning closure allocates once
- **WHEN** `fun make(s: String): () -> Int = { s.length }` is declared and `prism dump-clif main.prism` runs on a file holding it and `fun main() { val g = make("abc"); println(g() + g()) }`
- **THEN** the body of `make` calls the runtime's allocation function once and `main::make#closure0` calls none, and `prism run --check-allocations main.prism` prints `6`, reports no live allocation and exits with status 0

### Requirement: Owned Environments Are Freed by Their Drop Function
Each closure with an environment that is allocated MUST have a function `<closure symbol>#drop-environment` in its code unit, named by its descriptor's drop entry, that drops each owned capture needing a drop, in capture order, and then frees the environment. Dropping the function value MUST run it exactly once.

#### Scenario: The environment's drop function in the IR dump
- **WHEN** `fun make(s: String, n: Int): () -> Int = { s.length + n }` is declared and `prism dump-clif main.prism` runs
- **THEN** it prints the function `main::make#closure0#drop-environment`, which drops a `String` and calls the runtime's free function, and the data `main::make#closure0#descriptor` names it as its drop entry

### Requirement: Small Owned Environments Are Held in the Function Value
When an owned environment's layout fits in one pointer-sized word and none of its captures needs a drop, the function value MUST hold the environment's bytes in its environment word, its descriptor's drop entry MUST be null, and building it MUST NOT allocate.

#### Scenario: A handler capturing one handle allocates nothing
- **WHEN** `struct Handle : Copy { val id: Int }`, `struct Button { val onClick: () -> Int }` and `fun button(h: Handle): Button = Button({ h.id * 2 })` are declared and `prism dump-clif main.prism` runs on a file holding them and `fun main() { val b = button(Handle(21)); println(b.onClick()) }`
- **THEN** no function of the file calls the runtime's allocation function, and `prism run --check-allocations main.prism` prints `42` and exits with status 0

## MODIFIED Requirements

### Requirement: Function Values Are a Descriptor and an Environment
A value of a function type, composable or not, MUST be 16 bytes with the alignment of a pointer: the address of its closure's descriptor at offset 0, never null, and at offset 8 its environment word: the address of its environment, null when the closure captures nothing, or the environment itself when it is held inline. It MUST be held, passed and returned in memory like a struct, and a nullable function value MUST be laid out with a tag byte.

#### Scenario: Layout of a struct holding a function value
- **WHEN** `prism dump-layout main.prism` runs on a file declaring `struct Task { val run: () -> Unit, val id: Int }` and `fun later(f: (() -> Unit)?) {}`
- **THEN** it shows `Task` with size 24 and alignment 8, `run` at offset 0 and `id` at 16, and `(() -> Unit)?` with size 24, its tag at offset 0 and its payload at 8

#### Scenario: Signature of a function taking a function value
- **WHEN** `fun call(f: () -> Int): Int = f()` is declared and `prism dump-clif main.prism` runs
- **THEN** `call` takes one machine value, the address of `f`, and returns a 64-bit integer

#### Scenario: An escaping parameter is represented alike
- **WHEN** `fun call(f: escaping () -> Int): Int = f()` is declared and `prism dump-clif main.prism` runs
- **THEN** `call` takes one machine value, the address of `f`, and returns a 64-bit integer

### Requirement: Each Closure Has a Descriptor
Each compiled closure body MUST have one descriptor, static data of its code unit named `<closure symbol>#descriptor`, holding the address of the function-table slot of the closure's code and the environment's drop entry: the address of its `#drop-environment` function when its environment is allocated, and null when the closure only borrows or its environment is held inline. Every value the closure builds MUST point at that descriptor.

#### Scenario: Building a closure names its descriptor
- **WHEN** `fun run(f: () -> Int): Int = f()` is declared and `prism dump-clif main.prism` runs on a file holding `fun main() { val a = 1; println(run { a + 1 }) }`
- **THEN** the body of `main` writes the address of the data symbol `main::main#closure0#descriptor` into the closure value it passes to `run`
