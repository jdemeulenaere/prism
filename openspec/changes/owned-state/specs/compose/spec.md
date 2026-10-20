## ADDED Requirements

### Requirement: Remembering Owned Values in a Composable
`remember(calculation)` MUST be a composable that runs `calculation` the first time its position in its group is composed and keeps the result in a slot. On every later pass it MUST give, without running `calculation`, a copy of the kept value when its type is `Copy`, and otherwise a shared borrow of it. The slot MUST drop the value when it is released. The type MUST hold no borrow, and `calculation` MUST NOT be a composable context.

#### Scenario: Remembered value survives recomposition
- **WHEN** `compose fun C(label: String) { val id = remember { println("computed"); 7 }; println(label + id.toString()) }`, `fun screen(): compose () -> Unit = { C("x") }` and `fun main() { val c = composition(screen()); c.setContent(screen()); c.setContent(screen()) }` run
- **THEN** stdout is `computed` followed by `x7` three times, one per line

#### Scenario: Remembering a non-Copy value
- **WHEN** `compose fun Leaf() { }`, `compose fun C(label: String) { val s = remember { println("computed"); "a" + "b" }; Leaf(); println(s.length) }`, `fun screen(): compose () -> Unit = { C("x") }` and `fun main() { val c = composition(screen()); c.setContent(screen()) }` run with `--check-allocations`
- **THEN** stdout is `computed`, `2` and `2`, one per line, and the exit status is 0: `s` borrows the kept `String`, past the call of `Leaf`, and the slot frees it when the composition is disposed

#### Scenario: Remembering a value that holds a borrow rejected
- **WHEN** `compose fun C(r: &Int) { val s = remember<&Int> { r } }` is checked
- **THEN** the type checker reports a borrow in a type argument (`E4022`); without the type argument, `remember { r }` remembers a copy of the `Int`

#### Scenario: Remembered state survives recomposition
- **WHEN** `compose fun Counter(tick: &State<Int>) { val clicks = remember { println("created"); mutableStateOf(7) }; println("tick ${tick.value} clicks ${clicks.value}") }` is declared and `fun main() { val tick = mutableStateOf(0); val c = composition { Counter(tick) }; tick.value += 1; println(c.recompose()) }` runs
- **THEN** stdout is `created`, `tick 0 clicks 7`, `tick 1 clicks 7` and `true`, one per line

#### Scenario: Forgotten state frees its value
- **WHEN** `compose fun Item(n: Int) { val label = remember(n) { mutableStateOf("item $n") }; println(label.value) }` and `fun screen(n: Int): compose () -> Unit = { Item(n) }` are declared and `fun main() { val c = composition(screen(1)); c.setContent(screen(2)); print(c.dump()) }` runs with `--check-allocations`
- **THEN** stdout starts with `item 1` and `item 2`, the dump shows `Item`'s group ending with the slot `slot MutableState<String>`, and the exit status is 0

#### Scenario: A removed group frees its state
- **WHEN** `compose fun Child() { val s = remember { mutableStateOf("a" + "b") }; println(s.value) }` and `compose fun Parent(show: &State<Bool>) { if (show.value) { Child() } }` are declared and `fun main() { val show = mutableStateOf(true); val c = composition { Parent(show) }; show.value = false; println(c.recompose()); print(c.dump()) }` runs with `--check-allocations`
- **THEN** stdout is `ab`, `true` and a dump with no group for `Child`, and the exit status is 0

### Requirement: State Objects Are Owned Values
`MutableState<T>` and `State<T>` MUST be move-only for every `T`, neither `Copy` nor `Clone`, and invariant in `T`. A state object MUST own exactly one state, whose value lives in a runtime cell that never moves, so moving the object never moves its value. `==` of two state objects MUST compare their identity, never their values.

#### Scenario: A state held by a struct in a list
- **WHEN** `struct Todo { val title: String, val done: MutableState<Bool> }` is declared and `fun main() { val todos = listOf(Todo("a" + "b", mutableStateOf(false)), Todo("c" + "d", mutableStateOf(false))); todos[1].done.value = true; println(todos[0].done.value); println(todos[1].done.value) }` runs with `--check-allocations`
- **THEN** stdout is `false` and `true`, one per line, and the exit status is 0: each state is freed once, with its item

#### Scenario: Assigning a state moves it
- **WHEN** `fun main() { val s = mutableStateOf(1); val t = s; println(s.value) }` is checked
- **THEN** the borrow checker emits `E5001` at `s` in `println(s.value)`

