# Code Generation Specification Delta

## ADDED Requirements

### Requirement: Stack Checks Are Reload Points
The stack check on entry to every compiled body, and on entry to glue, MUST call the runtime's `prism_rt_stack_check_failed` on its cold path before raising "stack overflow". When the limit held the reload interrupt, that call MUST restore the limit, apply the pending reload and return 0, and the function MUST continue. Otherwise it returns 1 and "stack overflow" is raised as before. The call MUST receive the stack pointer the check compared. The fast path MUST stay one load, one comparison and one branch.

#### Scenario: The cold path of a stack check
- **WHEN** `prism dump-clif main.prism` runs on `fun helper(): Int = 1` and `fun main() { println(helper()) }`
- **THEN** the entry block of `main::main` loads `prism_rt_stack_limit`, compares the stack pointer with it, and branches to a cold block that calls `prism_rt_stack_check_failed` and raises "stack overflow" only when that call returns 1

#### Scenario: An interrupt does not end the program
- **WHEN** a Rust test runs `fun value(): Int = 1` and `fun main() { var n = 0; while (value() == 1) { n += 1 } }`, and stores the reload interrupt into the stack limit with a pending reload that makes `value` return `2`
- **THEN** `main` returns, and the program exits with status 0 and no runtime error

## MODIFIED Requirements

### Requirement: Calls Go Through the Function Table
Every call of a top-level function, extension, default value generator or top-level `val` accessor, in the same file or another, MUST load its target from the callee's slot in the program's function table and call it indirectly. Code MUST name a slot only by its symbol, and the linker MUST resolve each slot reference to the address of the slot that the table maps the symbol to when the code is finalized. A slot MUST keep its address for the life of the program, and a slot whose function is not linked MUST hold a stub that ends the program with a runtime error. Under `prism run --watch`, a symbol MAY be mapped to a new slot when its code's contract changes (`hot-reload`). Code finalized before then MUST keep calling the old slot. No call MUST be inlined.

#### Scenario: Calls within and across files are indirect
- **WHEN** `app/main.prism` imports `/lib/math`, which declares `public fun twice(n: Int): Int = n * 2`, and declares `fun helper(): Int = 1` and `fun main() { println(twice(helper())) }`
- **THEN** `prism dump-clif app/main.prism` shows each of the two calls in `main` as a load from the callee's slot followed by an indirect call, and `prism run app/main.prism` prints `2`

#### Scenario: Recursive calls
- **WHEN** `fun fact(n: Int): Int = if (n <= 1) 1 else n * fact(n - 1)` and `fun main() { println(fact(20)) }` run
- **THEN** the program prints `2432902008176640000`

#### Scenario: A repointed slot keeps its address
- **WHEN** a Rust test links a program defining `main::value`, records the address of its slot, and links a reload defining `main::value` again with the same signature
- **THEN** the table maps `main::value` to the same slot address, and the slot holds the reload's code

### Requirement: Top-Level Values Have Storage and an Accessor
Each top-level `val` MUST have storage and an initialization state that belong to the program, allocated by the linker in a value table keyed by the value's symbol and value key (`hot-reload`), and imported by its file's code unit as the data symbols `<module>::<name>#storage` and `<module>::<name>#state`. Each `val` MUST also have an accessor, compiled in its file's code unit, that initializes the storage on its first call, as the `execution` spec defines, and returns its address. A read of a top-level `val`, in its own file or another, MUST call the accessor through the function table.

#### Scenario: Reading an imported top-level value
- **WHEN** `lib/config.prism` declares `public val LIMIT: Int = 10`, and `app/main.prism` imports `/lib/config` and declares `fun main() { println(LIMIT + 1) }`
- **THEN** `prism run app/main.prism` prints `11`

#### Scenario: Storage is imported by the unit
- **WHEN** `prism dump-clif lib/config.prism` runs on `public val LIMIT: Int = 10`
- **THEN** the accessor `lib/config::LIMIT#get` addresses `lib/config::LIMIT#storage` and `lib/config::LIMIT#state` as imported data symbols
