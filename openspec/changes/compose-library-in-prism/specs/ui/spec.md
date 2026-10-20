## MODIFIED Requirements

### Requirement: Node Identity Survives Unrelated Edits
A node MUST be identified by its node group, which `Node` or `LeafNode` starts with Compose's constant node key under the calling toolkit composable's group, and which matches the old node group at the same position among its siblings. A node MUST keep its number, focus and layout cache while that group is matched, and a node group composed with another kind MUST get a new node.

#### Scenario: Inserting a button before a text keeps the text's node
- **WHEN** `compose fun Screen() { val more = remember { mutableStateOf(false) }; Column { if (more.value) { Button(onClick = { }) { Text("new") } }; Text("kept"); Button(onClick = { more.value = true }) { Text("more") } } }` runs headless with the script `click "more"` then `dump`
- **THEN** the dump shows `kept` as node `#2`, the node it was created as

#### Scenario: A node group composed with another kind gets a new node
- **WHEN** a composable of a headless app starts two node groups with the key 125 through `/prism/composer`, the first of kind column or row and holding two `Text`s, the second of kind text or spacer, each kind chosen by a state that a button flips, and the script is `dump`, `click "flip"`, `dump`, `click "flip"` and `dump`
- **THEN** each flip gives both node groups new nodes of their new kinds, the two texts keep their node numbers inside the new first node, and the program exits with status 0 under `--check-allocations`
