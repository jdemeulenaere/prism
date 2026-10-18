# UI Specification Delta

## Purpose
The UI toolkit turns compositions into user interfaces: the `/prism/ui` composables and modifiers, the native node tree they drive, constraint layout, the display list, input dispatched to handlers owned by nodes, `runApp`'s frame loop, and the headless mode with event scripts and text dumps through which every behaviour is tested.

## ADDED Requirements

### Requirement: The Toolkit Is a Native Node Tree Without Window Code
The node tree, layout, display list, input and frame loop MUST be in a `prism-ui` crate that depends only on `prism-runtime` and `prism-compose`. Generated code MUST reach it only through its C entry points, called by symbol. No window system crate MUST be a dependency of `prism-ui` or of the backend.

#### Scenario: Headless programs link no window code
- **WHEN** a Rust test of `prism-ui` reads its `Cargo.toml`, and `cargo test -p prism-ui` builds the crate's tests
- **THEN** its only dependencies are `prism-runtime` and `prism-compose`, and the tests build and pass without any window crate

### Requirement: The UI Toolkit Is Imported
The composables, modifiers and values of the toolkit MUST be declared by the bundled module `/prism/ui`, outside the prelude, so that a file uses them only after `import /prism/ui`.

#### Scenario: UI toolkit is imported
- **WHEN** a file without imports declares `compose fun Screen() { Column { } }`
- **THEN** the type checker emits `E4001` for `Column`, and the file typechecks once it declares `import /prism/ui`

### Requirement: Running an App
`runApp(title, width, height, content)` MUST compose `content` once, lay it out in a window of `width` × `height` logical pixels, and then run the frame loop until the window closes or the event script ends. It MUST then drop its composition, releasing every node and the values nodes own, and return.

#### Scenario: runApp returns and frees everything
- **WHEN** `fun main() { runApp(width = 200, height = 100) { Text("hi") }; println("after") }` runs with `prism run --check-allocations --headless` and no event script
- **THEN** stdout is `after` and a newline, and the exit status is 0

### Requirement: One App at a Time
A thread MUST run at most one app. Calling `runApp` while an app runs on the thread, for instance from a handler, MUST end the program with `ui misuse: runApp called while an app is running`.

#### Scenario: runApp from a handler
- **WHEN** `compose fun Screen() { Button(onClick = { runApp { } }) { Text("nest") } }` is the content of a headless app whose script is `click "nest"`
- **THEN** stderr is exactly `panic: ui misuse: runApp called while an app is running` and a newline, and the exit status is 101

### Requirement: Apps Need a Window or Headless Mode
When `prism run` has no window system to offer and `--headless` is not given, `runApp` MUST end the program with `ui misuse: no window system is available; run with --headless`.

#### Scenario: No window and no headless mode
- **WHEN** `fun main() { runApp { Text("hi") } }` runs with `prism run --check-allocations main.prism` in a build without a window host
- **THEN** stderr is exactly `panic: ui misuse: no window system is available; run with --headless` and a newline, and the exit status is 101

### Requirement: Starting an App in a Composable Is Rejected
A call of `runApp` in a composable context MUST be rejected with `E4030` (`CompositionInComposable`), as calls of `composition` are. In a non-composable closure written inside a composable, it MUST be accepted.

#### Scenario: runApp inside a composable
- **WHEN** `compose fun Screen() { runApp { } }` is checked with `import /prism/ui`
- **THEN** the type checker emits `E4030` at `runApp`

### Requirement: Toolkit Composables Run Only in an App
A toolkit composable composed in a composition that `runApp` did not start MUST end the program with `ui misuse: /prism/ui composables run only inside runApp`.

#### Scenario: A toolkit composable in a plain composition
- **WHEN** `fun main() { val c = composition { Text("a") } }` runs with `import /prism/ui`
- **THEN** stderr is exactly `panic: ui misuse: /prism/ui composables run only inside runApp` and a newline, and the exit status is 101

### Requirement: Layout Composables
`/prism/ui` MUST declare `Column(modifier, verticalArrangement, horizontalAlignment, content)`, `Row(modifier, horizontalArrangement, verticalAlignment, content)` and `Box(modifier, contentAlignment, content)`, each with the defaults `Modifier`, `Arrangement.Top` or `Arrangement.Start`, and `Alignment.Start`, `Alignment.Top` or `Alignment.TopStart`, and a composable content lambda as the last parameter.

