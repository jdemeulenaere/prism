## ADDED Requirements

### Requirement: Dropping a State Object Retires Its State
The drop glue of `State<T>` and `MutableState<T>` MUST call the compose runtime's retire entry with the state's word, so every drop of a state object, of a local, a field, a list element or a slot, retires its state exactly once.

#### Scenario: Retiring the states of a list of items
- **WHEN** `prism dump-clif` runs on `struct Todo { val title: String, val done: MutableState<Bool> }` and `fun f(todos: List<Todo>) { }`
- **THEN** the drop glue of `Todo` calls that of `MutableState<Bool>` for `done`, which calls `prism_compose_state_retire` with the word it loads from the state, and `f` drops `todos` through the list's drop glue, which calls `Todo`'s for each element

### Requirement: Weak Captures Are Copied Words
A weak capture MUST be held in a closure's environment as one word that owns nothing, so a closure holding only weak handles and `Copy` captures has environment equality and duplicate entries, and one holding a single weak handle keeps it inline in the environment word, allocating nothing.

#### Scenario: A handler with one weak capture allocates nothing
- **WHEN** `fun keep(f: attached () -> Unit) {}` is declared and `prism dump-clif` runs on `fun f(count: &MutableState<Int>) { keep { count.value += 1 } }`
- **THEN** `f` builds the function value with the weak handle's word inline, without calling the allocator, and the unit defines `main::f#closure0#equals-environment`

## MODIFIED Requirements

### Requirement: State Handles Are One Word
`State<T>`, `MutableState<T>` and `WeakReference<T>` MUST be laid out as one 8-byte, 8-aligned word for every `T`. A state object MUST have drop glue that retires its state, no clone glue, and equality glue that compares the words; a weak reference MUST have no drop glue and compare its words. `prism dump-layout` MUST name them `prism/compose/runtime::State<...>`, `prism/compose/runtime::MutableState<...>` and `prism/compose/runtime::WeakReference<...>`.

#### Scenario: Layout of a state handle
- **WHEN** `prism dump-layout` runs on a file declaring `struct Todo { val title: String }` and `fun f(s: MutableState<List<Todo>>, r: WeakReference<MutableState<Int>>) { }`
- **THEN** it prints the layouts of `prism/compose/runtime::MutableState<List<Todo>>` and `prism/compose/runtime::WeakReference<prism/compose/runtime::MutableState<Int>>` as size 8 and alignment 8, each holding only the hidden `handle: Int` word that every intrinsic struct has

### Requirement: State Operations Call the Compose Runtime
Each state operation MUST be a direct call of a compose runtime entry point with the state's word, loaded through the borrow of the state object, the slot type record of the value type, and the value passed by address where it moves. When the entry reports a failure, the generated code MUST raise the error the runtime recorded at the operation's location.

#### Scenario: A read in the IR dump
- **WHEN** `prism dump-clif` runs on `fun f(s: &State<Int>): Int = s.value`
- **THEN** the function loads the state's word through `s`, calls `prism_compose_state_read` with it and the record of `Int`, tests the result for null, and on null calls `prism_compose_state_error` with the location of `s.value`

### Requirement: One-Word Intrinsic Structs Travel as Words
A value of an intrinsic struct that the catalog lays out as one word (`NodeId`, `HandlerId`, `State<T>`, `MutableState<T>`, `WeakReference<T>`, `RecomposeScope`, `Composition`) MUST be passed to and returned from functions and externals as a 64-bit word, not by address. Its representation is fixed by the catalog, so no signature depends on a declared type's layout.

#### Scenario: An external taking a node handle
- **WHEN** `prism dump-clif` runs on a library test module declaring `private external fun prism_ui_set_enabled(node: NodeId, enabled: Bool)` and a call of it
- **THEN** the call passes two machine values, a 64-bit word and an 8-bit `Bool`, and no address

### Requirement: Top-Level Initializers Run as Program Code
The initializer of a top-level value MUST run between a call that marks program code and a call that ends it, so it may create states even when it first runs during a pass.

#### Scenario: A global state first read during composition
- **WHEN** `val Theme: MutableState<Int> = mutableStateOf(3)` and `compose fun C() { println(Theme.value) }` are declared and `fun main() { val c = composition { C() } }` runs with `--check-allocations`
- **THEN** stdout is `3`, and the exit status is 0
