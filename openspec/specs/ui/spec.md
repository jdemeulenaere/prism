# UI Specification

## Purpose
The UI toolkit turns compositions into user interfaces: the toolkit modules `/prism/compose/ui`, `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3`, split and named as Jetpack Compose's artifacts, with their composables and ordered modifier chains; the native node tree they drive and the handler table their modifiers name; constraint layout; the display list; input dispatched to handlers owned by nodes; `runApp`'s frame loop; and the headless mode with event scripts and text dumps through which every behaviour is tested. Unless a scenario names its imports, its program imports the four toolkit modules.

## Requirements

### Requirement: The Toolkit Is a Native Node Tree Without Window Code
The node tree, layout, display list, input and frame loop MUST be in a `prism-ui` crate that depends only on `prism-runtime` and `prism-compose`. Generated code MUST reach it only through its C entry points, called by symbol. No window system crate MUST be a dependency of `prism-ui` or of the backend.

#### Scenario: Headless programs link no window code
- **WHEN** a Rust test of `prism-ui` reads its `Cargo.toml`, and `cargo test -p prism-ui` builds the crate's tests
- **THEN** its only dependencies are `prism-runtime` and `prism-compose`, and the tests build and pass without any window crate

### Requirement: The Toolkit Modules Follow Compose's Artifacts
The toolkit MUST be declared by four bundled modules outside the prelude, one per Jetpack Compose artifact: `/prism/compose/ui` (`compose-ui`), `/prism/compose/foundation` (`compose-foundation`), `/prism/compose/layout` (`compose-foundation-layout`) and `/prism/compose/material3` (`compose-material3`). Each name MUST be declared by the module of the artifact that declares it in Compose, so that a file uses it only after importing that module.

#### Scenario: UI toolkit is imported
- **WHEN** a file without imports declares `compose fun Screen() { Column { } }`
- **THEN** the type checker emits `E4001` for `Column`, and the file typechecks once it declares `import /prism/compose/layout`

#### Scenario: Names live in their Compose module
- **WHEN** a file declares `import /prism/compose/ui`, `import /prism/compose/layout` and `compose fun Screen() { Box(modifier = Modifier.background(Color.Red)) { } }`
- **THEN** the type checker emits `E4006` for `background`, and the file typechecks once it also declares `import /prism/compose/foundation`

### Requirement: Running an App
`runApp(title, width, height, content)` MUST compose `content` once, lay it out in a window of `width` × `height` logical pixels, and then run the frame loop until the window closes or the event script ends. It MUST then drop its composition, releasing every node, every handler and the values nodes own, and return.

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
- **WHEN** `compose fun Screen() { runApp { } }` is checked with `import /prism/compose/ui`
- **THEN** the type checker emits `E4030` at `runApp`

### Requirement: Toolkit Composables Run Only in an App
A toolkit composable composed in a composition that `runApp` did not start MUST end the program with `ui misuse: toolkit composables run only inside runApp`.

#### Scenario: A toolkit composable in a plain composition
- **WHEN** `fun main() { val c = composition { Text("a") } }` runs
- **THEN** stderr is exactly `panic: ui misuse: toolkit composables run only inside runApp` and a newline, and the exit status is 101

### Requirement: Layout Composables
`/prism/compose/layout` MUST declare `Column(modifier, verticalArrangement, horizontalAlignment, content)`, `Row(modifier, horizontalArrangement, verticalAlignment, content)` and `Box(modifier, contentAlignment, content)`, each with the defaults `Modifier`, `Arrangement.Top` or `Arrangement.Start`, and `Alignment.Start`, `Alignment.Top` or `Alignment.TopStart`, and a composable content lambda with the receiver `ColumnScope`, `RowScope` or `BoxScope` as the last parameter.

#### Scenario: Counter screen
- **WHEN** `compose fun Counter() { val c = remember { mutableStateOf(0) }; Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { Text("Count: ${c.value}"); Button(onClick = { c.value += 1 }) { Text("+") } } }` is checked
- **THEN** the file typechecks, and the `Button` closure captures `c` by copy

### Requirement: Text Takes Borrowed Text
`Text(text: &String, modifier = Modifier, color = Color.Unspecified, fontSize = TextUnit.Unspecified, style = MaterialTheme.typography.bodyLarge)` MUST show its text in one text node, through `BasicText`. The text MUST be borrowed, so literals, templates and a state's contents in a composable context pass without a clone. A specified `color` or `fontSize` MUST take precedence over the style's, and an unspecified font size MUST be 16.

#### Scenario: Text of a state's string
- **WHEN** `compose fun Name(name: State<String>) { Text(name.value); Text("Hello ${name.value}") }` is checked
- **THEN** the file typechecks, without `E4017`

#### Scenario: An explicit colour wins over the style
- **WHEN** a headless app composes `Text("a", color = Color.Blue, style = TextStyle(Color.Red, 24.sp))` and its script is `dump`
- **THEN** the dump holds `text #1 0,0 12x30 "a" color #FF0000FF fontSize 24`

### Requirement: Buttons
`Button(onClick, modifier = Modifier, enabled = true, shape = ButtonDefaults.shape, colors = ButtonDefaults.buttonColors(), content: compose RowScope.() -> Unit)` MUST be a box node with the caller's modifier followed by `background(container colour, shape)` and `clickable` with the role `Button`, its content colour that of `colors`, holding a row with `defaultMinSize(58.dp, 40.dp)` and `padding(horizontal = 24.dp, vertical = 8.dp)`, arranged `Center` and aligned `CenterVertically`.

#### Scenario: A button's size and label position
- **WHEN** a headless app of 400 × 300 composes `Button(onClick = { }) { Text("+") }` and its script is `dump`
- **THEN** the dump holds `box #1 0,0 58x40 contentColor #FFFFFFFF background(#FF6750A4 RoundedCorner(50%)) clickable(Button)`, then `row #2 0,0 58x40 arrangement Center alignment CenterVertically defaultMinSize(58x40) padding(24,8,24,8)` one level deeper, and `text #3 25,10 8x20 "+"` one level deeper again