#### Scenario: States compare by identity
- **WHEN** `fun main() { val a = mutableStateOf(1); val b = mutableStateOf(1); println(a == b); println(a == a) }` runs
- **THEN** stdout is `false` and `true`, one per line

### Requirement: A State's Value Is a Place
For a non-`Copy` `T`, `s.value` MUST be a place of the state object `s`, reached by value or through any reference: it MAY be read in place and borrowed shared in any context, and outside composable contexts borrowed mutably, used as a `mut fun` receiver and written in part. A borrow of it MUST borrow `s`. Moving it out or taking it apart MUST be rejected with `E4017`.

#### Scenario: Iterating a list state in a composable
- **WHEN** `struct Todo { val title: String }` and `compose fun Todos(todos: &State<List<Todo>>) { println("count ${todos.value.size}"); for (todo in todos.value) { println(todo.title) } }` are declared and `fun main() { val todos = mutableStateOf(listOf(Todo("a"), Todo("b"))); val c = composition { Todos(todos) } }` runs with `--check-allocations`
- **THEN** stdout is `count 2`, `a` and `b`, one per line, and the exit status is 0

#### Scenario: Changing a list state in place outside composition
- **WHEN** `fun add(todos: &MutableState<List<Int>>) { todos.value.add(todos.value.size + 1) }` is declared and `fun main() { val todos = mutableStateOf(listOf(1)); add(todos); add(todos); println(todos.value.size); println(todos.value[2]) }` runs with `--check-allocations`
- **THEN** stdout is `3` and `3`, one per line, and the exit status is 0

#### Scenario: Moving a state's value out rejected
- **WHEN** `compose fun C(s: &State<String>) { val t = s.value }` is checked
- **THEN** the type checker emits `E4017`, saying that the value of a state cannot be moved out, with help suggesting a borrow or `.clone()`

#### Scenario: Taking a state's value apart rejected
- **WHEN** `struct U { val name: String, val n: Int }` and `compose fun C(u: &State<U>) { val (name, n) = u.value }` are checked
- **THEN** the type checker emits `E4017`, saying that the value of the state `u` cannot be taken apart, with help destructuring its borrow `&u.value`

#### Scenario: A borrowed value keeps its state borrowed
- **WHEN** `fun take(s: MutableState<List<Int>>) {}` is declared and `fun f() { val s = mutableStateOf(listOf(1)); val items = &s.value; take(s); println(items.size) }` is checked
- **THEN** the borrow checker emits `E5005` at `s` in `take(s)`, labelling the borrow `&s.value` and its later use

### Requirement: Borrows of a State's Value Outside Composition Hold Its Flag
Outside composable contexts, a shared borrow of a non-`Copy` state value MUST hold the state's flag shared, and a mutable borrow, a `mut fun` call on the value or a write to part of it MUST hold it exclusively, from the borrow to the end of its statement, or to the end of the block of the `val` or `var` it initializes. A `Copy` read and a whole-value write MUST hold the flag only during their call.

#### Scenario: Two references to one state that conflict
- **WHEN** `fun f(a: &MutableState<List<Int>>, b: &MutableState<List<Int>>) { val items = &a.value; b.value.add(3); println(items.size) }` is declared and `fun main() { val s = mutableStateOf(listOf(1)); f(s, s) }` runs
- **THEN** stderr is `panic: state accessed while borrowed` and the location of `b.value`, and the exit status is 101

#### Scenario: A flag ends with its statement
- **WHEN** `fun g(a: &MutableState<List<Int>>, b: &MutableState<List<Int>>) { println(a.value.size); b.value.add(2); println(a.value.size) }` is declared and `fun main() { val s = mutableStateOf(listOf(1)); g(s, s) }` runs with `--check-allocations`
- **THEN** stdout is `1` and `2`, one per line, and the exit status is 0

### Requirement: Dropping a State Retires It
Dropping a state object MUST retire its state: invalidate the scopes registered as its readers, forget them, and make every explicit weak reference to it give `null`. Until its cell is freed, a retired state's value MUST stay valid, and reads and writes through weak handles MUST keep working; a write to it MUST invalidate nothing.

