# Standard Library Delta

## ADDED Requirements

### Requirement: The UI Modules Are Bundled Outside the Prelude
The compiler MUST ship `/prism/compose/ui`, `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3` as bundled modules that are not in the prelude, all files of the package `prism.compose` beside `/prism/compose/runtime`. Their files MUST be checked, formatted and compiled by the library tests like every bundled module, and their externals MUST match `prism-ui`'s entry points.

#### Scenario: The UI modules' externals match the entry points
- **WHEN** `library_tests.rs` checks and compiles every bundled module
- **THEN** `prism/compose/ui.prism`, `prism/compose/foundation.prism`, `prism/compose/layout.prism` and `prism/compose/material3.prism` pass `prism check`, `prism fmt --check` and the backend's check of each external, with no `E6005`

#### Scenario: The UI modules are not in the prelude
- **WHEN** `prism check --explain-cache` runs on a file without imports, and then on the same file with `import /prism/compose/material3`
- **THEN** the first lists no file of `prism/compose/` but `prism/compose/runtime.prism`, and the second also lists `prism/compose/ui.prism`, `prism/compose/foundation.prism`, `prism/compose/layout.prism` and `prism/compose/material3.prism`

### Requirement: The UI Intrinsics
The intrinsic catalog MUST hold, as intrinsics of `/prism/compose/ui`: the one-word structs `NodeId : Copy`, with `package` visibility, and `HandlerId : Copy`, public, whose equality is the runtime entry `prism_ui_handlers_equal`; the `package` composables `Node(kind: Int, update: (NodeId) -> Unit, content: compose () -> Unit)` and `LeafNode(kind: Int, update: (NodeId) -> Unit)`; and the function `runApp`. A declaration of one of them with another signature or in another module MUST be `E4029`.

#### Scenario: The public signature of runApp
- **WHEN** `prism abi` prints the ABI of `/prism/compose/ui`
- **THEN** it holds the public intrinsic function `runApp` with the parameters `title: &String`, `width: Int` and `height: Int`, each with a default, and `content: escaping compose () -> Unit`, the public intrinsic struct `HandlerId : Copy`, and the public intrinsic properties `Int.dp` and `Int.sp`, and none of `NodeId`, `Node` and `LeafNode`, which are not public

#### Scenario: A UI intrinsic in another library module
- **WHEN** a library test module declares `private intrinsic compose fun LeafNode(kind: Int, update: (Int) -> Unit)`
- **THEN** the type checker emits `E4029`, saying that the compiler provides `LeafNode` only in `/prism/compose/ui`

#### Scenario: Calling runApp with its defaults
- **WHEN** `fun main() { runApp { } }` runs with `import /prism/compose/ui` under `prism run --headless` with the script `dump`
- **THEN** stdout is `window 800x600`: `width` and `height` took their defaults

### Requirement: The Unit Properties
The catalog MUST hold the public intrinsic properties `val Int.dp: Dp` and `val Int.sp: TextUnit` of `/prism/compose/ui`, lowered as the constructions `Dp(n)` and `TextUnit(n)`, and members of `Int` wherever that module is visible.

#### Scenario: Unit properties are members of Int
- **WHEN** a file declares `import /prism/compose/ui` and `fun Int.dp(): Int = 0`
- **THEN** the type checker emits `E4010`, saying that the member of `/prism/compose/ui` always takes precedence
