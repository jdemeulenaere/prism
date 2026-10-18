# Code Generation Delta

## ADDED Requirements

### Requirement: State Handles Are One Word
`State<T>` and `MutableState<T>` MUST be laid out as one 8-byte, 8-aligned word for every `T`, with no drop glue, clone glue that copies the word, and equality glue that compares the words. `prism dump-layout` MUST name them `prism/compose/runtime::State<...>` and `prism/compose/runtime::MutableState<...>`.

#### Scenario: Layout of a state handle
- **WHEN** `prism dump-layout` runs on a file declaring `struct Todo { val title: String }` and `fun f(s: MutableState<List<Todo>>) { }`
- **THEN** it prints the layout of `prism/compose/runtime::MutableState<List<Todo>>` as size 8 and alignment 8 with no fields

### Requirement: State Operations Call the Compose Runtime
Each state operation MUST be a direct call of a compose runtime entry point with the handle word, the slot type record of the value type, and the value passed by address where it moves. When the entry reports a failure, the generated code MUST raise the error the runtime recorded at the operation's location.

#### Scenario: A read in the IR dump
- **WHEN** `prism dump-clif` runs on `fun f(s: State<Int>): Int = s.value`
- **THEN** the function calls `prism_compose_state_read` with the handle and the record of `Int`, tests the result for null, and on null calls `prism_compose_state_error` with the location of `s.value`

### Requirement: Cell Records Compare by Equality Glue
The slot type record of a value type held in a cell MUST hold its drop glue, or null, and an equality entry exactly when `==` applies to the type, strings and lists included, or when it is a function type, compared by closure and captures. The equality entry MUST be the type's equality glue.

#### Scenario: Records of a unit's cells
- **WHEN** `prism dump-clif` runs on `fun main() { val s = mutableStateOf("a"); val l = mutableStateOf(listOf(1)); s.value = "b"; l.value = listOf(2) }`
- **THEN** the unit defines one slot type record each for `String` and `List<Int>`, naming `#equals(String)` and `#equals(List<Int>)` as their equality entries and their drop glue as their drop entries

### Requirement: Top-Level Initializers Run as Program Code
The initializer of a top-level value MUST run between a call that marks program code and a call that ends it, so the cells it creates belong to the program even when it first runs during a pass.

#### Scenario: A global state first read during composition
- **WHEN** `val Theme: MutableState<Int> = mutableStateOf(3)` and `compose fun C() { println(Theme.value) }` are declared and `fun main() { val c = composition { C() } }` runs with `--check-allocations`
- **THEN** stdout is `3`, and the exit status is 0
