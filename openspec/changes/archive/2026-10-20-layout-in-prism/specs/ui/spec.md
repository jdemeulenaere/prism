## ADDED Requirements

### Requirement: Layout Modifiers Are Measurement Steps
Each layout element of a chain (`padding`, `width`, `height`, `size`, `defaultMinSize`, `fillMax*`, `offset` and `layout`) MUST be a step written in Prism, as Compose's layout modifier nodes are: it measures the rest of the chain, the elements after it and then the node's content, once, under constraints it chooses, and places it. Other elements MUST pass the constraints on unchanged and record the bounds of the rest at their position.

#### Scenario: Layout modifiers inside a custom layout
- **WHEN** a headless app of 400 × 300 composes `Diagonal { Text("a", modifier = Modifier.padding(4.dp)); Text("b", modifier = Modifier.size(10.dp).padding(2.dp)) }`, with `Diagonal` as in "A custom layout places its children diagonally", and its script is `dump`
- **THEN** stdout is the lines `window 400x300`, `  layout #1 0,0 26x38`, `    text #2 0,0 16x28 "a" padding(4)` and `    text #3 16,28 10x10 "b" size(10x10) padding(2)`

### Requirement: The Layout Modifier
`/prism/compose/ui` MUST declare `Modifier.layout(measure: attached MeasureScope.(Measurable, Constraints) -> MeasureResult)`, which appends an element whose step is `measure`. The node MUST keep the step function as it keeps a handler. Two such elements MUST be equal when their functions have the same code and equal captures, all of them `Copy`. `dump` MUST print the element as `layout`.

#### Scenario: A layout modifier moves its content
- **WHEN** a headless app of 400 × 300 composes `Text("a", modifier = Modifier.layout { measurable, constraints -> val p = measurable.measure(constraints); layout(p.width + 10, p.height) { p.place(10, 0) } }.background(Color.Red))` and its script is `dump` then `dump draw`
- **THEN** stdout is the lines `window 400x300`, `  text #1 0,0 18x20 "a" layout background(#FFFF0000)`, `clear 400x300 #FFFFFFFF`, `rect 10,0 8x20 #FFFF0000` and `text 10,0 size 16 #FF000000 "a"`

#### Scenario: A card with an equal layout modifier is skipped
- **WHEN** `compose fun Card(modifier: Modifier) { println("card"); Box(modifier = modifier) { } }` is called as `Card(modifier = Modifier.layout { measurable, constraints -> val p = measurable.measure(constraints); layout(p.width, p.height) { p.place(0, 0) } })` in a column beside `Text("${c.value}")` and a `Button` incrementing `c`, and the headless script is `click "+"`
- **THEN** stdout is `card` once: the recomposition after the click skips `Card`

### Requirement: Offset
`offset(x: Dp = 0.dp, y: Dp = 0.dp)` of `/prism/compose/layout` MUST lay out the rest of the chain under the same constraints, keep its size, and place it at `(x, y)`, so that the elements after it, the content and the children are drawn and hit there while the node keeps its size and position. `dump` MUST print it as `offset(<x>,<y>)`.

#### Scenario: An offset background
- **WHEN** a headless app of 400 × 300 composes `Box(modifier = Modifier.offset(x = 10.dp, y = 5.dp).background(Color.Red).size(20.dp)) { }` and its script is `dump` then `dump draw`
- **THEN** stdout is the lines `window 400x300`, `  box #1 0,0 20x20 offset(10,5) background(#FFFF0000) size(20x20)`, `clear 400x300 #FFFFFFFF` and `rect 10,5 20x20 #FFFF0000`

#### Scenario: The clickable area follows an offset
- **WHEN** a headless app composes `Box(modifier = Modifier.offset(x = 20.dp).clickable { println("hit") }.size(10.dp)) { }` and its script is `click 5 5` then `click 25 5`
- **THEN** stdout is `hit` once

#### Scenario: A text field is hit where it is drawn
- **WHEN** a headless app of 400 × 300 composes `TextField(name.value, { name.value = it }, modifier = Modifier.offset(x = 150.dp))` and its script is `click 50 18`, `dump`, `click 250 18`, `type "a"` and `dump`
- **THEN** the first dump holds `textfield #1 0,0 200x36 offset(150,0) defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)`, unfocused, and the second `textfield #1 0,0 200x36 "a" focused offset(150,0) defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)`

### Requirement: A Layout Modifier Places Its Content
A layout modifier's step that returns without placing the rest of its chain MUST end the program with `ui misuse: a layout modifier did not place its content`.

#### Scenario: A layout modifier that hides its content
- **WHEN** a headless app composes `Text("a", modifier = Modifier.layout { measurable, constraints -> val p = measurable.measure(constraints); layout(p.width, p.height) { } })`
- **THEN** stderr is exactly `panic: ui misuse: a layout modifier did not place its content` and a newline, and the exit status is 101

### Requirement: Layout Modifier Steps Allocate Nothing
Once its buffers have grown, a layout MUST allocate nothing in the toolkit for the steps of `padding`, `width`, `height`, `size`, `defaultMinSize`, `fillMax*` and `offset`, including the elements passed to their steps.

#### Scenario: Resizing padded rows allocates nothing
- **WHEN** `crates/prism-ui/tests/allocations.rs` drives an app of a column of rows whose nodes carry paddings, sizes, minimum sizes and fills, measured by the test support's policies and steps, through 100 resizes alternating between two window sizes, each followed by a layout and a display list build
- **THEN** no allocation happens after the first iteration

### Requirement: The Rest of a Clean Chain Reuses Its Measurement
When a node is measured again with no change to its properties, modifier chain, measure policy or descendants, and one of its layout modifiers measures the rest of its chain under the constraints it gave it last time, the rest MUST reuse its last size, without running the layout modifiers after it or measuring the node's content.