### Requirement: Button Colours and Shape
`ButtonDefaults.buttonColors()` MUST give the container and content colours `#FF6750A4` and `#FFFFFFFF`, and `#1F1D1B20` and `#611D1B20` when disabled, and `ButtonDefaults.shape` MUST have corners of 50 percent.

#### Scenario: A button label is drawn in its content colour
- **WHEN** a headless app of 400 × 300 composes `Button(onClick = { }) { Text("+") }` and its script is `dump draw`
- **THEN** stdout is the lines `clear 400x300 #FFFFFFFF`, `rect 0,0 58x40 #FF6750A4 radius 20` and `text 25,10 size 16 #FFFFFFFF "+"`

### Requirement: Checkboxes
`Checkbox(checked, onCheckedChange: escaping (Bool) -> Unit, modifier = Modifier, enabled = true)` MUST be a box node with the caller's modifier, `toggleable` with the role `Checkbox` and `size(20.dp)`, then a background and a border of 2 in `#FF6750A4` when checked, or a border of 2 in `#FF49454F`, with corners of 2. A click on an enabled checkbox MUST call `onCheckedChange` with the opposite of `checked`, and the box MUST NOT change until the program passes a new `checked`.

#### Scenario: Toggling a checkbox
- **WHEN** a headless app of 400 × 300 composes `compose fun Task() { val done = remember { mutableStateOf(false) }; Row { Checkbox(checked = done.value, onCheckedChange = { done.value = it }); Text("milk") } }`, and its script is `click 10 10` then `dump`
- **THEN** stdout is the lines `window 400x300`, `  row #1 0,0 52x20`, `    box #2 0,0 20x20 toggleable(Checkbox on) size(20x20) background(#FF6750A4 RoundedCorner(2)) border(2 #FF6750A4 RoundedCorner(2))` and `    text #3 20,0 32x20 "milk"`

### Requirement: Text Fields
`TextField(value: &String, onValueChange, modifier = Modifier, enabled = true, textStyle = MaterialTheme.typography.bodyLarge)` MUST be a `BasicTextField` whose modifier is the caller's followed by `defaultMinSize(minWidth = 200.dp)`, a frame of `#FF79747E` and `#FF6750A4`, and `padding(8.dp)`. A text field node MUST show one unwrapped line of `value`, and only `value`: an edit calls `onValueChange` with the new text, and the field changes when the program passes a new `value`.

#### Scenario: Typing into a text field
- **WHEN** a headless app of 400 × 300 composes `compose fun Form() { val name = remember { mutableStateOf("") }; Column { TextField(value = name.value, onValueChange = { name.value = it }); Text("Hello ${name.value}") } }`, and its script is `click 100 18`, `type "ab"`, `key backspace`, `type "c"` and `dump`
- **THEN** stdout is the lines `window 400x300`, `  column #1 0,0 200x56`, `    textfield #2 0,0 200x36 "ac" focused defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)` and `    text #3 0,36 64x20 "Hello ac"`

#### Scenario: A field the program does not update
- **WHEN** the same app's `onValueChange` is `{ println(it) }` and its script is `click 100 18`, `type "ab"` and `dump`
- **THEN** stdout holds `a` and `b` on their own lines, and the dump shows the text field with no text

### Requirement: Surfaces
`Surface(modifier = Modifier, shape: Shape = RectangleShape, color: Color = MaterialTheme.colorScheme.surface, contentColor: Color = MaterialTheme.colorScheme.onSurface, content: compose () -> Unit)` MUST be a box node with the caller's modifier followed by `background(color, shape)`, whose content colour is `contentColor`.

#### Scenario: A surface sets its content colour
- **WHEN** a headless app of 400 × 300 composes `Surface(color = Color.Black, contentColor = Color.White) { Text("a") }` and its script is `dump draw`
- **THEN** stdout is the lines `clear 400x300 #FFFFFFFF`, `rect 0,0 8x20 #FF000000` and `text 0,0 size 16 #FFFFFFFF "a"`

### Requirement: Spacers
`Spacer(modifier: Modifier)` MUST be a node without content, whose size comes from its modifier and its constraints only.

#### Scenario: A spacer between texts
- **WHEN** a headless app composes `Column { Text("a"); Spacer(modifier = Modifier.height(8.dp)); Text("b") }` and its script is `dump`
- **THEN** the dump holds `spacer #3 0,20 0x8 height(8)` and `text #4 0,28 8x20 "b"`

### Requirement: Modifier Is an Ordered Chain
`Modifier` MUST be a `Copy` value holding a sequence of elements, and its companion value `Modifier` MUST be the empty chain. Each modifier function MUST return a copy with one element appended, and `then(other)` MUST append the elements of `other` in order. Two chains MUST be equal exactly when they hold equal elements in the same order. No modifier function MUST be composable.

#### Scenario: Modifier order matters
- **WHEN** `fun main() { println(Modifier.background(Color.Red).padding(8.dp) == Modifier.padding(8.dp).background(Color.Red)); println(Modifier.padding(8.dp).background(Color.Red) == Modifier.padding(8.dp).background(Color.Red)); println(Modifier.padding(4.dp).then(Modifier.size(10.dp)) == Modifier.padding(4.dp).size(10.dp)) }` runs
- **THEN** stdout is `false`, `true` and `true`, one per line

#### Scenario: Order decides what a background covers
- **WHEN** a headless app of 400 × 300 composes `Column { Box(modifier = Modifier.padding(8.dp).background(Color.Red)) { Text("a") }; Box(modifier = Modifier.background(Color.Blue).padding(8.dp)) { Text("b") } }` and its script is `dump draw`
- **THEN** stdout is the lines `clear 400x300 #FFFFFFFF`, `rect 8,8 8x20 #FFFF0000`, `text 8,8 size 16 #FF000000 "a"`, `rect 0,36 24x36 #FF0000FF` and `text 8,44 size 16 #FF000000 "b"`

### Requirement: Modifier Chains Hold at Most Sixteen Elements
Appending a seventeenth element to a chain MUST end the program with `ui misuse: a modifier chain holds more than 16 elements`.

