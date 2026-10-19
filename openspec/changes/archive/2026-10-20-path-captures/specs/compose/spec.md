## MODIFIED Requirements

### Requirement: Skipping Composables With Stable Parameters
A composable returning `Unit` whose parameters are all stable MUST be skippable. It MUST keep its arguments in its group's slots, `Copy` values by copy and function values as a keepable copy, and MUST skip its body, dropping the arguments it owns, when every argument is the same as the kept one (by `==` through the type's equality, by closure and captures for function values, or as its caller says) and its scope is valid. Nothing MUST be cloned to compare.

#### Scenario: Equal arguments skip the body
- **WHEN** `compose fun Badge(n: Int) { println("badge") }`, `fun screen(): compose () -> Unit = { Badge(1) }` and `fun main() { val c = composition(screen()); c.setContent(screen()) }` run
- **THEN** `badge` is printed only on the first composition

#### Scenario: Changed arguments run the body
- **WHEN** `compose fun Badge(n: Int) { println("badge $n") }`, `fun screen(n: Int): compose () -> Unit = { Badge(n) }` and `fun main() { val c = composition(screen(1)); c.setContent(screen(1)); c.setContent(screen(2)) }` run
- **THEN** stdout is `badge 1` and `badge 2`

#### Scenario: Handler capturing a Copy value is stable
- **WHEN** `compose fun Plus(onClick: escaping () -> Unit) { println("plus") }`, `fun screen(id: Int): compose () -> Unit = { Plus(onClick = { println(id) }) }` and `fun main() { val c = composition(screen(1)); c.setContent(screen(1)); c.setContent(screen(2)) }` run
- **THEN** `plus` is printed twice: the second pass builds a closure equal to the kept one, and the third one with another capture

#### Scenario: Handler capturing a state handle is stable
- **WHEN** `compose fun Plus(onClick: escaping () -> Unit) { println("plus") }` and `compose fun Screen(count: MutableState<Int>, tick: State<Int>) { println("screen ${tick.value}"); Plus(onClick = { count.value += 1 }) }` are declared and `fun main() { val count = mutableStateOf(0); val tick = mutableStateOf(0); val c = composition { Screen(count, tick) }; tick.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `screen 0`, `plus`, `screen 1` and `true`: the handler rebuilt with the same `count` handle equals the kept one, so `Plus` skips

#### Scenario: A handler owning a String is never kept
- **WHEN** `compose fun Plus(onClick: escaping () -> Unit) { println("plus") }`, `fun screen(): compose () -> Unit = { val s = "a" + "b"; Plus(onClick = { println(s) }) }` and `fun main() { val c = composition(screen()); c.setContent(screen()) }` run with `--check-allocations`
- **THEN** `plus` is printed twice, and the exit status is 0

#### Scenario: String parameters are always changed
- **WHEN** `compose fun Title(s: String) { println("title") }`, `fun screen(): compose () -> Unit = { Title("a") }` and `fun main() { val c = composition(screen()); c.setContent(screen()) }` run
- **THEN** `title` is printed twice

#### Scenario: A handler copying a field of a borrowed parameter is stable
- **WHEN** `struct Item { val id: Int, val title: String }`, `compose fun Plus(onClick: escaping () -> Unit) { println("plus") }` and `compose fun ItemRow(item: &Item, tick: State<Int>) { println("row ${tick.value}"); Plus(onClick = { println(item.id) }) }` are declared and `fun main() { val item = Item(1, "a" + "b"); val tick = mutableStateOf(0); val c = composition { ItemRow(item, tick) }; tick.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `row 0`, `plus`, `row 1` and `true`, and the exit status is 0: the handler owns only a copy of `item.id`, so the rebuilt handler equals the kept one and `Plus` skips
