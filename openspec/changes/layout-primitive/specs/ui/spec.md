## ADDED Requirements

### Requirement: The Layout Primitive
`/prism/compose/ui` MUST declare `Layout(content: compose () -> Unit, modifier: Modifier = Modifier, measurePolicy: attached MeasureScope.(&List<Measurable>, Constraints) -> MeasureResult)`. A call MUST make one node of kind `layout`, whose children are those `content` composes, and whose content is measured by calling `measurePolicy` with the node's children as measurables, in order, and the constraints left by its modifier chain.

#### Scenario: A custom layout places its children diagonally
- **WHEN** a headless app of 400 × 300 composes `Diagonal { Text("a"); Text("bb") }`, where `compose fun Diagonal(content: compose () -> Unit) { Layout(content = content) { measurables, constraints -> val placeables = measurables.map { it.measure(constraints) }; var width = 0; var height = 0; for (p in placeables) { width += p.width; height += p.height }; layout(width, height) { var x = 0; var y = 0; for (p in placeables) { p.place(x, y); x += p.width; y += p.height } } } }`, runs under `--check-allocations`, and its script is `dump`
- **THEN** stdout is the lines `window 400x300`, `  layout #1 0,0 24x40`, `    text #2 0,0 8x20 "a"` and `    text #3 8,20 16x20 "bb"`, and the exit status is 0

#### Scenario: A grid of two columns
- **WHEN** a headless app of 400 × 300 composes `Grid(columns = 2) { Text("a"); Text("b"); Text("c") }`, where `Grid`'s policy measures each child with `Constraints(0, constraints.maxWidth / columns, 0, constraints.maxHeight)`, places child `i` at column `i % columns` of that width and below the rows before it, and takes the maximum width and the rows' total height, and its script is `dump`
- **THEN** stdout is the lines `window 400x300`, `  layout #1 0,0 400x40`, `    text #2 0,0 8x20 "a"`, `    text #3 200,0 8x20 "b"` and `    text #4 0,20 8x20 "c"`

### Requirement: Constraints
`Constraints` MUST be a `Copy` struct of `minWidth`, `maxWidth`, `minHeight` and `maxHeight`, all `Int` logical pixels, with the companion value `Constraints.Infinity`, `Int`'s maximum, as the unbounded maximum. It MUST offer Compose's `fixed`, `fixedWidth`, `fixedHeight`, `hasBoundedWidth`, `hasBoundedHeight`, `hasFixedWidth`, `hasFixedHeight`, `isZero`, `constrainWidth`, `constrainHeight`, `constrain`, `offset` and `copy`.

#### Scenario: Building and using constraints
- **WHEN** a file importing `/prism/compose/ui` and `/prism/core` runs `fun main() { val c = Constraints.fixed(10, 20); println(c.constrainWidth(30)); println(c.hasFixedHeight); val loose = Constraints(0, Constraints.Infinity, 5, 50); println(loose.hasBoundedWidth); println(loose.constrainHeight(2)); println(loose.offset(vertical = -10).minHeight); println(loose.copy(maxHeight = 40).maxHeight) }` with `--check-allocations`
- **THEN** stdout is `10`, `true`, `false`, `5`, `0` and `40`, one per line

### Requirement: Measuring a Child
`Measurable.measure(constraints)` MUST measure the child under `constraints` and return a `Placeable` of its size. It MUST be called only by the measure policy of the child's parent while that policy runs, at most once per child in one call of the policy, and with constraints whose minimums are at least 0 and at most their maximums. A child keeps its last measurement when it and its constraints did not change.

#### Scenario: Measuring a child twice
- **WHEN** a headless app composes `Layout(content = { Text("a") }) { measurables, constraints -> val a = measurables[0].measure(constraints); val b = measurables[0].measure(constraints); layout(a.width, b.height) { } }`
- **THEN** stderr is exactly `panic: ui misuse: a measurable was measured twice in one measurement` and a newline, and the exit status is 101

