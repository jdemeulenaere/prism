# Code Generation Delta

## ADDED Requirements

### Requirement: External Functions Are Called by Symbol
A call of an `external fun` MUST be a direct call of the imported symbol named like the function, with the platform's C calling convention and the machine signature the same Prism signature has: scalar parameters by value, every other parameter as an address, a result held in memory through an address passed first, and no result for `Unit` and `Nothing`. An external MUST have no function-table slot and no code of its own. A hot reload never changes what it calls.

#### Scenario: A direct call of an external
- **WHEN** the unit of `/prism/system` is compiled for a program that calls `exitProcess(1)`
- **THEN** `exitProcess`'s code calls `prism_rt_exit` directly, passing `status` as a 64-bit integer, and the program's function table has no slot named `prism/system::prism_rt_exit`

### Requirement: Externals Match the Runtime
The backend MUST find each external it calls among the runtime's entry points by symbol, and MUST compare the machine signature it derives with the one that entry records: the parameter types, the result type, and whether the function never returns. A missing symbol or any difference MUST be reported as `E6005` (`ExternalMismatch`) at the external's declaration, naming the symbol and both signatures. `prism check` never reports it.

#### Scenario: An external with the wrong signature
- **WHEN** a library test module declares `external fun prism_rt_print_int(value: Double)` and a program calls it
- **THEN** compiling the program reports `E6005`, saying that the runtime's `prism_rt_print_int` takes `i64` where the declaration passes `f64`

#### Scenario: An unknown external symbol
- **WHEN** a library test module declares `external fun prism_rt_missing(): Int` and a program calls it
- **THEN** compiling the program reports `E6005`, saying that the runtime has no entry point `prism_rt_missing`

## MODIFIED Requirements

### Requirement: Calls Go Through the Function Table
Every call of a top-level function, extension, default value generator or top-level `val` accessor, in the same file or another, other than a call of an external function ("External Functions Are Called by Symbol"), MUST load its target from the callee's slot in the program's function table and call it indirectly. A slot MUST keep its address for the life of the program, and a slot whose function is not linked MUST hold a stub that ends the program with a runtime error. No call MUST be inlined.

#### Scenario: Calls within and across files are indirect
- **WHEN** `app/main.prism` imports `/lib/math`, which declares `public fun twice(n: Int): Int = n * 2`, and declares `fun helper(): Int = 1` and `fun main() { println(twice(helper())) }`
- **THEN** `prism dump-clif app/main.prism` shows each of the two calls in `main` as a load from the callee's slot followed by an indirect call, and `prism run app/main.prism` prints `2`

#### Scenario: Recursive calls
- **WHEN** `fun fact(n: Int): Int = if (n <= 1) 1 else n * fact(n - 1)` and `fun main() { println(fact(20)) }` run
- **THEN** the program prints `2432902008176640000`
