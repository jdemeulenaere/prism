## ADDED Requirements

### Requirement: Member Modules Are Inputs of Every Workspace File
Phase 2 of a workspace file MUST see the ABIs of its direct imports and, after them, the members of the built-in types that the member modules `/prism/core` and `/prism/collections` declare (`standard-library`, "Built-In Types and Their Members Need No Import"), and nothing else of the library. Phase 2 of a library file MUST see only the ABIs of its direct imports.

#### Scenario: Instances of imported library generics
- **WHEN** `import /prism/collections`, `import /prism/core` and `fun main() { println(listOf(1, 2).map { x -> x + 1 }.size) }` are run
- **THEN** the program's instances include `prism/collections::List.map<Int, Int>`, compiled into its own unit, and the program prints `2`

#### Scenario: Member modules give members only
- **WHEN** a file without imports declares `fun f(xs: &List<Int>): Int = xs.size` and `fun g(): Int = listOf(1).size`
- **THEN** `f` typechecks, and `g` is rejected with `E4001` for `listOf`, although `/prism/collections` is an input of the file

## REMOVED Requirements

### Requirement: Prelude Modules Are Inputs of Every Workspace File
**Reason**: There is no prelude: a workspace file's library inputs are its imports and the member modules, which give it only the members of the built-in types.
**Migration**: See "Member Modules Are Inputs of Every Workspace File"; the scenario "Instances of prelude generics" becomes "Instances of imported library generics".
