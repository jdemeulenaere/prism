## ADDED Requirements

### Requirement: Handler Parameters Are Attached
The toolkit's handler parameters MUST be `attached`: `onClick` of `Button` and `clickable`, `onCheckedChange` of `Checkbox`, `onValueChange` of `toggleable`, `TextField` and `BasicTextField`, and `runApp`'s `content`, as must the parameters of the externals that register handlers. Composable content parameters of composables MUST stay unmarked.

#### Scenario: A row's checkbox writes its item's own state
- **WHEN** a headless app composes `compose fun ItemRow(item: &Item) { Row { Checkbox(checked = item.done.value, onCheckedChange = { item.done.value = it }); Text(item.title) } }` and `compose fun Screen() { val items = remember { mutableStateOf(listOf(Item("a", mutableStateOf(false)), Item("b", mutableStateOf(false)))) }; Column { for (item in items.value) { key(item.id) { ItemRow(item) } } } }`, with `struct Item { val id: Int, val title: String, val done: MutableState<Bool> }` built with ids 1 and 2, under `--check-allocations`, and its script is `click checkbox 2` then `dump`
- **THEN** the dump shows the second checkbox checked and the first one not, and the exit status is 0

#### Scenario: The toolkit's handler parameters in the ABI
- **WHEN** `prism abi` prints the ABIs of `/prism/compose/material3` and `/prism/compose/foundation`
- **THEN** `onClick` of `Button` and `clickable`, `onCheckedChange` of `Checkbox` and `onValueChange` of `toggleable`, `TextField` and `BasicTextField` are shown as `attached`

### Requirement: A Removed Item's Handlers Never Reach Its Freed State
When a pass removes the groups of an item whose handlers captured the item's states, their nodes and the handlers those own MUST be released before the item's states are freed, so no event reaches a handler of a freed state and no stale state is reported.

#### Scenario: Removing an item with its own state
- **WHEN** the app of "A row's checkbox writes its item's own state" also composes `Button(onClick = { items.value.removeAt(0) }) { Text("remove first") }` below its rows, runs under `--check-allocations`, and its script is `click checkbox 1`, `click "remove first"`, `click checkbox 1` and `dump`
- **THEN** stderr is empty, the exit status is 0, and the dump shows one row, `b`, with its checkbox checked

## MODIFIED Requirements

### Requirement: Checkboxes
`Checkbox(checked, onCheckedChange: attached (Bool) -> Unit, modifier = Modifier, enabled = true)` MUST be a box node with the caller's modifier, `toggleable` with the role `Checkbox` and `size(20.dp)`, then a background and a border of 2 in `#FF6750A4` when checked, or a border of 2 in `#FF49454F`, with corners of 2. A click on an enabled checkbox MUST call `onCheckedChange` with the opposite of `checked`, and the box MUST NOT change until the program passes a new `checked`.

#### Scenario: Toggling a checkbox
- **WHEN** a headless app of 400 × 300 composes `compose fun Task() { val done = remember { mutableStateOf(false) }; Row { Checkbox(checked = done.value, onCheckedChange = { done.value = it }); Text("milk") } }`, and its script is `click 10 10` then `dump`
- **THEN** stdout is the lines `window 400x300`, `  row #1 0,0 52x20`, `    box #2 0,0 20x20 toggleable(Checkbox on) size(20x20) background(#FF6750A4 RoundedCorner(2)) border(2 #FF6750A4 RoundedCorner(2))` and `    text #3 20,0 32x20 "milk"`

### Requirement: Handlers Run Outside Composition
Handlers MUST run as handlers of the app's composition, outside any pass, so they may write states and create them; a state a handler creates belongs to whatever holds its state object.

#### Scenario: A handler creates and writes a state
- **WHEN** a headless app composes `Button(onClick = { val s = mutableStateOf(41); s.value += 1; println(s.value) }) { Text("go") }` under `--check-allocations`, and its script is `click "go"`
- **THEN** stdout is `42` and a newline, and the exit status is 0: the state, retired when the handler's local is dropped, is freed after the next pass or with the composition