#### Scenario: Invalid constraints
- **WHEN** a headless app composes `Layout(content = { Text("a") }) { measurables, constraints -> val a = measurables[0].measure(Constraints(10, 5, 0, 0)); layout(a.width, a.height) { } }`
- **THEN** stderr is exactly `panic: ui misuse: invalid constraints: a minimum is negative or above its maximum` and a newline, and the exit status is 101

### Requirement: Placing Children
`MeasureScope.layout(width, height, placementBlock)` MUST give the node's content that size, clamped into the content's constraints, run `placementBlock` at once, and return the `MeasureResult` a policy returns. In the block, `Placeable.place(x, y)` and `placeRelative(x, y)` MUST place a child measured in the same call at `(x, y)` from the content's top-left corner. Outside a placement block, `place` MUST be rejected with `E4034`.

#### Scenario: A flow row wraps its children
- **WHEN** a headless app of 400 × 300 composes `Box(modifier = Modifier.width(40.dp)) { FlowRow { Text("aa"); Text("bb"); Text("cc") } }`, where `FlowRow`'s policy measures each child with `constraints.copy(minWidth = 0, minHeight = 0)`, starts a new line when a child would pass `constraints.maxWidth`, and lays itself out as wide as its widest line and as high as its lines, and its script is `dump`
- **THEN** stdout is the lines `window 400x300`, `  box #1 0,0 40x40 width(40)`, `    layout #2 0,0 32x40`, `      text #3 0,0 16x20 "aa"`, `      text #4 16,0 16x20 "bb"` and `      text #5 0,20 16x20 "cc"`

#### Scenario: Placing outside a placement block
- **WHEN** `compose fun Screen() { Layout(content = { Text("a") }) { measurables, constraints -> val a = measurables[0].measure(constraints); a.place(0, 0); layout(a.width, a.height) { } } }` is checked
- **THEN** the type checker emits `E4034` at `place`, saying that it needs a `PlacementScope` and that the innermost receiver is a `MeasureScope`

### Requirement: Layout Sizes and Positions Are Bounded
A node's size MUST be at most 2,147,483,647 logical pixels on each axis, and a child MUST be placed at most that far from its parent's content's top-left corner on each axis. Larger values that a program asks for, through a modifier's size or padding, constraints without a bound or a measure policy's `layout` or `place`, MUST be clamped to that range, and never end the program.

#### Scenario: A child placed out of range
- **WHEN** a headless app of 400 × 300 composes `Column(modifier = Modifier.padding(10.dp)) { Layout(content = { Text("far"); Text("near") }) { measurables, constraints -> val far = measurables[0].measure(constraints); val near = measurables[1].measure(constraints); layout(near.width, near.height) { far.place(Constraints.Infinity, -9223372036854775807 - 1); near.place(0, 0) } } }` under `--check-allocations`, and its script is `dump`
- **THEN** the dump holds `text #3 2147483657,-2147483637 24x20 "far"` and `text #4 10,10 32x20 "near"`, and the exit status is 0

#### Scenario: A layout under unbounded constraints
- **WHEN** a headless app of 400 × 300 composes a `Layout` that measures its child, `Box(modifier = Modifier.padding(5.dp).background(Color.Red)) { Layout(content = { }) { measurables, constraints -> layout(constraints.maxWidth, constraints.maxHeight) { } } }`, under `Constraints(0, Constraints.Infinity, 0, Constraints.Infinity)`, and its script is `dump` and `dump draw`
- **THEN** the dump holds `box #2 0,0 2147483647x2147483647 padding(5) background(#FFFF0000)` and `layout #3 5,5 2147483647x2147483647`, and the exit status is 0

#### Scenario: A padding out of range
- **WHEN** a headless app of 400 × 300 composes `Column(modifier = Modifier.padding(start = 9223372036854775807.dp).padding(start = 9223372036854775807.dp).size(9223372036854775807.dp)) { Text("a") }` and its script is `dump`
- **THEN** the dump holds `column #1 0,0 400x300 padding(2147483647,0,0,0) padding(2147483647,0,0,0) size(2147483647x2147483647)` and `text #2 4294967294,0 0x20 "a"`, and the exit status is 0