#### Scenario: Counter screen
- **WHEN** `import /prism/ui` and `compose fun Counter() { val c = remember { mutableStateOf(0) }; Column(modifier = Modifier.padding(16), verticalArrangement = Arrangement.spacedBy(8)) { Text("Count: ${c.value}"); Button(onClick = { c.value += 1 }) { Text("+") } } }` are checked
- **THEN** the file typechecks, and the `Button` closure captures `c` by copy

### Requirement: Text Takes Borrowed Text
`Text(text: &String, modifier: Modifier = Modifier, color: Color = Color.Unspecified, fontSize: Int = 16)` MUST show its text in one node. The text MUST be borrowed, so literals, templates and the contents of a state in a composable context are passed without a clone.

#### Scenario: Text of a state's string
- **WHEN** `compose fun Name(name: State<String>) { Text(name.value); Text("Hello ${name.value}") }` is checked with `import /prism/ui`
- **THEN** the file typechecks, without `E4017`

### Requirement: Buttons
`Button(onClick: escaping () -> Unit, modifier = Modifier, enabled: Bool = true, content: compose () -> Unit)` MUST lay its content out as a row centred inside a content padding of 24 pixels horizontally and 8 vertically, at least 58 × 40 pixels. It MUST draw a pill-shaped container of colour `#FF6750A4`, or `#1F000000` when disabled, and give its content the content colour white (`#61000000` when disabled).

#### Scenario: A button's size and label position
- **WHEN** a headless app of 400 × 300 composes `Button(onClick = { }) { Text("+") }` and its script is `dump`
- **THEN** the dump holds `button #1 0,0 58x40` and, one level deeper, `text #2 25,10 8x20 "+"`

### Requirement: Checkboxes
`Checkbox(checked: Bool, onCheckedChange: escaping (Bool) -> Unit, modifier = Modifier, enabled: Bool = true)` MUST be a 20 × 20 box. A click on an enabled checkbox MUST call `onCheckedChange` with the opposite of `checked`. The box MUST NOT change until the program passes a new `checked`.

#### Scenario: Toggling a checkbox
- **WHEN** a headless app of 400 × 300 composes `compose fun Task() { val done = remember { mutableStateOf(false) }; Row { Checkbox(checked = done.value, onCheckedChange = { done.value = it }); Text("milk") } }`, and its script is `click 10 10` then `dump`
- **THEN** stdout is the lines `window 400x300`, `  row #1 0,0 52x20`, `    checkbox #2 0,0 20x20 checked` and `    text #3 20,0 32x20 "milk"`

### Requirement: Text Fields
`TextField(value: &String, onValueChange: escaping (String) -> Unit, modifier = Modifier, enabled: Bool = true)` MUST show one unwrapped line of `value` inside an inner padding of 8 pixels, at least 200 pixels wide. It MUST show `value` only: an edit calls `onValueChange` with the new text, and the field changes when the program passes a new `value`.

#### Scenario: Typing into a text field
- **WHEN** a headless app of 400 × 300 composes `compose fun Form() { val name = remember { mutableStateOf("") }; Column { TextField(value = name.value, onValueChange = { name.value = it }); Text("Hello ${name.value}") } }`, and its script is `click 100 18`, `type "ab"`, `key backspace`, `type "c"` and `dump`
- **THEN** stdout is the lines `window 400x300`, `  column #1 0,0 200x56`, `    textfield #2 0,0 200x36 "ac" focused` and `    text #3 0,36 64x20 "Hello ac"`

#### Scenario: A field the program does not update
- **WHEN** the same app's `onValueChange` is `{ println(it) }` and its script is `click 100 18`, `type "ab"` and `dump`
- **THEN** stdout holds `a` and `b` on their own lines, and the dump shows the text field with no text

### Requirement: Spacers
`Spacer(modifier: Modifier)` MUST be a node without content, whose size comes from its modifier and its constraints only.

#### Scenario: A spacer between texts
- **WHEN** a headless app composes `Column { Text("a"); Spacer(modifier = Modifier.height(8)); Text("b") }` and its script is `dump`
- **THEN** the dump holds `spacer #3 0,20 0x8 height 8` and `text #4 0,28 8x20 "b"`

