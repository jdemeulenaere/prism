## MODIFIED Requirements

### Requirement: First-Class Compose Primitives and Native Hot Reload
The language and runtime MUST provide native support for declarative UI via the `compose` keyword and state objects (`MutableState<T>`) owned like any value, whose contents live in runtime cells that never move. Generational state handles MUST remain an implementation detail of the runtime and of the closures the composition keeps.

#### Scenario: Hot reload function pointer swapping
- **WHEN** a composable function body is edited during development
- **THEN** the Tier 1 JIT swaps the function pointer in memory while preserving the existing slot-table UI state

#### Scenario: State owned by data
- **WHEN** a struct holds a `MutableState<Bool>` field, a list of such structs is kept in a remembered state, and an event handler removes one element
- **THEN** the removed element's state is freed without a garbage collector or a reference count, after the pass that removes the nodes showing it, and no handler of the app reports a stale state