### Requirement: Children a Policy Does Not Place
A child that its parent's policy did not place in its last measurement, and that child's subtree, MUST NOT be drawn, hit by presses or found by script targets. The children of a text or text field, which only a raw node group of `/prism/composer` can give it, MUST be unplaced too. `dump` MUST print an unplaced node with `unplaced` after its other properties and before its modifier elements.

#### Scenario: An unplaced text
- **WHEN** a headless app of 400 × 300 composes `Layout(content = { Text("shown"); Text("hidden") }) { measurables, constraints -> val first = measurables[0].measure(constraints); layout(first.width, first.height) { first.place(0, 0) } }` and its script is `dump`, `dump draw` and `click "hidden"`
- **THEN** stdout is the lines `window 400x300`, `  layout #1 0,0 40x20`, `    text #2 0,0 40x20 "shown"`, `    text #3 0,0 0x0 "hidden" unplaced`, `clear 400x300 #FFFFFFFF` and `text 0,0 size 16 #FF000000 "shown"`, stderr is exactly `panic: event script line 3: no node shows the text "hidden"` and a newline, and the exit status is 101

### Requirement: Measure Policies Are Kept by Their Nodes
A node MUST keep its measure policy in the handler table, as it keeps its handlers. On each pass, a node given a policy with the same code and equal captures MUST keep its own and its measurement; given another, it MUST drop its old policy and be measured again at the next layout. A policy MUST be called through its code's function-table slot.

#### Scenario: A changed policy lays its node out again
- **WHEN** a headless app of 400 × 300 composes `Column { Spaced(gap.value) { Text("a"); Text("b") }; Button(onClick = { gap.value = 10 }) { Text("wider") } }`, with `val gap = remember { mutableStateOf(0) }` and `compose fun Spaced(gap: Int, content: compose () -> Unit)` a `Layout` that stacks its children with `gap` pixels after each, runs under `--check-allocations`, and its script is `dump`, `click "wider"` and `dump`
- **THEN** the first dump holds `layout #2 0,0 8x40` and `text #4 0,20 8x20 "b"`, the second holds `layout #2 0,0 8x60` and `text #4 0,30 8x20 "b"`, and the exit status is 0

### Requirement: A Layout Node Needs a Measure Policy
A node of kind `column`, `row`, `box`, `spacer` or `layout` that is measured without a measure policy, which only a raw node group of `/prism/composer` can make, MUST end the program with `ui misuse: a layout node has no measure policy`.

#### Scenario: A raw column without a policy
- **WHEN** a headless app composes `compose fun Raw() { val k = currentComposer(); startNodeGroup(k, 125, 1); endGroup(k) }` and its script is `dump`
- **THEN** stderr is exactly `panic: ui misuse: a layout node has no measure policy` and a newline, and the exit status is 101

### Requirement: Measure Policies Run as Program Code
A measure policy MUST run during layout, outside any pass, as a handler of the app's composition: states it creates belong to whatever holds them, and a runtime error in it ends the program with its location. A state it writes MUST invalidate its readers, which the frame loop recomposes and lays out again before the next event, for at most eight layouts in a row. A policy's reads of states MUST NOT make its node measure again when those states change.

#### Scenario: A policy writes a state
- **WHEN** a headless app of 400 × 300 composes `Column { Text("width ${width.value}"); Layout(content = { Text("abc") }) { measurables, constraints -> val a = measurables[0].measure(constraints); width.value = a.width; layout(a.width, a.height) { a.place(0, 0) } } }`, with `val width = remember { mutableStateOf(0) }`, under `--check-allocations`, and its script is `dump`
- **THEN** the dump holds `text #2 0,0 64x20 "width 24"`, and the exit status is 0