### Requirement: Modifier Is a Copy Value Built by Chained Calls
`Modifier` MUST be a `Copy` struct, and its companion value `Modifier` MUST be the empty modifier. Its functions (`padding`, `width`, `height`, `size`, `fillMaxWidth`, `fillMaxHeight`, `fillMaxSize`, `weight`, `background`, `border`) MUST each return a copy with one property set. The result MUST NOT depend on the order of calls for different properties.

#### Scenario: Modifier order does not matter
- **WHEN** `fun main() { println(Modifier.background(Color.Red).padding(8) == Modifier.padding(8).background(Color.Red)); println(Modifier == Modifier.padding(0)) }` runs with `import /prism/ui`
- **THEN** stdout is `true` and `true`, one per line

### Requirement: Modifiers Are Stable Parameters
A composable parameter of type `Modifier` MUST be stable, so a skippable composable called with an equal modifier MUST skip its body.

#### Scenario: A card with an equal modifier is skipped
- **WHEN** `compose fun Card(modifier: Modifier) { println("card"); Box(modifier = modifier) { } }` is called as `Card(modifier = Modifier.padding(4))` in a column beside `Text("${c.value}")` and a `Button` incrementing `c`, and the headless script is `click "+"`
- **THEN** stdout is `card` once: the recomposition after the click skips `Card`

### Requirement: Padding Accumulates
`padding(all = 0, horizontal = all, vertical = all, start = horizontal, top = vertical, end = horizontal, bottom = vertical)` MUST add its values to the modifier's padding on each side. Padding MUST be laid out inside the node's size, around its content.

#### Scenario: Chained paddings add up
- **WHEN** a headless app composes `Box(modifier = Modifier.padding(8).padding(4)) { Text("a") }` and its script is `dump`
- **THEN** the dump holds `box #1 0,0 32x44 padding 12` and `text #2 12,12 8x20 "a"`

#### Scenario: Horizontal padding
- **WHEN** a headless app composes `Box(modifier = Modifier.padding(horizontal = 8)) { Text("a") }` and its script is `dump`
- **THEN** the dump holds `box #1 0,0 24x20 padding 8,0,8,0` and `text #2 8,0 8x20 "a"`

### Requirement: Size Modifiers
`width(w)`, `height(h)` and `size(width, height = width)` MUST make a node's size exact, clamped into its constraints. `fillMaxWidth()`, `fillMaxHeight()` and `fillMaxSize()` MUST make it the maximum its constraints allow on that axis, when that maximum is bounded. A later call MUST override an earlier one for the same property.

#### Scenario: A size larger than the window is clamped
- **WHEN** a headless app of 400 × 300 composes `Box(modifier = Modifier.size(500)) { }` and its script is `dump`
- **THEN** the dump holds `box #1 0,0 400x300 width 500 height 500`

### Requirement: Weights Share the Remaining Space of a Row or Column
In a `Row` or `Column`, the children with a `weight` MUST share, by weight, the space that the children without one leave on the main axis, when the main axis is bounded. Elsewhere, `weight` MUST be ignored.

#### Scenario: A weighted text fills a row
- **WHEN** a headless app composes `Row(modifier = Modifier.width(200)) { Text("a", modifier = Modifier.weight(1.0)); Text("bb") }` and its script is `dump`
- **THEN** the dump holds `row #1 0,0 200x20 width 200`, `text #2 0,0 184x20 "a" weight 1.0` and `text #3 184,0 16x20 "bb"`

### Requirement: Backgrounds and Borders
`background(color, shape = RectangleShape)` and `border(width, color, shape = RectangleShape)` MUST draw behind the node's content over its whole bounds, padding included, whatever the order of the calls. `RectangleShape`, `CircleShape` and `RoundedCornerShape(radius)` MUST give corner radii of 0, half the smaller side, and `radius`.

#### Scenario: A rounded background around padding
- **WHEN** a headless app composes `Box(modifier = Modifier.padding(8).background(Color.Red, RoundedCornerShape(4))) { Text("a") }` and its script is `dump draw`
- **THEN** stdout is the lines `clear 400x300 #FFFFFFFF`, `rect 0,0 24x36 #FFFF0000 radius 4` and `text 8,8 size 16 #FF000000 "a"`, for a window of 400 × 300

### Requirement: Colors
`Color` MUST be a `Copy` struct holding an ARGB value in `argb`, built by `Color(0xAARRGGBB)`. Its companion value `Color` MUST hold `Black`, `DarkGray`, `Gray`, `LightGray`, `White`, `Red`, `Green`, `Blue`, `Yellow`, `Cyan`, `Magenta`, `Transparent` and `Unspecified` with Jetpack Compose's values, and `Unspecified` MUST be `Color(0x1_0000_0000)`.

