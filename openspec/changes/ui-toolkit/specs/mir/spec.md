# MIR Delta

## ADDED Requirements

### Requirement: Node Calls Start Node Groups
In composed MIR, a call of the intrinsic composable `Node` or `LeafNode` MUST start a node group with its kind, keyed by its position among the node calls of its region. It MUST call `update` with the node the group returns, call `content` (for `Node`) with the caller's composer, and end the group. A node call MUST add no other group.

#### Scenario: A toolkit composable starts a node group
- **WHEN** `prism dump-mir --composed` runs on a library test module of `/prism/ui` that declares `compose fun T(text: &String) { LeafNode(4) { node -> } }`
- **THEN** the body of `T` shows `composer start node group` with key 16 hexadecimal digits followed by `/* node 0 */` and kind `4`, a call of the update closure with its result, and `composer end group` before its body group ends

### Requirement: runApp Gets a Root Content Adapter
A body that calls `runApp` MUST have one root content adapter closure, `<symbol>#root-content`, built from the content it is given and passed to the app, as for `composition`.

#### Scenario: The adapter of an app
- **WHEN** `prism dump-mir --composed` runs on `fun main() { runApp { } }` with `import /prism/ui`
- **THEN** `main` builds the closure value `main::main#root-content` from the content lambda, moved, and passes it to the call that runs the app