#### Scenario: A seventeenth element
- **WHEN** `fun main() { var m = Modifier; for (i in 1..17) { m = m.padding(1.dp) }; println("never") }` runs
- **THEN** stdout is empty, stderr is exactly `panic: ui misuse: a modifier chain holds more than 16 elements` and a newline, and the exit status is 101

### Requirement: Modifiers Are Stable Parameters
A composable parameter of type `Modifier` MUST be stable, so a skippable composable called with an equal chain MUST skip its body. A handler element MUST compare equal to one built on another pass from a closure with the same code and equal captures, all of them `Copy`, including when the chain it is compared with was kept by a composable that ran again because its chain changed, and when the captures hold chains naming handlers themselves.

#### Scenario: A card with an equal modifier is skipped
- **WHEN** `compose fun Card(modifier: Modifier) { println("card"); Box(modifier = modifier) { } }` is called as `Card(modifier = Modifier.padding(4.dp))` in a column beside `Text("${c.value}")` and a `Button` incrementing `c`, and the headless script is `click "+"`
- **THEN** stdout is `card` once: the recomposition after the click skips `Card`

#### Scenario: A card with an equal clickable modifier is skipped
- **WHEN** a headless app of 400 × 300 composes `compose fun Screen() { val c = remember { mutableStateOf(0) }; Column { Text("${c.value}"); Card(modifier = Modifier.size(40.dp).clickable { c.value += 1 }) } }`, with `Card` as above, and its script is `click 20 40`, `click 20 40` and `dump`
- **THEN** stdout is `card` once, followed by the lines `window 400x300`, `  column #1 0,0 40x60`, `    text #2 0,0 8x20 "2"` and `    box #3 0,20 40x40 size(40x40) clickable`

#### Scenario: A card skips again after its toggleable modifier changed
- **WHEN** a headless app of 400 × 300 composes `compose fun Screen() { val c = remember { mutableStateOf(0) }; val t = remember { mutableStateOf(false) }; Column { Text("${c.value}"); Card(modifier = Modifier.size(20.dp).toggleable(value = t.value) { t.value = it }); Box(modifier = Modifier.size(20.dp).clickable { c.value += 1 }) { } } }`, with `Card` as above, and its script is `click 5 25`, `click 5 45`, `click 5 45` and `dump`
- **THEN** stdout is `card` twice, for the first pass and after the toggle, followed by the dump, whose text reads `2`: the passes after the two clicks skip `Card` again

#### Scenario: A handler that captures a clickable modifier
- **WHEN** a headless app of 400 × 300 composes `compose fun Screen() { val c = remember { mutableStateOf(0) }; val inner = Modifier.clickable { c.value += 1 }; Column { Text("${c.value}"); Box(modifier = Modifier.size(20.dp).clickable { println(inner.count) }) { }; Box(modifier = inner.size(20.dp)) { } } }` and its script is `click 5 45`, `click 5 25` and `dump`
- **THEN** stdout is `1` followed by the dump, whose text reads `1`, and the exit status is 0: the second pass compares the first box's handlers, and with them the handler of the modifier they capture

### Requirement: Padding Accumulates
`padding(all = 0.dp, horizontal = all, vertical = all, start = horizontal, top = vertical, end = horizontal, bottom = vertical)` MUST lay out the rest of the chain inside the padding, so that chained paddings add up.

#### Scenario: Chained paddings add up
- **WHEN** a headless app composes `Box(modifier = Modifier.padding(8.dp).padding(4.dp)) { Text("a") }` and its script is `dump`
- **THEN** the dump holds `box #1 0,0 32x44 padding(8) padding(4)` and `text #2 12,12 8x20 "a"`

#### Scenario: Horizontal padding
- **WHEN** a headless app composes `Box(modifier = Modifier.padding(horizontal = 8.dp)) { Text("a") }` and its script is `dump`
- **THEN** the dump holds `box #1 0,0 24x20 padding(8,0,8,0)` and `text #2 8,0 8x20 "a"`

### Requirement: Size Modifiers
`width(w)`, `height(h)` and `size(width, height = width)` MUST make the size of the rest of the chain exact, clamped into its constraints. An element MUST constrain the elements after it, so a padding before a size adds to the node's size and a padding after it does not.

#### Scenario: A size larger than the window is clamped
- **WHEN** a headless app of 400 × 300 composes `Box(modifier = Modifier.size(500.dp)) { }` and its script is `dump`
- **THEN** the dump holds `box #1 0,0 400x300 size(500x500)`

#### Scenario: Padding before or after a size
- **WHEN** a headless app composes `Column { Box(modifier = Modifier.padding(8.dp).size(40.dp)) { Text("a") }; Box(modifier = Modifier.size(40.dp).padding(8.dp)) { Text("b") } }` and its script is `dump`
- **THEN** the dump holds `box #2 0,0 56x56 padding(8) size(40x40)`, `text #3 8,8 8x20 "a"`, `box #4 0,56 40x40 size(40x40) padding(8)` and `text #5 8,64 8x20 "b"`

### Requirement: Minimum and Filling Sizes
`defaultMinSize(minWidth, minHeight)` MUST raise a minimum of 0 to each specified value. `fillMaxWidth(fraction = 1.0)`, `fillMaxHeight(fraction)` and `fillMaxSize(fraction)` MUST make the size that fraction of the maximum the constraints allow on that axis, rounded down, when that maximum is bounded.

#### Scenario: Minimum and fractional sizes
- **WHEN** a headless app of 400 × 300 composes `Column { Box(modifier = Modifier.defaultMinSize(minWidth = 30.dp)) { Text("a") }; Box(modifier = Modifier.fillMaxWidth(0.5)) { } }` and its script is `dump`
- **THEN** the dump holds `box #2 0,0 30x20 defaultMinSize(30x-)` and `box #4 0,20 200x0 fillMaxWidth(0.5)`

### Requirement: Weights Share the Remaining Space of a Row or Column
In a row or column, the children with a `weight(weight, fill = true)` MUST share, by weight, the space that the children without one leave on the main axis, when the main axis is bounded: exactly with `fill`, and at most without it.