#### Scenario: Named and constructed colours are equal
- **WHEN** `fun main() { println(Color.Red == Color(0xFFFF0000)); println(Color.Unspecified.argb) }` runs with `import /prism/ui`
- **THEN** stdout is `true` and `4294967296`, one per line

### Requirement: Content Colour Is Inherited
A `Text` whose colour is unspecified MUST be drawn in the content colour of its nearest ancestor that sets one, and in black when none does. `Button` MUST set its content colour.

#### Scenario: A button label is white
- **WHEN** the counter screen runs headless in a window of 400 × 300 with the script `dump draw`
- **THEN** stdout is the lines `clear 400x300 #FFFFFFFF`, `text 16,16 size 16 #FF000000 "Count: 0"`, `rect 16,44 58x40 #FF6750A4 radius 20` and `text 41,54 size 16 #FFFFFFFF "+"`

### Requirement: Arrangements and Alignments
`Arrangement` MUST offer `Top`, `Bottom`, `Start`, `End`, `Center`, `SpaceBetween`, `SpaceAround`, `SpaceEvenly` and `spacedBy(space)` on a row's or column's main axis, with integer positions. `Alignment` MUST offer the nine two-axis values (`TopStart` to `BottomEnd`, `Center`) and the one-axis `Start`, `CenterHorizontally`, `End`, `Top`, `CenterVertically` and `Bottom`.

#### Scenario: Column stacks children
- **WHEN** a headless app composes `Column(verticalArrangement = Arrangement.spacedBy(8)) { Text("a"); Text("b") }` and its script is `dump`
- **THEN** the dump holds `column #1 0,0 8x48 arrangement spacedBy(8)`, `text #2 0,0 8x20 "a"` and `text #3 0,28 8x20 "b"`: the second is placed 28 pixels below the first

#### Scenario: Space between two texts
- **WHEN** a headless app composes `Row(modifier = Modifier.width(100), horizontalArrangement = Arrangement.SpaceBetween) { Text("a"); Text("b") }` and its script is `dump`
- **THEN** the dump holds `text #2 0,0 8x20 "a"` and `text #3 92,0 8x20 "b"`

#### Scenario: Centred content in a box
- **WHEN** a headless app of 400 × 300 composes `Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("ab") }` and its script is `dump`
- **THEN** the dump holds `box #1 0,0 400x300 alignment Center fillMaxWidth fillMaxHeight` and `text #2 192,140 16x20 "ab"`

### Requirement: Constraint Layout in Logical Pixels
Layout MUST be one measure-and-place pass over the node tree. A parent passes minimum and maximum widths and heights to each child, the child returns its size, and the parent places it. Sizes and positions MUST be `Int` logical pixels. The window's root MUST pass 0 to its size on each axis, and place its children at its top-left corner.

#### Scenario: Padding around a column
- **WHEN** a headless app composes `Column(modifier = Modifier.padding(16)) { Text("Hi") }` and its script is `dump`
- **THEN** the dump holds `column #1 0,0 48x52 padding 16` and `text #2 16,16 16x20 "Hi"`

### Requirement: Text Measurement
Text MUST be measured through a font metrics interface. Headless runs MUST use fixed-advance metrics: every character advances half the font size, rounded down, and a line is 5/4 of the font size high, rounded down. Lines MUST break at each newline, and greedily at spaces when the next word would exceed the maximum width. A word wider than the line MUST stay whole.

#### Scenario: Wrapped text
- **WHEN** a headless app composes `Text("aa bb cc", modifier = Modifier.width(40))` and its script is `dump` then `dump draw`
- **THEN** the dump holds `text #1 0,0 40x40 "aa bb cc" width 40`, and the display list holds `text 0,0 size 16 #FF000000 "aa bb"` and `text 0,20 size 16 #FF000000 "cc"`

#### Scenario: A larger font
- **WHEN** a headless app composes `Text("abc", fontSize = 24)` and its script is `dump`
- **THEN** the dump holds `text #1 0,0 36x30 "abc" size 24`

### Requirement: Layout Reuses Clean Measurements
A node whose properties and descendants did not change since its last measurement, and that is measured with the same constraints, MUST reuse its last size without measuring its subtree.

