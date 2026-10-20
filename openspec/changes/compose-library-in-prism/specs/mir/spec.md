## ADDED Requirements

### Requirement: The Current Composer Is the Body's Composer Argument
In composed MIR, a call of `currentComposer()` MUST be an assignment of a copy of the body's `$composer` argument, with no call and no group.

#### Scenario: The composer read in a composable
- **WHEN** `prism dump-mir --composed` runs on `compose fun C(): Int = currentComposer()` with `import /prism/compose/runtime`
- **THEN** the body of `main::C` assigns a copy of `$composer` to its return place, and calls no composer operation for it

### Requirement: Bodies With Explicit Groups Start No Group
In composed MIR, the body of a composable annotated `@ExplicitGroupsComposable` MUST still take the composer and change words of every composable, and MUST start and end no group, keep no parameter, test no skipping and have no restart closure. Its composable calls MUST pass its composer and the change words it derives as in any composable body.

#### Scenario: A node function's body
- **WHEN** `prism dump-mir --composed` runs on a library test module `/prism/compose/test` that declares `@ExplicitGroupsComposable compose fun L(kind: Int, update: (Int) -> Unit) { update(kind) }`
- **THEN** the body of `L` takes `$composer` and one change word, calls the update closure, and calls no `composer start_group`, `composer start_restart_group` or `composer end_group`, and the dump has no `L#restart` body

### Requirement: Library Composables Are Called Like Any Function
Calls of `composition`, `setContent`, `recompose`, `dump`, `invalidate`, `currentRecomposeScope`, `SideEffect`, `Node`, `LeafNode` and `runApp` MUST lower as calls of their declarations, composable ones with the caller's composer and change words, and no body MUST get a root content adapter.

#### Scenario: A composition started in MIR
- **WHEN** `compose fun Hello() {}` and `fun main() { val c = composition { Hello() } }` are composed
- **THEN** `prism dump-mir --composed` shows `main` calling `prism/compose/runtime::composition` with the content lambda, moved, and no `main::main#root-content` body

#### Scenario: A toolkit composable calls its node function
- **WHEN** `prism dump-mir --composed` runs on a library test module `/prism/compose/test`, of the package `prism.compose`, that imports `ui` and declares `compose fun T(text: &String) { LeafNode(4) { node -> } }`
- **THEN** the body of `T` calls `prism/compose/ui::LeafNode` with `const 4`, the update closure, its composer and a change word, inside its own body group, and starts no node group itself

## MODIFIED Requirements

### Requirement: Composed MIR Marks Remember Windows and Side Effects
In composed MIR, the calculation of a `remember` that misses its slot MUST run between a `begin_remember` and an `end_remember` composer operation, the end after the result's slot is written, and `remember` of a non-`Copy` type MUST give a shared reference to the slot's value. A call of `SideEffect` MUST be a call of its declaration, which queues the effect through the compose runtime.

#### Scenario: Composed remember of a state
- **WHEN** `prism dump-mir --composed` runs on `compose fun C() { val s = remember { mutableStateOf(0) } }`
- **THEN** the missing path calls `composer begin_remember`, then the calculation, then `composer set_slot<MutableState<Int>>`, then `composer end_remember`, and `s` holds a shared reference to the slot's `MutableState<Int>` on both paths

## REMOVED Requirements

### Requirement: Root Content Adapters
**Reason**: The compose runtime calls composable content itself, with the composer and the force bit (compose, "Root Content Is Called With the Force Bit"), so no body builds an adapter for `composition` or `setContent` (design D4).
**Migration**: "Library Composables Are Called Like Any Function" replaces it; its scenario becomes "A composition started in MIR".

### Requirement: runApp Gets a Root Content Adapter
**Reason**: `runApp` is a Prism function that gives its content to `prism_ui_run_app`, which the runtime calls with the force bit (design D4, D10).
**Migration**: Covered by "Library Composables Are Called Like Any Function"; its fixture is replaced.

### Requirement: Node Calls Start Node Groups
**Reason**: `Node` and `LeafNode` are Prism functions with explicit groups that start their node groups through the protocol, with Compose's constant node key (design D9).
**Migration**: "Library Composables Are Called Like Any Function" ("A toolkit composable calls its node function") and ui "Node Identity Survives Unrelated Edits" replace it.