#### Scenario: A policy that writes on every layout
- **WHEN** a headless app of 400 × 300 composes `Column { Layout(content = { Spacer(modifier = Modifier.width(count.value.dp)) }) { measurables, constraints -> count.value = count.value + 1; ... }; Text("count ${count.value}") }`, with `val count = remember { mutableStateOf(1) }`, and its script is `dump`
- **THEN** the dump holds `spacer #3 0,0 8x0 width(8)` and `text #4 0,0 56x20 "count 8"`: eight layouts ran before the event, and the exit status is 0

#### Scenario: A policy divides by zero
- **WHEN** a headless app of 400 × 300 composes `Layout(content = { }) { measurables, constraints -> val zero = constraints.minWidth; layout(constraints.maxWidth / zero, 0) { } }` and no script
- **THEN** stderr starts with `panic: division by zero` and names the division's location in `main.prism`, and the exit status is 101

### Requirement: Measure Policy Misuse
Calling `measure` or `layout` outside the measurement of the node they name, calling `layout` twice in one measurement, and placing a child outside its parent's placement block or before measuring it MUST end the program with `ui misuse: a measurable was measured outside its parent's measure policy`, `ui misuse: layout was called outside its node's measurement`, `ui misuse: layout was called twice in one measurement` and `ui misuse: a placeable was placed outside its parent's placement`.

#### Scenario: Measuring with no layout running
- **WHEN** a Rust test of `prism-ui` calls the measure entry point with the handle of a child of a laid-out node during a handler of a running app
- **THEN** the runtime error `ui misuse: a measurable was measured outside its parent's measure policy` ends the test's child process with status 101

#### Scenario: Calling layout twice
- **WHEN** a headless app composes `Layout(content = { }) { measurables, constraints -> layout(0, 0) { }; layout(1, 1) { } }`
- **THEN** stderr is exactly `panic: ui misuse: layout was called twice in one measurement` and a newline, and the exit status is 101

### Requirement: A Measure Policy Calls layout
A measure policy that returns without having called `layout` in its measurement MUST end the program with `ui misuse: a measure policy returned without calling layout`.

#### Scenario: A policy that never calls layout
- **WHEN** a Rust test of `prism-ui` lays out an app whose node's Rust policy measures its child and returns without calling the placement entry points
- **THEN** the runtime error `ui misuse: a measure policy returned without calling layout` ends the test's child process with status 101

### Requirement: Columns, Rows, Boxes and Spacers Are Layouts
`Column`, `Row`, `Box` and `Spacer` MUST be Prism code of `/prism/compose/layout` over the measure policy protocol, with their arrangements, alignments and weights computed in Prism. Their nodes MUST keep the kinds `column`, `row`, `box` and `spacer`, and the layouts they compute MUST be those that this specification states for them.

#### Scenario: The counter screen through Prism layouts
- **WHEN** the counter screen runs headless in a window of 400 × 300 with the script `dump`
- **THEN** stdout is exactly the lines `window 400x300`, `  column #1 0,0 96x100 arrangement spacedBy(8) padding(16)`, `    text #2 16,16 64x20 "Count: 0"`, `    box #3 16,44 58x40 contentColor #FFFFFFFF background(#FF6750A4 RoundedCorner(50%)) clickable(Button)`, `      row #4 16,44 58x40 arrangement Center alignment CenterVertically defaultMinSize(58x40) padding(24,8,24,8)` and `        text #5 41,54 8x20 "+"`

### Requirement: Layout Allocates Nothing
Once its buffers have grown, a layout MUST allocate nothing in the toolkit, including the calls of the containers' measure policies and the measurables lent to them.

#### Scenario: Resizing an app allocates nothing
- **WHEN** `crates/prism-ui/tests/allocations.rs` drives an app of a column of rows, boxes and texts, measured by the test support's policies, through 100 resizes alternating between two window sizes, each followed by a layout of the whole tree and a display list build
- **THEN** no allocation happens after the first iteration

## MODIFIED Requirements

