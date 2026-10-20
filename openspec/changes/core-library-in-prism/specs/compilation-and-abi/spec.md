## MODIFIED Requirements

### Requirement: Library Declarations in the ABI
Phase 1 MUST extract and hash a library module's ABI as a workspace file's, from its syntax tree alone. An exported function's entry MUST record whether it is ordinary, intrinsic or external. An exported extension property, intrinsic or with a getter, of any module, MUST be an entry with its type parameters, receiver type, name, type, writability and whether it is intrinsic. Both MUST be covered by the ABI hash and shown by `prism abi` with their modifiers (`public intrinsic val <T> List<T>.size: Int`, `public val Int.dp: Dp`). A getter's body MUST NOT be part of the ABI.

#### Scenario: The ABI of a library module
- **WHEN** the ABI of `/prism/core` is extracted
- **THEN** it holds `public intrinsic mut fun <T> List<T>.add(element: T): Unit`, `public intrinsic val <T> List<T>.size: Int`, the ordinary `public fun <T> List<T>.isEmpty(): Bool` and the property `public val <T> List<T>.indices: Range`, and changing an entry from intrinsic to ordinary changes the module's ABI hash

#### Scenario: Editing a getter recompiles nothing downstream
- **WHEN** `lib/geo.prism` exports `public struct Box : Copy { val w: Int }` and `public val Box.twice: Int get() = w * 2`, `app/main.prism` imports `/lib/geo` and prints `Box(3).twice`, the program has been run, and the getter's body becomes `w + w + 1`
- **THEN** the ABI hash of `lib/geo` is unchanged, and the next `prism run --explain-cache app/main.prism` prints `7`, `  checked lib/geo.prism` and `  reused app/main.prism`

#### Scenario: A property's type is part of the ABI
- **WHEN** the exported `public val Box.twice: Int get() = w * 2` of `lib/geo.prism` becomes `public val Box.twice: Double get() = 2.0`
- **THEN** the ABI hash of `lib/geo` changes, and a file importing it is checked again

### Requirement: Member Modules Are Inputs of Every Workspace File
Phase 2 of a workspace file MUST see the ABIs of its direct imports and, after them, the members of the built-in types that the member module `/prism/core` declares (`standard-library`, "Built-In Types and Their Members Need No Import"), and nothing else of the library. Instances of the generic members it calls MUST be planned for the program as those of an import's generics, whether or not the file imports `/prism/core`. Phase 2 of a library file MUST see only the ABIs of its direct imports.

#### Scenario: Instances of imported library generics
- **WHEN** `import /prism/collections`, `import /prism/core` and `fun main() { println(listOf(1, 2).map { x -> x + 1 }.size) }` are run
- **THEN** the program's instances include `prism/collections::List.map<Int, Int>`, compiled into its own unit, and the program prints `2`

#### Scenario: Member modules give members only
- **WHEN** a file without imports declares `fun f(xs: &List<Int>): Int = xs.size` and `fun g(): Int = listOf(1).size`
- **THEN** `f` typechecks, and `g` is rejected with `E4001` for `listOf`, which `/prism/collections` declares

#### Scenario: Instances of generic members
- **WHEN** a file importing `/prism/collections` and `/prism/system`, and not `/prism/core`, declares `fun main() { val xs = listOf("a"); exitProcess(if (xs.isNotEmpty()) 3 else 4) }` and runs with `prism run --explain-cache`
- **THEN** the program's instances include `prism/core::List.isNotEmpty<String>`, and the exit status is 3