#### Scenario: A weighted text fills a row
- **WHEN** a headless app composes `Row(modifier = Modifier.width(200.dp)) { Text("a", modifier = Modifier.weight(1.0)); Text("bb") }` and its script is `dump`
- **THEN** the dump holds `row #1 0,0 200x20 width(200)`, `text #2 0,0 184x20 "a" weight(1.0)` and `text #3 184,0 16x20 "bb"`

### Requirement: A Child's Alignment
A child's `align(alignment)` MUST override its parent's alignment for that child, on the axes the parent aligns: horizontal in a column, vertical in a row, both in a box.

#### Scenario: Aligning one child
- **WHEN** a headless app composes `Column { Column(modifier = Modifier.width(100.dp)) { Text("a"); Text("b", modifier = Modifier.align(Alignment.End)) }; Box(modifier = Modifier.size(100.dp)) { Text("c", modifier = Modifier.align(Alignment.BottomEnd)) } }` and its script is `dump`
- **THEN** the dump holds `text #3 0,0 8x20 "a"`, `text #4 92,20 8x20 "b" align(End)` and `text #6 92,120 8x20 "c" align(BottomEnd)`

### Requirement: Scoped Modifiers Need Their Scope
`weight` MUST be available only in the content of a row or a column, and `align` only in the content of a box, a row or a column, as extensions of `Modifier` whose context is `RowScope`, `ColumnScope` or `BoxScope`, resolved against the innermost implicit receiver.

#### Scenario: A scoped modifier outside its scope
- **WHEN** `compose fun Screen() { Box { Text("a", modifier = Modifier.weight(1.0)) } }` is checked
- **THEN** the type checker rejects `weight`, which needs a `RowScope` or `ColumnScope` receiver, with the error `receiver-lambdas` reports for a missing context

### Requirement: Backgrounds and Borders
`background(color, shape = RectangleShape)` and `border(width, color, shape = RectangleShape)` MUST draw over the bounds of the rest of the chain at their position, in chain order, before the node's own content. `RectangleShape`, `CircleShape`, `RoundedCornerShape(size)` and `RoundedCornerShape(percent = p)` MUST give corner radii of 0, half the smaller side, `size`, and `p` percent of the smaller side.

#### Scenario: A rounded background around padding
- **WHEN** a headless app composes `Box(modifier = Modifier.background(Color.Red, RoundedCornerShape(4.dp)).padding(8.dp)) { Text("a") }` and its script is `dump draw`
- **THEN** stdout is the lines `clear 400x300 #FFFFFFFF`, `rect 0,0 24x36 #FFFF0000 radius 4` and `text 8,8 size 16 #FF000000 "a"`, for a window of 400 × 300

### Requirement: Clickable and Toggleable Modifiers
`clickable(enabled = true, role = null, onClick)` MUST make the bounds of the rest of the chain at its position a click target that calls `onClick`, and `toggleable(value, enabled = true, role = null, onValueChange)` one that calls `onValueChange(!value)`.

#### Scenario: A clickable box
- **WHEN** a headless app composes `Box(modifier = Modifier.size(40.dp).clickable { println("box") }) { }` and its script is `click 10 10`
- **THEN** stdout is `box` and a newline

#### Scenario: The clickable area follows the chain
- **WHEN** a headless app composes `Box(modifier = Modifier.padding(20.dp).clickable { println("hit") }.size(10.dp)) { }` and its script is `click 5 5` then `click 25 25`
- **THEN** stdout is `hit` once

#### Scenario: A toggleable text
- **WHEN** a headless app composes `compose fun Switcher() { val on = remember { mutableStateOf(false) }; Text(if (on.value) "on" else "off", modifier = Modifier.toggleable(value = on.value) { on.value = it }) }` and its script is `click "off"` then `dump`
- **THEN** the dump holds `text #1 0,0 16x20 "on" toggleable(on)`

### Requirement: Handlers Are Owned by the Nodes That Use Them
A handler given to `clickable`, `toggleable` or a text field MUST be held in the handler table of `prism-ui`, named by a `HandlerId`, and owned by the node that claims it, which drops it when it takes another or is released. A handler no node claims MUST be dropped at the end of the pass in which it was made. A second node given a handler MUST get a duplicate when the handler owns only `Copy` captures.

#### Scenario: A Copy handler shared by two nodes
- **WHEN** a headless app composes `compose fun Two() { val c = remember { mutableStateOf(0) }; val m = Modifier.size(10.dp).clickable { c.value += 1 }; Column { Box(modifier = m) { }; Box(modifier = m) { }; Text("${c.value}") } }` and its script is `click 5 5`, `click 5 15` and `dump`
- **THEN** the dump holds `text #4 0,20 8x20 "2"`, and the exit status is 0

### Requirement: Handler Misuse
Making a handler while no app runs on the thread, giving a handler that owns captures that are not `Copy` to two nodes, and naming a dropped handler MUST end the program with `ui misuse: modifier handlers are made only while runApp runs`, `ui misuse: a handler that owns captures that are not Copy was given to two nodes` and `ui misuse: stale modifier handler`.

#### Scenario: A non-Copy handler cannot be shared
- **WHEN** a headless app composes `compose fun Two() { val label = "x"; val m = Modifier.clickable { println(label) }; Box(modifier = m) { }; Box(modifier = m) { } }` and no script
- **THEN** stderr is exactly `panic: ui misuse: a handler that owns captures that are not Copy was given to two nodes` and a newline, and the exit status is 101

#### Scenario: Handlers need a running app
- **WHEN** `fun main() { val m = Modifier.clickable { } }` runs
- **THEN** stderr is exactly `panic: ui misuse: modifier handlers are made only while runApp runs` and a newline, and the exit status is 101

### Requirement: Units
`Dp` and `TextUnit` MUST be `Copy` structs holding whole logical pixels in `value`, and `n.dp` and `n.sp` MUST build them from an `Int`.