#### Scenario: One changed text is measured alone
- **WHEN** a Rust test lays out a tree of a column of 100 texts with a measurer that counts the characters it measures, changes one text, and lays out again
- **THEN** the second layout measures only the changed text's characters

### Requirement: Nodes Follow Their Composables
Each toolkit composable call MUST own one node, created the first time its group is composed and reused while its group is matched. Its properties MUST be updated in place on later passes, and it MUST be removed when its group is removed. Nodes MUST be numbered from 1 per app in creation order, never reusing a number.

#### Scenario: Recomposition updates the node in place
- **WHEN** the counter screen runs headless with the script `click "+"` then `dump`
- **THEN** the dump holds `text #2 16,16 64x20 "Count: 1"`: the same node shows the new text

#### Scenario: A removed text is recreated as a new node
- **WHEN** `compose fun Toggle() { val on = remember { mutableStateOf(true) }; Column { if (on.value) { Text("shown") }; Button(onClick = { on.value = !on.value }) { Text("toggle") } } }` runs headless with the script `click "toggle"`, `dump`, `click "toggle"` and `dump`
- **THEN** the first dump holds no `shown` node and `button #3 0,0 96x40`, and the second holds `text #5 0,0 40x20 "shown"` and `button #3 0,20 96x40`

#### Scenario: Keyed items move their nodes
- **WHEN** `compose fun Items() { val reversed = remember { mutableStateOf(false) }; Column { val ids = if (reversed.value) listOf(3, 2, 1) else listOf(1, 2, 3); for (id in ids) { key(id) { Text("item $id") } }; Button(onClick = { reversed.value = true }) { Text("reverse") } } }` runs headless with the script `click "reverse"` then `dump`
- **THEN** the dump lists `text #4 0,0 48x20 "item 3"`, `text #3 0,20 48x20 "item 2"` and `text #2 0,40 48x20 "item 1"`, in this order

### Requirement: Nodes Own Their Values
A node MUST own the strings and handler closures it is given. Each one MUST be dropped when it is replaced, and the rest when the node is released.

#### Scenario: Replaced handlers are freed
- **WHEN** `compose fun Greeter() { val c = remember { mutableStateOf(0) }; val label = "clicked ${c.value}"; Button(onClick = { println(label); c.value += 1 }) { Text("greet") } }` runs headless under `--check-allocations` with the script `click "greet"` twice
- **THEN** stdout is `clicked 0` and `clicked 1`, one per line, and the exit status is 0: every replaced handler's string was freed

### Requirement: Pointer Clicks
A press MUST hit-test the laid-out tree topmost first, in reverse drawing order, for a button, checkbox or text field. A release that hits the same node MUST perform its click: call a button's `onClick`, toggle a checkbox, or focus a text field. A press that hits no text field MUST clear the focus.

#### Scenario: Click increments a counter
- **WHEN** the counter screen runs headless with the script `click "+"`, `click "+"` and `dump`
- **THEN** the dump holds a text node reading `Count: 2`, and the run passes the allocation check

#### Scenario: Releasing elsewhere does not click
- **WHEN** the counter screen runs headless with the script `press "+"`, `release 300 200` and `dump`
- **THEN** the dump holds `text #2 16,16 64x20 "Count: 0"`

#### Scenario: The topmost button is clicked
- **WHEN** a headless app composes `Box { Button(onClick = { println("under") }) { Text("under") }; Button(onClick = { println("over") }) { Text("over") } }` and its script is `click 10 20`
- **THEN** stdout is `over` and a newline

### Requirement: Disabled Controls Absorb Input
A button, checkbox or text field with `enabled = false` MUST be hit by presses as an enabled one is, and MUST call no handler and take no focus.

#### Scenario: A disabled button
- **WHEN** a headless app composes `Button(onClick = { println("clicked") }, enabled = false) { Text("Go") }` and its script is `click "Go"` then `dump`
- **THEN** stdout holds no `clicked`, and the dump holds `button #1 0,0 64x40 disabled`

### Requirement: Keyboard Input Goes to the Focused Text Field
A typed character MUST make the focused, enabled text field call `onValueChange` with its value followed by that character. Backspace MUST call it with the value without its last character, and do nothing when the value is empty. Enter MUST be consumed with no effect.