### Requirement: Constraint Layout in Logical Pixels
Layout MUST be one measure-and-place pass over the node tree: a parent passes minimum and maximum widths and heights to each child, which returns its size, and the parent places it. In a node, each modifier element MUST constrain and size the elements after it, from the first, and the content MUST then be measured, clamped to what remains: by the node's measure policy, by text measurement for texts and text fields. Sizes and positions MUST be `Int` logical pixels. The window's root MUST pass 0 to its size on each axis and place its children at its top-left corner.

#### Scenario: Padding around a column
- **WHEN** a headless app composes `Column(modifier = Modifier.padding(16.dp)) { Text("Hi") }` and its script is `dump`
- **THEN** the dump holds `column #1 0,0 48x52 padding(16)` and `text #2 16,16 16x20 "Hi"`

### Requirement: Layout Reuses Clean Measurements
A node whose properties, modifier chain, measure policy and descendants did not change since its last measurement, and that is measured with the same constraints, MUST reuse its last size without measuring its subtree or calling its policy. A node whose changed children keep their sizes MUST keep its size and their positions without calling its policy.

#### Scenario: One changed text is measured alone
- **WHEN** a Rust test lays out a tree of a column of 100 texts with a measurer that counts the characters it measures, changes one text, and lays out again
- **THEN** the second layout measures only the changed text's characters

#### Scenario: A text keeping its size calls no policy
- **WHEN** a Rust test lays out a column of 100 texts, measured by a test support policy that counts its calls, changes one text to another of the same width, and lays out again
- **THEN** the second layout calls the column's policy zero times, and a change to a wider text calls it once

### Requirement: Script Targets by Kind
Wherever an event script accepts a text target, it MUST also accept `<kind> [<n>]`: the n-th node, 1 by default and in tree order, of kind `column`, `row`, `box`, `text`, `spacer`, `textfield` or `layout`, or whose `clickable` or `toggleable` element has the role `button` or `checkbox`. The command MUST act at the centre of that node's bounds, rounded down, hit-testing from there.

#### Scenario: Clicking the second checkbox
- **WHEN** a headless app of 400 × 300 composes `compose fun Two() { val a = remember { mutableStateOf(false) }; val b = remember { mutableStateOf(false) }; Column { Checkbox(checked = a.value, onCheckedChange = { a.value = it }); Checkbox(checked = b.value, onCheckedChange = { b.value = it }) } }`, and its script is `click checkbox 2` then `dump`
- **THEN** stdout is the lines `window 400x300`, `  column #1 0,0 20x40`, `    box #2 0,0 20x20 toggleable(Checkbox off) size(20x20) border(2 #FF49454F RoundedCorner(2))` and `    box #3 0,20 20x20 toggleable(Checkbox on) size(20x20) background(#FF6750A4 RoundedCorner(2)) border(2 #FF6750A4 RoundedCorner(2))`

#### Scenario: Focusing a field by its kind
- **WHEN** the typing app of `ui`'s "Typing into a text field" runs headless with the script `click textfield`, `type "ab"` and `dump`
- **THEN** the dump holds `textfield #2 0,0 200x36 "ab" focused defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)`

#### Scenario: Clicking inside a custom layout by its kind
- **WHEN** a headless app of 400 × 300 composes `Diagonal { Button(onClick = { println("hit") }) { Text("go") } }`, with `Diagonal` as in "A custom layout places its children diagonally", and its script is `click layout`
- **THEN** stdout is `hit` and a newline

### Requirement: Dumping the Laid-Out Tree
`dump` MUST print `window <width>x<height>`, then one line per node in tree order, indented by two spaces per depth: `<kind> #<number> <x>,<y> <width>x<height>`, with absolute bounds and a kind among `column`, `row`, `box`, `text`, `spacer`, `textfield` and `layout`. Its non-default properties MUST follow, in the order text, `color`, `fontSize`, `contentColor`, `disabled`, `focused`, `arrangement`, `alignment` and `unplaced`, then its modifier elements in chain order, each with its arguments in parentheses.

