# Standard Library Delta

## ADDED Requirements

### Requirement: The UI Module Is Bundled Outside the Prelude
The compiler MUST ship `/prism/ui` as a bundled module that is not in the prelude. Its file MUST be checked, formatted and compiled by the library tests like every bundled module, and its externals MUST match `prism-ui`'s entry points.

#### Scenario: The UI module's externals match the entry points
- **WHEN** `library_tests.rs` checks and compiles every bundled module
- **THEN** `prism/ui.prism` passes `prism check`, `prism fmt --check` and the backend's check of each external, with no `E6005`

#### Scenario: The UI module is not in the prelude
- **WHEN** `prism check --explain-cache` runs on a file without imports, and then on the same file with `import /prism/ui`
- **THEN** the first lists no `prism/ui.prism`, and the second lists it

### Requirement: The UI Intrinsics
The intrinsic catalog MUST hold `/prism/ui`'s intrinsics:
- the struct `NodeId : Copy`, one word;
- the composables `Node(kind: Int, update: (NodeId) -> Unit, content: compose () -> Unit)` and `LeafNode(kind: Int, update: (NodeId) -> Unit)`;
- the function `runApp`.

A declaration of one of them with another signature or in another module MUST be `E4029`.

#### Scenario: The public signature of runApp
- **WHEN** `prism abi` prints the ABI of `/prism/ui`
- **THEN** it holds the public intrinsic function `runApp` with the parameters `title: &String`, `width: Int` and `height: Int`, each with a default, and `content: escaping compose () -> Unit`, and none of `NodeId`, `Node` and `LeafNode`, which are private

#### Scenario: A UI intrinsic in another library module
- **WHEN** a library test module declares `private intrinsic compose fun LeafNode(kind: Int, update: (Int) -> Unit)`
- **THEN** the type checker emits `E4029`, saying that the compiler provides `LeafNode` only in `/prism/ui`

### Requirement: Intrinsic Parameters May Have Default Values
A parameter of an intrinsic function MUST be allowed a default value. The default MUST be checked and generated as for any function, in the declaring file, and evaluated by the caller before the call is lowered as the intrinsic.

#### Scenario: Calling runApp with its defaults
- **WHEN** `fun main() { runApp { } }` runs with `import /prism/ui` under `prism run --headless` with the script `dump`
- **THEN** stdout is `window 800x600`: `width` and `height` took their defaults