#### Scenario: Backspace on an empty field
- **WHEN** the typing app, with `onValueChange = { println(it) }`, runs the script `click 100 18`, `key backspace`, `key enter` and `dump`
- **THEN** stdout is the dump alone, which shows `textfield #2 0,0 200x36 focused`

### Requirement: Each Event Sees the Previous Event's Result
Events MUST be dispatched one at a time, outside composition. After each event, the frame loop MUST recompose every invalid scope and lay out again before it dispatches the next event.

#### Scenario: Two typed characters both arrive
- **WHEN** the typing app's script is `click 100 18`, `type "ab"` and `dump`
- **THEN** the dump shows the text field reading `ab`: the second character is appended to the value that the first one's recomposition passed

### Requirement: Handlers Run Outside Composition
Handlers MUST run as handlers of the app's composition, outside any pass, so they may write states, and the cells they create belong to the composition.

#### Scenario: A handler creates and writes a state
- **WHEN** a headless app composes `Button(onClick = { val s = mutableStateOf(41); s.value += 1; println(s.value) }) { Text("go") }` under `--check-allocations`, and its script is `click "go"`
- **THEN** stdout is `42` and a newline, and the exit status is 0: the cell is freed with the composition

### Requirement: Idle Apps Do No Work
When no scope is invalid, no layout is dirty and no event arrives, the frame loop MUST run no pass, no layout and no display list build.

#### Scenario: Frames without changes compose nothing
- **WHEN** `compose fun Screen() { println("compose"); Text("idle") }` runs headless with the script `frame`, `frame` and `dump`
- **THEN** stdout is `compose` once, followed by the dump

### Requirement: The Display List
After layout, when anything drawn or laid out changed, the toolkit MUST build a display list in tree order, parents before children. It MUST hold a clear of the window, each node's background and border, and its kind's own rectangles, borders and text runs, one text run per line.

#### Scenario: A display list dump
- **WHEN** a headless app of 400 × 300 composes `Box(modifier = Modifier.border(1, Color.Black)) { Text("ab") }` and its script is `dump draw`
- **THEN** stdout is the lines `clear 400x300 #FFFFFFFF`, `border 0,0 16x20 width 1 #FF000000` and `text 0,0 size 16 #FF000000 "ab"`

### Requirement: Headless Runs Follow an Event Script
A headless run MUST read its event script's commands in order and end the app after the last one: `click`, `press` and `release` at a text target or at `<x> <y>`, `type "<text>"`, `key backspace`, `key enter`, `resize <width> <height>`, `frame`, `dump`, `dump tree` and `dump draw`. Blank lines and `#` comments MUST be ignored.

#### Scenario: Scripted counter session
- **WHEN** the counter program runs headless with the script `# two clicks`, `click "+"`, `click "+"` and `dump`
- **THEN** stdout holds a tree whose text node reads `Count: 2`, and the run passes the allocation check

#### Scenario: Resizing the window
- **WHEN** a headless app composes `Box(modifier = Modifier.fillMaxSize()) { }` and its script is `resize 200 100` then `dump`
- **THEN** stdout is the lines `window 200x100` and `  box #1 0,0 200x100 fillMaxWidth fillMaxHeight`

### Requirement: Script Targets Are Found by Their Text
A target `"<text>" [<n>]` MUST name the n-th node, 1 by default and in tree order, whose text, or text field value, is exactly the string. The command MUST act at the centre of that node's bounds, rounded down, hit-testing from there. A target that matches no node MUST end the program with `event script line <n>: no node shows the text "<text>"`.

#### Scenario: A missing target
- **WHEN** the counter program runs headless with the script `click "Minus"`
- **THEN** stderr is exactly `panic: event script line 1: no node shows the text "Minus"` and a newline, and the exit status is 101

#### Scenario: Typing without a focused field
- **WHEN** the counter program runs headless with the script `type "x"`
- **THEN** stderr is exactly `panic: event script line 1: no text field is focused` and a newline, and the exit status is 101

### Requirement: Dumping the Laid-Out Tree
`dump` MUST print the line `window <width>x<height>`, then one line per node in tree order, indented by two spaces per depth: `<kind> #<number> <x>,<y> <width>x<height>`, with absolute bounds. Each node's non-default properties follow, in the order text, `color`, `size`, `checked`, `disabled`, `focused`, `arrangement`, `alignment`, `padding`, `width`, `height`, `fillMaxWidth`, `fillMaxHeight`, `weight`, `background`, `border`.

