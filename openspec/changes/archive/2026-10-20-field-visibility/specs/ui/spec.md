## ADDED Requirements

### Requirement: Modifier Chains Are Built Only Through Modifier Functions
The fields of `Modifier` (`count`, `first` and `second`) and of `ModifierElements` (`e0` to `e7`) MUST be `private` to `/prism/compose/ui`. A program MUST build and combine chains only through `Modifier` and the modifier functions (`padding`, `size`, `clickable`, `then`, ...): constructing either struct or reading its fields outside that module MUST be `E4007`.

#### Scenario: A chain cannot be built or read by hand
- **WHEN** a file importing `/prism/compose/ui` declares `fun count(m: &Modifier): Int = m.count` and `fun empty(): Modifier = Modifier(0, Modifier.first, Modifier.second)`
- **THEN** the type checker emits `E4007` at `count`, at the construction of `Modifier`, and at `first` and `second`, each naming a private field of `prism/compose/ui`

#### Scenario: Chains built through modifier functions still compare
- **WHEN** a file importing `/prism/compose/ui`, `/prism/compose/layout` and `/prism/core` runs `fun main() { println(Modifier.padding(4.dp).size(8.dp) == Modifier.padding(4.dp).then(Modifier.size(8.dp))) }` with `--check-allocations`
- **THEN** stdout is `true`, and the exit status is 0

## MODIFIED Requirements

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
- **WHEN** a headless app of 400 × 300 composes `compose fun Screen() { val c = remember { mutableStateOf(0) }; val inner = Modifier.clickable { c.value += 1 }; Column { Text("${c.value}"); Box(modifier = Modifier.size(20.dp).clickable { println(inner != Modifier) }) { }; Box(modifier = inner.size(20.dp)) { } } }` and its script is `click 5 45`, `click 5 25` and `dump`
- **THEN** stdout is `true` followed by the dump, whose text reads `1`, and the exit status is 0: the second pass compares the first box's handlers, and with them the handler of the modifier they capture