#### Scenario: Building units
- **WHEN** `fun main() { println(16.dp == Dp(16)); println(16.dp.value + 2.sp.value) }` runs
- **THEN** stdout is `true` and `18`, one per line

### Requirement: Colors
`Color` MUST be a `Copy` struct holding an ARGB value in `argb`, built by `Color(0xAARRGGBB)`. Its companion value `Color` MUST hold `Black`, `DarkGray`, `Gray`, `LightGray`, `White`, `Red`, `Green`, `Blue`, `Yellow`, `Cyan`, `Magenta`, `Transparent` and `Unspecified` with Jetpack Compose's values, and `Unspecified` MUST be `Color(0x1_0000_0000)`.

#### Scenario: Named and constructed colours are equal
- **WHEN** `fun main() { println(Color.Red == Color(0xFFFF0000)); println(Color.Unspecified.argb) }` runs
- **THEN** stdout is `true` and `4294967296`, one per line

### Requirement: Content Colour Is Inherited
A text whose colour is unspecified MUST be drawn in the content colour of its nearest ancestor that sets one, and in black when none does. `Surface` and `Button` MUST set their node's content colour.

#### Scenario: A button label is white
- **WHEN** the counter screen runs headless in a window of 400 × 300 with the script `dump draw`
- **THEN** stdout is the lines `clear 400x300 #FFFFFFFF`, `text 16,16 size 16 #FF000000 "Count: 0"`, `rect 16,44 58x40 #FF6750A4 radius 20` and `text 41,54 size 16 #FFFFFFFF "+"`

### Requirement: Arrangements and Alignments
`Arrangement` MUST offer `Top`, `Bottom`, `Start`, `End`, `Center`, `SpaceBetween`, `SpaceAround`, `SpaceEvenly` and `spacedBy(space)` on a row's or column's main axis, with integer positions. `Alignment` MUST offer the nine two-axis values (`TopStart` to `BottomEnd`, `Center`) and the one-axis `Start`, `CenterHorizontally`, `End`, `Top`, `CenterVertically` and `Bottom`.

#### Scenario: Column stacks children
- **WHEN** a headless app composes `Column(verticalArrangement = Arrangement.spacedBy(8.dp)) { Text("a"); Text("b") }` and its script is `dump`
- **THEN** the dump holds `column #1 0,0 8x48 arrangement spacedBy(8)`, `text #2 0,0 8x20 "a"` and `text #3 0,28 8x20 "b"`: the second is placed 28 pixels below the first

#### Scenario: Space between two texts
- **WHEN** a headless app composes `Row(modifier = Modifier.width(100.dp), horizontalArrangement = Arrangement.SpaceBetween) { Text("a"); Text("b") }` and its script is `dump`
- **THEN** the dump holds `text #2 0,0 8x20 "a"` and `text #3 92,0 8x20 "b"`

#### Scenario: Centred content in a box
- **WHEN** a headless app of 400 × 300 composes `Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text("ab") }` and its script is `dump`
- **THEN** the dump holds `box #1 0,0 400x300 alignment Center fillMaxSize` and `text #2 192,140 16x20 "ab"`

### Requirement: Constraint Layout in Logical Pixels
Layout MUST be one measure-and-place pass over the node tree: a parent passes minimum and maximum widths and heights to each child, which returns its size, and the parent places it. In a node, each modifier element MUST constrain and size the elements after it, from the first, and the kind MUST then measure the content, clamped to what remains. Sizes and positions MUST be `Int` logical pixels. The window's root MUST pass 0 to its size on each axis and place its children at its top-left corner.

#### Scenario: Padding around a column
- **WHEN** a headless app composes `Column(modifier = Modifier.padding(16.dp)) { Text("Hi") }` and its script is `dump`
- **THEN** the dump holds `column #1 0,0 48x52 padding(16)` and `text #2 16,16 16x20 "Hi"`

### Requirement: Text Measurement
Text MUST be measured through a font metrics interface. Headless runs MUST use fixed-advance metrics: every character advances half the font size, rounded down, and a line is 5/4 of the font size high, rounded down. Lines MUST break at each newline, and greedily at spaces when the next word would exceed the maximum width. A word wider than the line MUST stay whole.

#### Scenario: Wrapped text
- **WHEN** a headless app composes `Text("aa bb cc", modifier = Modifier.width(40.dp))` and its script is `dump` then `dump draw`
- **THEN** the dump holds `text #1 0,0 40x40 "aa bb cc" width(40)`, and the display list holds `text 0,0 size 16 #FF000000 "aa bb"` and `text 0,20 size 16 #FF000000 "cc"`

#### Scenario: A larger font
- **WHEN** a headless app composes `Text("abc", fontSize = 24.sp)` and its script is `dump`
- **THEN** the dump holds `text #1 0,0 36x30 "abc" fontSize 24`

### Requirement: Layout Reuses Clean Measurements
A node whose properties, modifier chain and descendants did not change since its last measurement, and that is measured with the same constraints, MUST reuse its last size without measuring its subtree.

#### Scenario: One changed text is measured alone
- **WHEN** a Rust test lays out a tree of a column of 100 texts with a measurer that counts the characters it measures, changes one text, and lays out again
- **THEN** the second layout measures only the changed text's characters

### Requirement: Nodes Follow Their Composables
Each node call of a toolkit composable MUST own one node, created the first time its group is composed and reused while its group is matched. Its properties MUST be updated in place on later passes, and it MUST be removed when its group is removed. Nodes MUST be numbered from 1 per app in creation order, never reusing a number.

#### Scenario: Recomposition updates the node in place
- **WHEN** the counter screen runs headless with the script `click "+"` then `dump`
- **THEN** the dump holds `text #2 16,16 64x20 "Count: 1"`: the same node shows the new text

#### Scenario: A removed text is recreated as a new node
- **WHEN** `compose fun Toggle() { val on = remember { mutableStateOf(true) }; Column { if (on.value) { Text("shown") }; Button(onClick = { on.value = !on.value }) { Text("toggle") } } }` runs headless with the script `click "toggle"`, `dump`, `click "toggle"` and `dump`
- **THEN** the first dump holds no `shown` node and `box #3 0,0 96x40`, and the second holds `text #6 0,0 40x20 "shown"` and `box #3 0,20 96x40`

