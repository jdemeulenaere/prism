## ADDED Requirements

### Requirement: The Current Composer Is a Built-In Call
A call of `currentComposer()` of `/prism/composer` MUST be a call of its built-in, of type `Int`, with no argument.

#### Scenario: The current composer in the tree
- **WHEN** `compose fun C(): Int = currentComposer()` is checked with `import /prism/composer` and its typed tree dumped
- **THEN** the body is a call of the built-in `currentComposer` of type `Int`

## MODIFIED Requirements

### Requirement: State Built-Ins Are Identified
Calls of `mutableStateOf`, `update`, `weak` and `get` MUST be calls of their built-ins, with their declared type arguments, the value type first for `mutableStateOf` and `update`. A call of `SideEffect` MUST be a call of its declaration.

#### Scenario: An update call
- **WHEN** `fun f(s: &MutableState<List<Int>>) { s.update { v -> v } }` is checked and its typed tree dumped
- **THEN** the call is a call of the built-in `update` with type argument `List<Int>`, a shared receiver borrow of `*s` as its receiver and the closure as its argument

#### Scenario: A side effect in the tree
- **WHEN** `compose fun C() { SideEffect { } }` is checked and its typed tree dumped
- **THEN** the call is a call of the declaration `prism/compose/runtime::SideEffect` with the closure as its argument
