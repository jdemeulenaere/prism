# Compilation and ABI Delta

## ADDED Requirements

### Requirement: Library Declarations in the ABI
Phase 1 MUST extract and hash a library module's ABI as a workspace file's, from its syntax tree alone. An exported function's entry MUST record whether it is ordinary, intrinsic or external, and an exported intrinsic extension property MUST be an entry with its type parameters, receiver type, name and type. Both MUST be covered by the ABI hash and shown by `prism abi` with their modifiers (`public intrinsic val <T> List<T>.size: Int`).

#### Scenario: The ABI of a library module
- **WHEN** the ABI of `/prism/collections` is extracted
- **THEN** it holds `public intrinsic mut fun <T> List<T>.add(element: T): Unit`, `public intrinsic val <T> List<T>.size: Int` and the ordinary `public fun <T, R> List<T>.map(transform: (&T) -> R): List<R>`, and changing an entry from intrinsic to ordinary changes the module's ABI hash

### Requirement: Prelude Modules Are Inputs of Every Workspace File
Phase 2 of a workspace file MUST see the ABIs of the prelude modules as well as those of its direct imports, after them, and nothing else of the library. Instance planning MUST treat a call of a prelude generic as a call of a direct import's generic. Phase 2 of a library file MUST see only the ABIs of its direct imports.

#### Scenario: Instances of prelude generics
- **WHEN** `fun main() { println(listOf(1, 2).map { x -> x + 1 }.size) }` is run, in a file without imports
- **THEN** the program's instances include `prism/collections::List.map<Int, Int>`, compiled into its own unit, and the program prints `2`