#### Scenario: Keyed items move their nodes
- **WHEN** `compose fun Items() { val reversed = remember { mutableStateOf(false) }; Column { val ids = if (reversed.value) listOf(3, 2, 1) else listOf(1, 2, 3); for (id in ids) { key(id) { Text("item $id") } }; Button(onClick = { reversed.value = true }) { Text("reverse") } } }` runs headless with the script `click "reverse"` then `dump`
- **THEN** the dump lists `text #4 0,0 48x20 "item 3"`, `text #3 0,20 48x20 "item 2"` and `text #2 0,40 48x20 "item 1"`, in this order

### Requirement: Nodes Own Their Values
A node MUST own the strings it is given and the handlers it claims. Each one MUST be dropped when it is replaced, and the rest when the node is released.

#### Scenario: Replaced handlers are freed
- **WHEN** `compose fun Greeter() { val c = remember { mutableStateOf(0) }; val label = "clicked ${c.value}"; Button(onClick = { println(label); c.value += 1 }) { Text("greet") } }` runs headless under `--check-allocations` with the script `click "greet"` twice
- **THEN** stdout is `clicked 0` and `clicked 1`, one per line, and the exit status is 0: every replaced handler's string was freed

### Requirement: Pointer Clicks
A press MUST hit-test the laid-out tree topmost first, in reverse drawing order, for a `clickable` or `toggleable` element, in the bounds at its position, or a text field. A release that hits the same target MUST perform its click: call a `clickable`'s `onClick`, call a `toggleable`'s `onValueChange` with the opposite of its value, or focus a text field. A press that hits no text field MUST clear the focus.

#### Scenario: Click increments a counter
- **WHEN** the counter screen runs headless with the script `click "+"`, `click "+"` and `dump`
- **THEN** the dump holds a text node reading `Count: 2`, and the run passes the allocation check

#### Scenario: Releasing elsewhere does not click
- **WHEN** the counter screen runs headless with the script `press "+"`, `release 300 200` and `dump`
- **THEN** the dump holds `text #2 16,16 64x20 "Count: 0"`

#### Scenario: The topmost button is clicked
- **WHEN** a headless app composes `Box { Button(onClick = { println("under") }) { Text("under") }; Button(onClick = { println("over") }) { Text("over") } }` and its script is `click 10 20`
- **THEN** stdout is `over` and a newline

### Requirement: Disabled Targets Absorb Input
A `clickable` or `toggleable` element with `enabled = false`, or a text field with `enabled = false`, MUST be hit by presses as an enabled one is, and MUST call no handler and take no focus.

#### Scenario: A disabled button
- **WHEN** a headless app composes `Button(onClick = { println("clicked") }, enabled = false) { Text("Go") }` and its script is `click "Go"` then `dump`
- **THEN** stdout holds no `clicked`, and the dump holds `box #1 0,0 64x40 contentColor #611D1B20 background(#1F1D1B20 RoundedCorner(50%)) clickable(Button disabled)`

### Requirement: Keyboard Input Goes to the Focused Text Field
A typed character MUST make the focused, enabled text field call `onValueChange` with its value followed by that character. Backspace MUST call it with the value without its last character, and do nothing when the value is empty. Enter MUST be consumed with no effect.

#### Scenario: Backspace on an empty field
- **WHEN** the typing app, with `onValueChange = { println(it) }`, runs the script `click 100 18`, `key backspace`, `key enter` and `dump`
- **THEN** stdout is the dump alone, which shows `textfield #2 0,0 200x36 focused defaultMinSize(200x-) textFieldFrame(#FF79747E #FF6750A4) padding(8)`

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
After layout, when anything drawn or laid out changed, the toolkit MUST build a display list in tree order, parents before children. It MUST hold a clear of the window, then for each node its drawing elements in chain order, each at the bounds of its position in the chain, and then its kind's own text runs and caret, one text run per line.

#### Scenario: A display list dump
- **WHEN** a headless app of 400 × 300 composes `Box(modifier = Modifier.border(1.dp, Color.Black)) { Text("ab") }` and its script is `dump draw`
- **THEN** stdout is the lines `clear 400x300 #FFFFFFFF`, `border 0,0 16x20 width 1 #FF000000` and `text 0,0 size 16 #FF000000 "ab"`

### Requirement: Headless Runs Follow an Event Script
A headless run MUST read its event script's commands in order and end the app after the last one: `click`, `press` and `release` at a text target or at `<x> <y>`, `type "<text>"`, `key backspace`, `key enter`, `resize <width> <height>`, `frame`, `dump`, `dump tree` and `dump draw`. Blank lines and `#` comments MUST be ignored.

#### Scenario: Scripted counter session
- **WHEN** the counter program runs headless with the script `# two clicks`, `click "+"`, `click "+"` and `dump`
- **THEN** stdout holds a tree whose text node reads `Count: 2`, and the run passes the allocation check

#### Scenario: Resizing the window
- **WHEN** a headless app composes `Box(modifier = Modifier.fillMaxSize()) { }` and its script is `resize 200 100` then `dump`
- **THEN** stdout is the lines `window 200x100` and `  box #1 0,0 200x100 fillMaxSize`

### Requirement: Script Targets Are Found by Their Text
A target `"<text>" [<n>]` MUST name the n-th node, 1 by default and in tree order, whose text, or text field value, is exactly the string. The command MUST act at the centre of that node's bounds, rounded down, hit-testing from there. A target that matches no node MUST end the program with `event script line <n>: no node shows the text "<text>"`.

#### Scenario: A missing target
- **WHEN** the counter program runs headless with the script `click "Minus"`
- **THEN** stderr is exactly `panic: event script line 1: no node shows the text "Minus"` and a newline, and the exit status is 101

#### Scenario: Typing without a focused field
- **WHEN** the counter program runs headless with the script `type "x"`
- **THEN** stderr is exactly `panic: event script line 1: no text field is focused` and a newline, and the exit status is 101