#### Scenario: A resized node reuses the rest after a size
- **WHEN** a headless app of 400 × 300 composes `Column { Layout(content = { }, modifier = Modifier.size(20.dp).padding(2.dp)) { measurables, constraints -> println("measured ${constraints.minWidth}x${constraints.minHeight}"); layout(constraints.minWidth, constraints.minHeight) { } } }` and its script is `resize 300 200` then `dump`
- **THEN** stdout is the lines `measured 16x16`, once, `window 300x200`, `  column #1 0,0 20x20` and `    layout #2 0,0 20x20 size(20x20) padding(2)`

## MODIFIED Requirements

### Requirement: Layout and Drawing After a Reload
After the pass that follows a reload, the frame loop MUST lay out again every node measured by a measure policy or holding a `layout` modifier, as the reload may have changed the code they run, and build the display list only when that layout or the pass changed a node's bounds or what it draws. Matched nodes MUST keep their numbers and focus, and no frame MUST be presented when nothing visible changed.

#### Scenario: A focused field keeps its focus and text across a reload
- **WHEN** the typing app, run headless under `--watch`, gets the script `click 100 18`, `type "ab"`, an edit of `Text("Hello ${name.value}")` to `Text("Hi ${name.value}")`, then `type "c"` and `dump`
- **THEN** the dump holds `textfield #2 0,0 200x36 "abc" focused defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)` and `text #3 0,36 48x20 "Hi abc"`: both nodes kept their numbers, and the field its focus

#### Scenario: Editing a function a policy calls
- **WHEN** a headless app under `--watch` composes, in `app/main.prism`, `Spaced { Text("a"); Text("b") }`, where `Spaced` is a `Layout` that stacks its children with `gap()` pixels after each and `public fun gap(): Int = 0` is declared in `lib/gaps.prism`, and its script is `dump`, `edit lib/gaps.prism lib/gaps.prism.v2`, whose `gap()` returns 10, and `dump`
- **THEN** the first dump holds `text #3 0,20 8x20 "b"` and the second `layout #1 0,0 8x60` and `text #3 0,30 8x20 "b"`, although no composable body changed

#### Scenario: Editing a function a layout modifier calls
- **WHEN** a headless app under `--watch` composes, in `app/main.prism`, `Text("a", modifier = Modifier.layout { measurable, constraints -> val p = measurable.measure(constraints); layout(p.width + shift(), p.height) { p.place(shift(), 0) } })`, with `public fun shift(): Int = 0` declared in `lib/shifts.prism`, and its script is `dump`, `edit lib/shifts.prism lib/shifts.prism.v2`, whose `shift()` returns 10, and `dump`
- **THEN** the first dump holds `text #1 0,0 8x20 "a" layout` and the second `text #1 0,0 18x20 "a" layout`

### Requirement: Script Targets Are Found by Their Text
A target `"<text>" [<n>]` MUST name the n-th node, 1 by default and in tree order, whose text, or text field value, is exactly the string. The command MUST act at the centre of that node's content, where its steps placed it, rounded down, hit-testing from there. A target that matches no node MUST end the program with `event script line <n>: no node shows the text "<text>"`.

#### Scenario: A missing target
- **WHEN** the counter program runs headless with the script `click "Minus"`
- **THEN** stderr is exactly `panic: event script line 1: no node shows the text "Minus"` and a newline, and the exit status is 101

#### Scenario: Typing without a focused field
- **WHEN** the counter program runs headless with the script `type "x"`
- **THEN** stderr is exactly `panic: event script line 1: no text field is focused` and a newline, and the exit status is 101

#### Scenario: A text target follows an offset
- **WHEN** a headless app of 400 × 300 composes `Text("go", modifier = Modifier.offset(x = 100.dp).clickable { println("go") })` and its script is `click "go"`
- **THEN** stdout is `go` and a newline

### Requirement: Script Targets by Kind
Wherever an event script accepts a text target, it MUST also accept `<kind> [<n>]`: the n-th node, 1 by default and in tree order, of kind `column`, `row`, `box`, `text`, `spacer`, `textfield` or `layout`, or whose `clickable` or `toggleable` element has the role `button` or `checkbox`. The command MUST act, rounded down and hit-testing from there, at the centre of that node's content, where its steps placed it, or for a role, of that element's bounds.

#### Scenario: Clicking the second checkbox
- **WHEN** a headless app of 400 × 300 composes `compose fun Two() { val a = remember { mutableStateOf(false) }; val b = remember { mutableStateOf(false) }; Column { Checkbox(checked = a.value, onCheckedChange = { a.value = it }); Checkbox(checked = b.value, onCheckedChange = { b.value = it }) } }`, and its script is `click checkbox 2` then `dump`
- **THEN** stdout is the lines `window 400x300`, `  column #1 0,0 20x40`, `    box #2 0,0 20x20 toggleable(Checkbox off) size(20x20) border(2 #FF49454F RoundedCorner(2))` and `    box #3 0,20 20x20 toggleable(Checkbox on) size(20x20) background(#FF6750A4 RoundedCorner(2)) border(2 #FF6750A4 RoundedCorner(2))`

#### Scenario: Focusing a field by its kind
- **WHEN** the typing app of `ui`'s "Typing into a text field" runs headless with the script `click textfield`, `type "ab"` and `dump`
- **THEN** the dump holds `textfield #2 0,0 200x36 "ab" focused defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)`

#### Scenario: Clicking inside a custom layout by its kind
- **WHEN** a headless app of 400 × 300 composes `Diagonal { Button(onClick = { println("hit") }) { Text("go") } }`, with `Diagonal` as in "A custom layout places its children diagonally", and its script is `click layout`
- **THEN** stdout is `hit` and a newline