#### Scenario: A dump of the counter screen
- **WHEN** the counter screen runs headless in a window of 400 × 300 with the script `dump`
- **THEN** stdout is exactly the lines `window 400x300`, `  column #1 0,0 96x100 arrangement spacedBy(8) padding(16)`, `    text #2 16,16 64x20 "Count: 0"`, `    box #3 16,44 58x40 contentColor #FFFFFFFF background(#FF6750A4 RoundedCorner(50%)) clickable(Button)`, `      row #4 16,44 58x40 arrangement Center alignment CenterVertically defaultMinSize(58x40) padding(24,8,24,8)` and `        text #5 41,54 8x20 "+"`

### Requirement: Layout and Drawing After a Reload
After the pass that follows a reload, the frame loop MUST lay out every node measured by a measure policy again, as the reload may have changed the code it runs, and build the display list only when that layout or the pass changed a node's bounds or what it draws. Matched nodes MUST keep their numbers and focus, and no frame MUST be presented when nothing visible changed.

#### Scenario: A focused field keeps its focus and text across a reload
- **WHEN** the typing app, run headless under `--watch`, gets the script `click 100 18`, `type "ab"`, an edit of `Text("Hello ${name.value}")` to `Text("Hi ${name.value}")`, then `type "c"` and `dump`
- **THEN** the dump holds `textfield #2 0,0 200x36 "abc" focused defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)` and `text #3 0,36 48x20 "Hi abc"`: both nodes kept their numbers, and the field its focus

#### Scenario: Editing a function a policy calls
- **WHEN** a headless app under `--watch` composes, in `app/main.prism`, `Spaced { Text("a"); Text("b") }`, where `Spaced` is a `Layout` that stacks its children with `gap()` pixels after each and `public fun gap(): Int = 0` is declared in `lib/gaps.prism`, and its script is `dump`, `edit lib/gaps.prism lib/gaps.prism.v2`, whose `gap()` returns 10, and `dump`
- **THEN** the first dump holds `text #3 0,20 8x20 "b"` and the second `layout #1 0,0 8x60` and `text #3 0,30 8x20 "b"`, although no composable body changed

### Requirement: Layout and Drawing Take Linear Time
A full layout and a display list build MUST take time linear in the nodes, their modifier elements and the characters of text measured, with one call of a measure policy per node it measures. A relayout MUST take time linear in the dirty nodes, their ancestors and those ancestors' children.

#### Scenario: Layout timing benchmark
- **WHEN** `crates/prism-ui/tests/ui_timing.rs` runs with `PRISM_ASSERT_TIMING=1` on trees of 1,000, 10,000 and 100,000 nodes
- **THEN** each tenfold increase multiplies the full layout and display list times by at most 15, and relaying out after one text change of the 10,000-node tree takes under a tenth of its full layout

### Requirement: Node Identity Survives Unrelated Edits
A node MUST be identified by its node group, which `Node` or `LeafNode` starts with Compose's constant node key under the calling toolkit composable's group, and which matches the old node group at the same position among its siblings. A node MUST keep its number, focus and layout cache while that group is matched, and a node group composed with another kind MUST get a new node.

#### Scenario: Inserting a button before a text keeps the text's node
- **WHEN** `compose fun Screen() { val more = remember { mutableStateOf(false) }; Column { if (more.value) { Button(onClick = { }) { Text("new") } }; Text("kept"); Button(onClick = { more.value = true }) { Text("more") } } }` runs headless with the script `click "more"` then `dump`
- **THEN** the dump shows `kept` as node `#2`, the node it was created as

#### Scenario: A node group composed with another kind gets a new node
- **WHEN** a composable of a headless app starts two node groups with the key 125 through `/prism/composer`, the first of kind text or text field and holding two `Text`s, the second of kind text field or text, each kind chosen by a state that a button flips, and the script is `dump`, `click "flip"`, `dump`, `click "flip"` and `dump`
- **THEN** each flip gives both node groups new nodes of their new kinds, the two texts keep their node numbers inside the new first node, where they are unplaced, and the program exits with status 0 under `--check-allocations`