### Requirement: Dumping the Laid-Out Tree
`dump` MUST print `window <width>x<height>`, then one line per node in tree order, indented by two spaces per depth: `<kind> #<number> <x>,<y> <width>x<height>`, with absolute bounds and a kind among `column`, `row`, `box`, `text`, `spacer` and `textfield`. Its non-default properties MUST follow, in the order text, `color`, `fontSize`, `contentColor`, `disabled`, `focused`, `arrangement` and `alignment`, then its modifier elements in chain order, each with its arguments in parentheses.

#### Scenario: A dump of the counter screen
- **WHEN** the counter screen runs headless in a window of 400 × 300 with the script `dump`
- **THEN** stdout is exactly the lines `window 400x300`, `  column #1 0,0 96x100 arrangement spacedBy(8) padding(16)`, `    text #2 16,16 64x20 "Count: 0"`, `    box #3 16,44 58x40 contentColor #FFFFFFFF background(#FF6750A4 RoundedCorner(50%)) clickable(Button)`, `      row #4 16,44 58x40 arrangement Center alignment CenterVertically defaultMinSize(58x40) padding(24,8,24,8)` and `        text #5 41,54 8x20 "+"`

### Requirement: Dumping the Display List
`dump draw` MUST print the display list, one command per line: `clear <width>x<height> <color>`, `rect <x>,<y> <width>x<height> <color>`, `border <x>,<y> <width>x<height> width <n> <color>` and `text <x>,<y> size <n> <color> "<text>"`. Rectangles and borders with rounded corners MUST end with `radius <r>`, and colours MUST be written `#AARRGGBB`.

#### Scenario: A rounded border
- **WHEN** a headless app composes `Box(modifier = Modifier.border(2.dp, Color.Blue, CircleShape).size(40.dp)) { }` and its script is `dump draw`
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
A full layout and a display list build MUST take time linear in the nodes, their modifier elements and the characters of text measured. A relayout MUST take time linear in the dirty nodes, their ancestors and those ancestors' children.

#### Scenario: Layout timing benchmark
- **WHEN** `crates/prism-ui/tests/ui_timing.rs` runs with `PRISM_ASSERT_TIMING=1` on trees of 1,000, 10,000 and 100,000 nodes
- **THEN** each tenfold increase multiplies the full layout and display list times by at most 15, and relaying out after one text change of the 10,000-node tree takes under a tenth of its full layout

### Requirement: Steady-State Frames Allocate Nothing
Once its buffers have grown, an iteration of the frame loop whose event handler, pass, layout and display list create nothing new MUST allocate nothing in the toolkit, including the registration, comparison and release of handlers that own only `Copy` captures in one word.

#### Scenario: Allocation-counting test
- **WHEN** `crates/prism-ui/tests/allocations.rs` drives an app with a counting allocator through 100 events whose handler writes nothing, each followed by a pass that registers a clickable handler, a layout and a display list build
- **THEN** no allocation happens after the first iteration

### Requirement: The Toolkit Adds Linear Compile Time
Checking and lowering calls of toolkit composables MUST take time linear in the calls and modifier chains.

#### Scenario: Compile timing of a large screen
- **WHEN** `compile_timing.rs` runs its `ui-calls` shape, one composable calling `n` toolkit composables with modifier chains inside a `Column`, with `PRISM_ASSERT_TIMING=1`
- **THEN** each doubling of `n` at most roughly doubles the front end and code generation times

## Design Rationale


