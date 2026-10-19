## MODIFIED Requirements

### Requirement: The Todo App Adds Items From Its Text Field
The todo app MUST hold its items in a `MutableState<List<Todo>>`, each item holding its done state in a `MutableState<Bool>` of its own. Clicking `Add` MUST append an item, not done, whose title is the text field's value, and clear the field. With an empty field, `Add` MUST add nothing. Below the items, the app MUST show `<n> left`, the number of items not done.

#### Scenario: Adding, toggling and removing
- **WHEN** the session `adding-toggling-and-removing` adds `Milk`, `Eggs` and `Bread` by `click textfield`, `type` and `click "Add"`, then runs `click checkbox 2`, `click "Remove" 1` and `dump`
- **THEN** the dump shows two rows, `Eggs` checked and `Bread` not, an empty text field and `1 left`, and the session passes the allocation check

#### Scenario: An empty title is not added
- **WHEN** the session `an-empty-title-is-not-added` runs `click "Add"` and `dump` on the empty app
- **THEN** the dump shows no row and `0 left`

### Requirement: Rows Toggle and Remove Their Item
Each item MUST be shown in a row of a `Checkbox` showing whether it is done, its title and a `Remove` button. The checkbox MUST write only the item's own done state, and `Remove` MUST remove the item, keeping the others in order.

#### Scenario: Toggling an item back
- **WHEN** the session `toggling-an-item-back` adds `Milk`, runs `click checkbox 1`, `dump`, `click checkbox 1` and `dump`
- **THEN** the first dump shows the checkbox checked and `0 left`, and the second shows it unchecked and `1 left`