#### Scenario: A dump of the counter screen
- **WHEN** the counter screen runs headless in a window of 400 × 300 with the script `dump`
- **THEN** stdout is exactly the lines `window 400x300`, `  column #1 0,0 96x100 arrangement spacedBy(8) padding 16`, `    text #2 16,16 64x20 "Count: 0"`, `    button #3 16,44 58x40` and `      text #4 41,54 8x20 "+"`

### Requirement: Dumping the Display List
`dump draw` MUST print the display list, one command per line: `clear <width>x<height> <color>`, `rect <x>,<y> <width>x<height> <color>`, `border <x>,<y> <width>x<height> width <n> <color>` and `text <x>,<y> size <n> <color> "<text>"`. Rectangles and borders with rounded corners MUST end with `radius <r>`, and colours MUST be written `#AARRGGBB`.

#### Scenario: A rounded border
- **WHEN** a headless app composes `Box(modifier = Modifier.border(2, Color.Blue, CircleShape).size(40)) { }` and its script is `dump draw`
- **THEN** stdout holds `border 0,0 40x40 width 2 #FF0000FF radius 20`

### Requirement: Dumps Are Program Output
Dumps MUST be written to stdout through the program's output buffer, in order with what the program prints.

#### Scenario: Prints and dumps interleave
- **WHEN** `compose fun Screen() { println("composed"); Text("x") }` runs headless with the script `dump`, after `main` printed `start` before calling `runApp`
- **THEN** stdout is `start`, `composed`, `window 800x600` and `  text #1 0,0 8x20 "x"`, in this order

### Requirement: UI Runtime Errors Have No Location
The UI runtime errors, `ui misuse: <detail>` and `event script line <n>: <detail>`, MUST end the program as every runtime error does, without a location line. The misuse details MUST include `stale node handle`, `unknown node kind <k>` and `node property set outside composition` for uses that only the toolkit's own code can make.

#### Scenario: A stale node handle
- **WHEN** a Rust test calls the text setter entry point with the handle of a released node during a pass of an app
- **THEN** the runtime error `ui misuse: stale node handle` ends the test's child process with status 101

### Requirement: Node Identity Survives Unrelated Edits
A node MUST be identified by its node group, whose key comes from the toolkit composable's body and the node call's position in it, under the calling composable's group. A node MUST keep its number, focus and layout cache while that group is matched, and a node group composed with another kind MUST get a new node.

#### Scenario: Inserting a button before a text keeps the text's node
- **WHEN** `compose fun Screen() { val more = remember { mutableStateOf(false) }; Column { if (more.value) { Button(onClick = { }) { Text("new") } }; Text("kept"); Button(onClick = { more.value = true }) { Text("more") } } }` runs headless with the script `click "more"` then `dump`
- **THEN** the dump shows `kept` as node `#2`, the node it was created as

### Requirement: Layout and Drawing Take Linear Time
A full layout and a display list build MUST take time linear in the nodes and the characters of text measured. A relayout MUST take time linear in the dirty nodes, their ancestors and those ancestors' children.

#### Scenario: Layout timing benchmark
- **WHEN** `crates/prism-ui/tests/ui_timing.rs` runs with `PRISM_ASSERT_TIMING=1` on trees of 1,000, 10,000 and 100,000 nodes
- **THEN** each tenfold increase multiplies the full layout and display list times by at most 15, and relaying out after one text change of the 10,000-node tree takes under a tenth of its full layout

### Requirement: Steady-State Frames Allocate Nothing
Once its buffers have grown, an iteration of the frame loop whose event handler, pass, layout and display list create nothing new MUST allocate nothing in the toolkit.

#### Scenario: Allocation-counting test
- **WHEN** `crates/prism-ui/tests/allocations.rs` drives an app with a counting allocator through 100 events whose handler writes nothing, each followed by a pass, a layout and a display list build
- **THEN** no allocation happens after the first iteration

### Requirement: The Toolkit Adds Linear Compile Time
Checking and lowering calls of toolkit composables MUST take time linear in the calls and modifier chains.

#### Scenario: Compile timing of a large screen
- **WHEN** `compile_timing.rs` runs its `ui-calls` shape, one composable calling `n` toolkit composables with modifier chains inside a `Column`, with `PRISM_ASSERT_TIMING=1`
- **THEN** each doubling of `n` at most roughly doubles the front end and code generation times