Decisions from `ui-toolkit` (planned on 2026-10-18 and revised on 2026-10-19 by user direction to follow Jetpack Compose's APIs and modules, implemented and reviewed on 2026-10-19, and logged in `docs/decisions-for-review.md`):

- **A `prism-ui` crate behind C entry points, with per-thread state** (D1, D2). The node arena and tree, the handler table, layout, the display list, input, the frame loop, the headless host and its scripts live in `prism-ui`, which depends only on `prism-runtime` and `prism-compose`, so headless programs and its tests build without a window crate (`desktop-window` adds one). Generated code reaches it through `UI_FUNCTIONS`, searched after the runtime's and the compose runtime's tables. Its state is a `thread_local!`, and nothing in it is borrowed while program code runs: handlers are copied out of their entries before they are called, released values are dropped after the state is let go, and handler equality copies both function values out before it runs their equality entry, which may compare handler ids again (found in review: such a re-entrant comparison aborted the process). Rejected: UI code in `prism-compose`, a locked global state, one crate with the window.
- **Toolkit composables are Prism code over two node intrinsics** (D3, D24). `/prism/compose/ui` declares, with `package` visibility, `Node(kind, update, content)`, `LeafNode(kind, update)`, `NodeId`, six node kinds and the `prism_ui_*` setters, so the toolkit modules of `prism.compose` share them and no program reaches them; `update` runs on every pass and each setter compares before it invalidates layout or drawing. Material components have no nodes of their own: a `Button` is a clickable `Surface` box holding a `Row`, a `Checkbox` a toggleable box, a `TextField` a framed `BasicTextField`, so dumps show the nodes Compose's tree would hold and a user component can do what `Button` does. Rejected: composables in Rust (no skipping or hot reload), a public node layer (a stopgap API that `Layout` and `Modifier.Node` should replace), a Rust kind per component.
- **Compose's names, parameter order, defaults and modules** (D5, D21, user direction). Each composable and modifier has Compose's signature minus the parameters left out, so Compose code ports by editing imports and `1f`; the deviations each have a reason: `&String` text, no overloading (one `padding` whose defaults read earlier parameters, `size(width, height = width)`, `RoundedCornerShape(size, percent)`), one `Alignment` and one `Arrangement` type for lack of nested types, no composition locals (`MaterialTheme` is a value and content colour is inherited at draw time), whole-pixel units, required `Box` content. One bundled module per Compose artifact (`ui`, `foundation`, `layout` for foundation-layout, `material3`), all in the package directory `prism/compose/` so `package` visibility shares the node layer; Compose's sub-packages fold into their artifact, since Prism imports modules, not declarations. Only `/prism/compose/runtime` is in the prelude. Rejected: one `/prism/ui` module, a module per Compose package, toolkit names in every file's scope.
- **`Modifier` is Compose's ordered chain in a `Copy` value** (D6, user direction, replacing `compose-architecture` D18). Up to 16 `ModifierElement`s in two blocks of eight, about 650 bytes, compared element by element, so `padding(8.dp).background(c)` and `background(c).padding(8.dp)` differ in layout, drawing and `==`, nothing allocates and a `Modifier` parameter is stable. A seventeenth element is `ui misuse`, since chains are values built across files and no limit can be checked statically. Rejected: the first design's order-insensitive property struct, an owned list (move-only, never stable), interned chains (nothing could free them), capacities of 8 or 32.
- **Modifiers are plain data; handlers live in a table** (D23, D25, user direction). No modifier composes, remembers or reads state, unlike the deprecated `Modifier.composed`; an element creates or updates state in its node. A handler given to `clickable`, `toggleable` or a text field moves into `prism-ui`'s handler table and the element holds a `Copy` `HandlerId` whose equality compares the handlers' code and captures, so `Modifier.clickable { c.value += 1 }` built on two passes is equal and its receiver skips. An entry is pending until a node claims it, owned by that node, and freed when replaced, with its node, or unclaimed at the end of its pass; a second node gets a duplicate of a keepable handler, and a non-keepable one shared is a misuse. Ids kept by skippable composables must stay live (review fix): a handler whose environment is one word owning nothing is interned, so equal ones are one id on every pass; for other handlers a node keeps its entries only when its whole new chain is equal, as the composable passing it keeps its old argument, and otherwise takes the entries the chain names. Rejected: closures inside elements (a non-`Copy` modifier), a composition slot per `clickable`, ids that never compare equal, counted sharing, forwarding released ids.
- **Units, colours and companion values** (D4, D7, D9). `Dp` and `TextUnit` are one-`Int` structs of whole logical pixels built by the intrinsic properties `16.dp` and `14.sp`, lowered as constructions; integer layout keeps dumps exact. `Color` is an ARGB `Int` with Compose's named colours, `Unspecified` lying outside 32 bits. Companion values (`type-system`) give Compose's exact spellings: `Modifier.padding(16.dp)`, `modifier: Modifier = Modifier`, `Color.Red` beside `Color(0xFF336699)`, `Arrangement.spacedBy(8.dp)`, `TextStyle.Default`. The theme is Material 3's baseline light scheme as a value, until composition locals exist. Rejected: `Int` sizes, `Double` units, enum colours, lower-case values.
- **Borrowed text** (D8). `Text` and `TextField` take `&String`, so literals, templates and a state's contents pass without `.clone()`; a node copies the bytes only when they changed. Rejected: an owned `String` (`E4017` for `Text(name.value)`).
- **Scoped content lambdas** (D22, user direction). `Column`, `Row` and `Box` take `compose ColumnScope.() -> Unit` and the like, and `weight` and `align` are extensions whose context is the scope, resolved against the innermost receiver only (`receiver-lambdas`), so `Modifier.weight(1.0)` outside a row or column is `E4034`. Rejected: plain `weight` with a run-time error, an explicit scope argument.
- **Constraint layout over the chain, with incremental relayout** (D11, D12). Each node folds its chain from the first element (padding, sizes, minimum sizes, fills; weights and `align` are parent data, the first one counting), then its kind's policy measures the content with Compose's arrangements and alignments in integer pixels, remainders dropped. Relayout climbs from the recorded dirty nodes and runs a parent's policy only when a child's size changed, so a same-width text change costs O(depth); a randomized test compares incremental layouts with full ones. Text goes through a `TextMeasurer`; headless runs use fixed advances (half the font size per character, lines of 5/4 of it), so dumps are arithmetic and depend on no font file, and `desktop-window` chooses its window metrics. Rejected: `taffy`, floating-point layout, re-measuring every frame, Inter's metrics headless, measure policies in Prism.
- **A display list and content colour** (D9, D13). One walk, parents first, emits each node's drawing elements in chain order at the bounds recorded at their position, then its text runs and caret, into reused buffers; an unspecified text colour takes the nearest ancestor's content colour, set by `Surface`, so a `Button`'s label is white without composition locals. Rejected: drawing from the tree in the renderer, ASCII-art snapshots.
- **Input** (D14). Hit testing walks in reverse drawing order for `clickable` and `toggleable` elements at their chain bounds and text fields; a release over the pressed target clicks it, a disabled target absorbs input, and a press that hits no text field clears the focus. A text field shows only its `value` and calls `onValueChange` with each edit. One pass and layout follow every event, so `type "ab"` appends to the value the first character's pass produced (revising §5's batched pass). Rejected: batched passes, press-time clicks, hit testing by kind, a field-owned buffer.
- **`runApp`, hosts and headless scripts** (D15, D16, D17). `runApp` is an intrinsic, so `E4030` sees it and its content gets a root content adapter; its loop blocks on a `Host` only when nothing is invalid or dirty, so idle apps do no work, and `desktop-window` supplies a window host. `prism run --headless[=WxH] --events <script>` parses the script before compiling, names targets by their text so scripts survive layout changes, and prints dumps through the program's output buffer, in order with `println`. UI errors have no location (`ui misuse`, `event script line <n>`): their detail names the construct. Rejected: a Prism frame loop, headless by default, PNG goldens, parsing scripts at `runApp`, ignoring missing targets.
- **Costs** (D19, D26). A toolkit composable costs a node group and a few comparing setters per pass, a steady-state frame allocates nothing (a counting-allocator test), layout and display list builds are linear (about 0.44 and 0.25 ms for 10,000 nodes, release build) and a click of a 1,000-text app costs about 0.19 ms headless. The compiler pays constant work per companion lookup, `.dp`, node call and `runApp` call; `ui-calls` checks and compiles linearly, about 0.07 ms per call in the borrow check once default generators borrow only what they read (`mir`).