#### Scenario: A dropped state recomposes its readers
- **WHEN** `fun consume(s: MutableState<Int>) {}` is declared and `fun main() { val s = mutableStateOf(5); val ref = s.weak(); val c = composition { println("read ${ref.get()?.value ?: -1}") }; consume(s); println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `read 5`, `read -1` and `true`, one per line, and the exit status is 0: the root content read the state through the weak reference, so dropping the state invalidated it

#### Scenario: A weak reference to a dropped state is null
- **WHEN** `fun consume(s: MutableState<Int>) {}` is declared and `fun main() { val s = mutableStateOf(1); val ref = s.weak(); println(ref.get()?.value ?: -1); consume(s); println(ref.get()?.value ?: -1) }` runs with `--check-allocations`
- **THEN** stdout is `1` and `-1`, one per line, and the exit status is 0

### Requirement: Retired States Are Freed After the Next Pass
A retired state's cell MUST be freed, its value dropped, at the end of the first pass on the thread that starts after it was retired, after that pass's releases. When no composition is live on the thread, it MUST be freed when it is retired or when the last composition is disposed. Under `--check-allocations`, every retired cell MUST be freed before the allocation check reports.

#### Scenario: A retired cell outlives the pass that retired it
- **WHEN** a Rust test retires a cell with a recording drop entry during a pass of a live composition, records a mark after that pass, runs a second pass and records a second mark
- **THEN** the drop is recorded once, between the two marks

#### Scenario: A state dropped with no composition is freed at once
- **WHEN** a Rust test retires a cell with a recording drop entry while no composition is live, and records a mark
- **THEN** the drop is recorded once, before the mark

### Requirement: Explicit Weak References
`s.weak()` MUST give a `WeakReference` to the state object `s`: one `Copy` word that does not keep the state. `ref.get()` MUST give `null` once the state is retired, and otherwise a shared borrow of a state object that reaches the same state, valid for as long as `ref` is borrowed.

#### Scenario: A stored handler reaches a state through a weak reference
- **WHEN** `struct Clicker { val onClick: () -> Unit }` and `fun clicker(count: &MutableState<Int>): Clicker { val ref = count.weak(); return Clicker({ ref.get()?.update { it + 1 } }) }` are declared and `fun main() { val count = mutableStateOf(0); val c = clicker(count); c.onClick(); c.onClick(); println(count.value) }` runs with `--check-allocations`
- **THEN** stdout is `2`, and the exit status is 0

#### Scenario: A weak reference outlives its state
- **WHEN** the same `Clicker` and `clicker` are declared with `fun consume(s: MutableState<Int>) {}`, and `fun main() { val count = mutableStateOf(0); val c = clicker(count); consume(count); c.onClick(); println("done") }` runs with `--check-allocations`
- **THEN** stdout is `done`, and the exit status is 0: the handler found no state and did nothing

### Requirement: Owned State Operations Take Constant Time
Retiring a state, taking and ending a flag, making a weak handle and resolving one MUST take amortized constant time in the number of cells and the size of the composition, and a pass MUST free the retired cells due at its end in time proportional to their number.

#### Scenario: Retiring many states
- **WHEN** `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-compose --test runtime_timing -- --ignored --nocapture` retires 1,000, 10,000 and 100,000 cells between two passes of a composition
- **THEN** the time to retire them and to free them at the end of the next pass grows roughly linearly with their number

## MODIFIED Requirements

### Requirement: Release Order at the End of a Pass
At the end of a pass, the runtime MUST release what the pass removed, forgot or replaced, in the order it was removed, forgotten or replaced. A removed group's subtree MUST be released children before parents, the last child first. Within a group, its node MUST be released first, with the handlers it owns, then its restart function, then its slots last to first.

#### Scenario: Release order of a removed subtree
- **WHEN** a Rust test removes a restart group holding two slots and a restart function, whose child is a node group holding one slot and a node, all with recording drop entries
- **THEN** they are released in the order: the child's node, the child's slot, the group's restart function, its second slot, then its first slot

### Requirement: Stable Parameters
A parameter, the receiver included, MUST be stable when its type is `Copy` and holds no borrow, by declared markers alone (a type parameter only when bounded by `Copy`), when it is a shared borrow of a state object (`&State<T>`, `&MutableState<T>`), or when it is an `escaping` or `attached` non-composable function type, or a nullable of one. A function value MUST be kept only when it is keepable: its closure captures nothing or owns only `Copy` captures and weak handles.

#### Scenario: Which composables are skippable
- **WHEN** `prism dump-mir --composed` runs on a file declaring `struct Point : Copy { val x: Int }` and the composables `A(n: Int, p: Point)`, `B(s: String)`, `C(r: &Int)`, `D(content: compose () -> Unit)`, `<T> E(x: T)`, `<T : Copy> F(x: T)`, `G(onClick: escaping () -> Unit)`, `H(): Int`, `I(s: &State<Int>, m: &MutableState<String>)`, `J(onClick: attached () -> Unit)` and `K(s: MutableState<Int>)`
- **THEN** the headers of `A`, `F`, `G`, `I` and `J` are marked `skippable`, and those of the others are not

### Requirement: Skipping Composables With Stable Parameters
A composable returning `Unit` whose parameters are all stable MUST be skippable. It MUST keep its arguments in its group's slots, `Copy` values by copy, function values as a keepable copy and borrowed states as weak handles, and MUST skip its body, dropping the arguments it owns, when every argument is the same as the kept one (by the type's `==`, by closure and captures for function values, by identity for states, or as its caller says) and its scope is valid. Nothing MUST be cloned to compare.

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
- **WHEN** `compose fun Plus(onClick: attached () -> Unit) { println("plus") }` and `compose fun Screen(count: &MutableState<Int>, tick: &State<Int>) { println("screen ${tick.value}"); Plus(onClick = { count.value += 1 }) }` are declared and `fun main() { val count = mutableStateOf(0); val tick = mutableStateOf(0); val c = composition { Screen(count, tick) }; tick.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `screen 0`, `plus`, `screen 1` and `true`: the handler rebuilt with a weak handle of the same `count` equals the kept one, so `Plus` skips

#### Scenario: A handler owning a String is never kept
- **WHEN** `compose fun Plus(onClick: escaping () -> Unit) { println("plus") }`, `fun screen(): compose () -> Unit = { val s = "a" + "b"; Plus(onClick = { println(s) }) }` and `fun main() { val c = composition(screen()); c.setContent(screen()) }` run with `--check-allocations`
- **THEN** `plus` is printed twice, and the exit status is 0

#### Scenario: String parameters are always changed
- **WHEN** `compose fun Title(s: String) { println("title") }`, `fun screen(): compose () -> Unit = { Title("a") }` and `fun main() { val c = composition(screen()); c.setContent(screen()) }` run
- **THEN** `title` is printed twice

#### Scenario: A handler copying a field of a borrowed parameter is stable
- **WHEN** `struct Item { val id: Int, val title: String }`, `compose fun Plus(onClick: escaping () -> Unit) { println("plus") }` and `compose fun ItemRow(item: &Item, tick: &State<Int>) { println("row ${tick.value}"); Plus(onClick = { println(item.id) }) }` are declared and `fun main() { val item = Item(1, "a" + "b"); val tick = mutableStateOf(0); val c = composition { ItemRow(item, tick) }; tick.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `row 0`, `plus`, `row 1` and `true`, and the exit status is 0: the handler owns only a copy of `item.id`, so the rebuilt handler equals the kept one and `Plus` skips

#### Scenario: A borrowed state is compared by identity
- **WHEN** `compose fun Label(s: &State<Int>) { println("label ${s.value}") }` and `compose fun Screen(a: &State<Int>, b: &State<Int>, pick: &State<Int>) { println("screen ${pick.value}"); Label(if (pick.value > 1) b else a) }` are declared and `fun main() { val a = mutableStateOf(1); val b = mutableStateOf(2); val pick = mutableStateOf(0); val c = composition { Screen(a, b, pick) }; pick.value = 1; println(c.recompose()); pick.value = 2; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `screen 0`, `label 1`, `screen 1`, `true`, `screen 2`, `label 2` and `true`, one per line: `Label` skips while it is given the same state, and runs when it is given another

### Requirement: The Compose Prelude
The bundled module `/prism/compose/runtime` MUST be a prelude module. It MUST declare `Composition`, `composition`, `setContent`, `recompose` and `dump`, `remember`, `key`, `RecomposeScope`, `currentRecomposeScope` and `invalidate`, and the state declarations `State`, `MutableState`, `mutableStateOf`, `value`, `update`, `WeakReference`, `weak`, `get` and `SideEffect`, and a file's own declaration or import of one of those names MUST shadow it.

#### Scenario: Compose primitives need no import
- **WHEN** a file without imports declares `compose fun Counter() { val id = remember { 7 }; val s = currentRecomposeScope() }` and `fun main() { val c = composition { Counter() } }`
- **THEN** it typechecks, `remember` and `currentRecomposeScope` resolve to `/prism/compose/runtime`, and `prism run` exits with status 0

#### Scenario: A file's own `key`
- **WHEN** a file declares `fun key(n: Int): Int = n` and `fun f(): Int = key(3)`
- **THEN** it typechecks, and the call resolves to the file's own `key`

#### Scenario: A file's own mutableStateOf
- **WHEN** a file declares `fun mutableStateOf(n: Int): Int = n + 1` and `fun main() { println(mutableStateOf(1)) }`
- **THEN** it runs and prints `2`: the call resolves to the file's own `mutableStateOf`

### Requirement: Creating a State
`mutableStateOf(value)` MUST create a state holding `value`, moved in, and return the `MutableState<T>` that owns it.

#### Scenario: A state of a list
- **WHEN** `struct Todo { val title: String }` and `fun main() { val todos = mutableStateOf(listOf(Todo("a"))); println(todos.value.size) }` run with `--check-allocations`
- **THEN** stdout is `1`, and the exit status is 0

### Requirement: Writing a Value
`s.value = v` on a `MutableState<T>`, reached by value or through any reference, MUST move `v` into the state and drop the old value, for every `T`, outside composable contexts. For a `Copy` `T`, a compound assignment `s.value op= e` MUST read the value, compute `value op e` and write the result to the same state, evaluating `s` once.

#### Scenario: Compound assignment through a handle
- **WHEN** `fun main() { val count = mutableStateOf(1); count.value += 2; count.value *= 5; println(count.value) }` runs
- **THEN** stdout is `15`

#### Scenario: Writing a string drops the old one
- **WHEN** `fun main() { val name = mutableStateOf("a" + "b"); name.value = "c" + "d"; println(name.value) }` runs with `--check-allocations`
- **THEN** stdout is `cd`, and the exit status is 0

#### Scenario: Writing through a shared borrow
- **WHEN** `fun reset(s: &MutableState<Int>) { s.value = 0 }` is declared and `fun main() { val count = mutableStateOf(5); reset(count); println(count.value) }` runs
- **THEN** stdout is `0`

#### Scenario: Compound assignment of a String state rejected
- **WHEN** `fun main() { val s = mutableStateOf("a"); s.value += "x" }` is checked
- **THEN** the type checker emits `E4017`, saying that `+=` cannot be used on the value of the state `s` because `String` is not `Copy`, with help suggesting `s.value = s.value + "x"` and `s.update { it + "x" }`

### Requirement: Conflicting Accesses to a Borrowed Cell
While a flag holds a state shared, a write of it, `update` of it or an exclusive flag on it MUST end the program with the runtime error `state accessed while borrowed`. While `update` or an exclusive flag holds a state, any access to it MUST end the program with that error.

#### Scenario: Reading a state while updating it
- **WHEN** `fun main() { val s = mutableStateOf(1); s.update { n -> n + s.value } }` runs
- **THEN** stderr is `panic: state accessed while borrowed` and the location of `s.value` in the closure, and the exit status is 101

### Requirement: Reads During a Pass Register Their Scope
A read of a state during a pass (`s.value`, or a borrow of it) MUST register the innermost restart scope of the pass as a reader of the state, whether the read is in a composable, a content lambda or a plain function called from composition. A read outside a pass MUST register nothing.

#### Scenario: A counter write recomposes only its reader
- **WHEN** `compose fun Label(count: &State<Int>) { println("label ${count.value}") }`, `compose fun Title(n: Int) { println("title $n") }` and `compose fun Screen(count: &State<Int>) { println("screen"); Title(1); Label(count) }` are declared and `fun main() { val count = mutableStateOf(0); val c = composition { Screen(count) }; val onClick = { count.value += 1 }; onClick(); println(c.recompose()); println(c.recompose()) }` runs
- **THEN** stdout is `screen`, `title 1`, `label 0`, `label 1`, `true` and `false`, one per line

#### Scenario: A read in a plain function registers the calling scope
- **WHEN** `fun describe(s: &State<Int>): String = "n = ${s.value}"`, `compose fun Show(s: &State<Int>) { println(describe(s)) }` and `compose fun Root(s: &State<Int>) { println("root"); Show(s) }` are declared and `fun main() { val count = mutableStateOf(0); val c = composition { Root(count) }; count.value = 1; println(c.recompose()) }` runs
- **THEN** stdout is `root`, `n = 0`, `n = 1` and `true`, one per line

#### Scenario: A read inside a content lambda invalidates the lambda's author
- **WHEN** `compose fun Column(content: compose () -> Unit) { content() }` and `compose fun Counter(c: &State<Int>) { println("counter"); Column { println("count ${c.value}") } }` are declared and `fun main() { val n = mutableStateOf(0); val c = composition { Counter(n) }; n.value = 1; println(c.recompose()) }` runs
- **THEN** stdout is `counter`, `count 0`, `counter`, `count 1` and `true`, one per line

#### Scenario: Reads outside composition register nothing
- **WHEN** `compose fun Quiet() { println("quiet") }` and `fun main() { val count = mutableStateOf(0); val c = composition { Quiet() }; println(count.value); count.value = 5; println(c.recompose()) }` run
- **THEN** stdout is `quiet`, `0` and `false`, one per line

### Requirement: Writes Invalidate the Readers
A write of a state (`value =`, a compound assignment, `update`, or a mutable borrow of its value) MUST invalidate every scope registered as a reader of the state, so that the next pass recomposes them, and MUST then forget those registrations. A scope that has left its composition MUST be skipped.

#### Scenario: Two readers of one state
- **WHEN** `compose fun A(s: &State<Int>) { println("a ${s.value}") }` and `compose fun B(s: &State<Int>) { println("b ${s.value}") }` are declared and `fun main() { val s = mutableStateOf(0); val c = composition { A(s); B(s) }; s.value = 1; println(c.recompose()); s.value = 2; println(c.recompose()) }` runs
- **THEN** stdout is `a 0`, `b 0`, `a 1`, `b 1`, `true`, `a 2`, `b 2` and `true`, one per line

#### Scenario: Adding to a list state recomposes its reader
- **WHEN** `compose fun Count(s: &State<List<Int>>) { println("count ${s.value.size}") }` is declared and `fun main() { val s = mutableStateOf(listOf(1)); val c = composition { Count(s) }; s.value.add(2); println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `count 1`, `count 2` and `true`, one per line, and the exit status is 0

### Requirement: Equal Value Writes Invalidate Nothing
A `value =` write, or a compound assignment, whose new value is equal to the state's value by `==` (for function values, by closure and captures, as for skipping) MUST keep the old value, drop the new one and invalidate nothing. A write of a type without `==`, `update` and a mutable borrow of the value MUST always invalidate.

#### Scenario: An equal write recomposes nothing
- **WHEN** `compose fun Show(n: &State<Int>, s: &State<String>) { println("show ${n.value} ${s.value}") }` is declared and `fun main() { val n = mutableStateOf(1); val s = mutableStateOf("a" + "b"); val c = composition { Show(n, s) }; n.value = 1; s.value = "a" + "b"; n.value += 0; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `show 1 ab` and `false`, and the exit status is 0

### Requirement: State Writes Are Rejected in Composable Contexts
A write of a state in a composable context, by `value =`, a compound assignment, `update`, or a mutable borrow, `mut fun` call or part write of `s.value`, MUST be rejected with `E4031` (`StateWriteInComposable`), with a note that state is read-only during composition and help suggesting an event handler.

#### Scenario: Writing a state in a composable rejected
- **WHEN** `compose fun C(s: &MutableState<Int>, l: &MutableState<List<Int>>) { s.value = 1; s.value += 1; s.update { n -> n }; l.value.add(1) }` is checked
- **THEN** the type checker emits `E4031` four times, once at each write

#### Scenario: A handler inside a composable may write
- **WHEN** `fun later(f: () -> Unit) { }` and `compose fun C(s: &MutableState<Int>) { later { s.value += 1 } }` are checked
- **THEN** the file typechecks

### Requirement: State Writes During a Pass End the Program
A write of a state, `update`, or an exclusive flag on it, run while a pass runs on the thread, MUST end the program with the runtime error `state written during composition`, at the write.

#### Scenario: A plain function writes during composition
- **WHEN** `fun reset(s: &MutableState<Int>) { s.value = 0 }` and `compose fun Bad(s: &MutableState<Int>) { reset(s) }` are declared and `fun main() { val s = mutableStateOf(1); val c = composition { Bad(s) } }` runs
- **THEN** stderr is `panic: state written during composition` and the location of `s.value = 0`, and the exit status is 101

### Requirement: A Cell Released While Borrowed Is Freed When the Borrow Ends
A retired state whose cell a flag or `update` holds when its free is due MUST be freed, its value dropped, when the last of them ends; for `update`, the result of `transform` MUST then be dropped instead of stored.

#### Scenario: Releasing a cell during an edit
- **WHEN** a Rust test retires a cell while a shared flag holds it, runs the pass at whose end the cell is due, and then ends the flag
- **THEN** the cell's value is dropped once, when the flag ends, and a later use of a weak handle of it is the error `stale state handle`

### Requirement: Stale Handles End the Program
Any use of a weak handle whose cell was freed, by a closure that captured it or through a borrow a weak reference gave, MUST end the program with the runtime error `stale state handle`, at the use. A cell index whose generation reaches its maximum MUST be retired, never reused.

#### Scenario: Stale handle detected
- **WHEN** `fun consume(s: MutableState<Int>) {}` is declared and `fun main() { val s = mutableStateOf(5); val ref = s.weak(); val c = composition { }; val r = ref.get(); consume(s); c.setContent { }; println(r?.value ?: -1) }` runs
- **THEN** stdout is empty, stderr is `panic: stale state handle` and the location of `r?.value`, and the exit status is 101: the pass of `setContent` freed the retired cell, which the borrow `r` taken before still named

### Requirement: Side Effects Run After Their Pass
`SideEffect(effect)`, whose `effect` is an `attached` parameter, MUST be a composable that queues `effect` each time a pass runs the call. After the pass ends and has released what it removed, the queued effects MUST run in the order they were queued, outside composition and as handlers of the composition, and each effect MUST then be dropped. An effect's writes MUST be recomposed by the next pass.

#### Scenario: Side effects run after composition
- **WHEN** `compose fun Log(n: Int) { println("compose $n"); SideEffect { println("effect $n") } }` and `fun main() { val c = composition { Log(1); Log(2) }; println("after") }` run with `--check-allocations`
- **THEN** stdout is `compose 1`, `compose 2`, `effect 1`, `effect 2` and `after`, one per line, and the exit status is 0

#### Scenario: A side effect may write state
- **WHEN** `compose fun Mark(seen: &MutableState<Int>, n: &State<Int>) { println("mark ${n.value}"); SideEffect { seen.value = 1 } }` is declared and `fun main() { val seen = mutableStateOf(0); val n = mutableStateOf(0); val c = composition { Mark(seen, n) }; println(seen.value); println(c.recompose()) }` runs
- **THEN** stdout is `mark 0`, `1` and `false`: `Mark` does not read `seen`, so the write invalidates nothing

### Requirement: State Rules Cost Linear Compile Time
Checking and lowering state accesses MUST cost constant work per access, and deciding a weak capture constant work per captured path: a member lookup, a marker or capture-function lookup and a test of the composable context in the type checker, and a constant number of calls in MIR, with no analysis across bodies or files.

#### Scenario: State accesses scale linearly
- **WHEN** `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` runs the `state-accesses`, `guarded-state-accesses` and `weak-captures` shapes at their three sizes
- **THEN** the time of each grows roughly linearly with its size

### Requirement: State Whose Type Changed Is Reset
After a reload, a slot whose type fingerprint differs from the type the new code reads MUST be discarded at the end of the pass, dropping its value, which retires the states it owns, and the new code MUST compute its value again. Every other slot MUST keep its value.

#### Scenario: Changing a remembered type resets that state only
- **WHEN** `compose fun Screen() { Column { Counter(); Likes() } }` runs headless under `--watch` and `--check-allocations`, `Likes` remembering its own `mutableStateOf(0)` shown as `Likes: <n>` with a `like` button, the script clicks `+` twice and `like` once, edits `Counter`'s state to `remember { mutableStateOf("") }` shown as `Name: ${c.value}`, and dumps
- **THEN** the dump shows `Name: ` and `Likes: 1`, and the session exits with status 0: the old state was freed

### Requirement: A Scope Whose Composable's Signature Changed Recomposes From Its Parent
When a reload changes the parameter types of a skippable composable, the restart scopes of its old code MUST not be restarted by their old restart functions. Each MUST be recomposed by the nearest enclosing scope whose restart function is current, and its own kept arguments and remembered values MUST be reset, while its child groups keep their state. A retired scope whose group is started during the pass in which the reload was applied MUST be reset when it starts.

#### Scenario: A composable whose parameters changed restarts with the new code
- **WHEN** a headless app run under `--watch` shows `Label(c)`, where `compose fun Label(count: &State<Int>) { Text("n = ${count.value}") }` reads the state itself, the script clicks `+` three times, edits `Label` to `compose fun Label(count: &State<Int>, times: Int)` showing `count.value * times` and the call to `Label(c, 10)`, dumps, clicks `+` and dumps again
- **THEN** the first dump shows `n = 30` and the second `n = 40`: the scope restarted by the click runs the new `Label` with its new arguments

#### Scenario: A scope whose signature changed during a pass is reset
- **WHEN** a plain composition's root reads a state and calls `Wrapper("w")`, a non-skippable composable that calls `Label(5)`, where `compose fun Label(n: Int)` remembers `1` and `2` and prints them, and the program, run under `--watch` with `after line 4: edit main.prism main.prism.v2`, writes the state and recomposes twice; the edit makes `Label(n: Int, times: Int)` remember `10` and `20` and print `label 2: $n $times $a $b`, and `Wrapper` call `Label(5, 2)`; line 4 is printed by the root during the first recompose
- **THEN** that recompose prints `label 2: 5 2 10 20`: the new `Label` started the old scope's group during the pass in which the reload was applied, reset its slots, and read no slot the old code wrote

## REMOVED Requirements

### Requirement: State Handles Are Copy Words
**Reason**: `MutableState<T>` and `State<T>` are owned, move-only state objects (A2′, user direction 2026-10-20); generational handles are an implementation detail of weak captures.
**Migration**: See "State Objects Are Owned Values". Pass states as `&State<T>` or `&MutableState<T>`; closures given to `attached` parameters capture them through weak handles, and other escaping code uses `s.weak()`.

### Requirement: Borrowing a Non-Copy Value in Composable Contexts
**Reason**: A state's value is a place that borrows normally in every context.
**Migration**: See "A State's Value Is a Place", which keeps its scenarios with `&State<T>` parameters, and "Borrows of a State's Value Outside Composition Hold Its Flag".

### Requirement: State Places Outside Composable Contexts
**Reason**: `E4033` is retired: outside composable contexts a state's value is borrowed and written in place, guarded at run time.
**Migration**: Write `todos.value.size`, `todos.value.add(x)` and `p.value.x = 1` directly.

### Requirement: Reading a State With a Closure
**Reason**: `read` is removed; a borrow of the value does the same in any context.
**Migration**: Replace `s.read { v -> f(v) }` with `f(s.value)`.

### Requirement: Editing a State in Place
**Reason**: `edit` is removed; a mutable borrow of the value does the same outside composable contexts.
**Migration**: Replace `s.edit { v -> v.add(x) }` with `s.value.add(x)`.

### Requirement: A Remembered Cell Belongs to Its Slot
**Reason**: A state belongs to its state object; a remembered state object belongs to its slot like any remembered value.
**Migration**: See "Remembering a Value in a Composable", which keeps the scenarios "Remembered state survives recomposition", "Forgotten state frees its value" and "A removed group frees its state", and "Dropping a State Retires It".

### Requirement: A Handler's Cell Belongs to Its Composition
**Reason**: A state created by a handler belongs to whatever holds its state object and is retired when that drops it.
**Migration**: See "State Objects Are Owned Values" and "Retired States Are Freed After the Next Pass".

### Requirement: Other Cells Belong to the Program
**Reason**: Cells have no program owner any more; a top-level state belongs to its top-level value, released with the other top-level values.
**Migration**: See "State Objects Are Owned Values" and execution, "Retired Cells Are Freed Before the Allocation Check".

### Requirement: Cells in Composition Dumps
**Reason**: The runtime no longer knows which slot holds a state; a remembered state shows as its slot.
**Migration**: Read the slot line (`slot MutableState<Int>`) instead of the `cell Int` line.

### Requirement: Remembering a Value in a Composable
**Reason**: `remember` now keeps values that are not `Copy` (A2′, user direction 2026-10-20), so the scenario "Remembering a non-Copy value rejected" no longer holds.
**Migration**: See "Remembering Owned Values in a Composable", which keeps "Remembered value survives recomposition". A non-`Copy` remembered value is a shared borrow of the slot's value.
