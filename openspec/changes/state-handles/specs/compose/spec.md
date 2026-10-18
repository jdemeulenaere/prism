# Compose Specification Delta

## Purpose
Compose is Prism's native support for declarative UI. This change adds reactive state to the capability that `compose-runtime` creates and `compose-transform` extends: state handles and cells, reading and writing them, read tracking and invalidation, the rules that keep state read-only during composition, cell ownership, and `SideEffect`.

## ADDED Requirements

### Requirement: State Handles Are Copy Words
`State<T>` and `MutableState<T>` MUST be `Copy` for every `T` and invariant in `T`. A handle MUST be one 64-bit word naming a cell of the runtime's cell arena by a 32-bit index and a 32-bit generation, and copying a handle MUST NOT copy the cell's value.

#### Scenario: A handler capturing a handle copies the word
- **WHEN** `fun main() { val count = mutableStateOf(1); val onClick = { count.value += 1 }; onClick(); onClick(); println(count.value) }` runs with `--check-allocations`
- **THEN** stdout is `3` and a newline, and the exit status is 0

#### Scenario: A handle of a non-Copy value is Copy
- **WHEN** `fun twice(s: State<String>): Int = 2` and `fun main() { val s = mutableStateOf("a" + "b"); val t = s; println(twice(s) + twice(t)) }` run
- **THEN** stdout is `4`: neither use of `s` moves it

### Requirement: Creating a State
`mutableStateOf(value)` MUST create a cell holding `value`, moved in, and return a `MutableState<T>` handle of it.

#### Scenario: A state of a list
- **WHEN** `struct Todo { val title: String }` and `fun main() { val todos = mutableStateOf(listOf(Todo("a"))); println(todos.read { list -> list.size }) }` run with `--check-allocations`
- **THEN** stdout is `1`, and the exit status is 0

### Requirement: Reading a Copy Value
For a `Copy` `T`, `s.value` MUST read a copy of the cell's value, in any context.

#### Scenario: Reading a counter outside composition
- **WHEN** `fun main() { val count = mutableStateOf(41); val n = count.value + 1; println(n) }` runs
- **THEN** stdout is `42`

### Requirement: Writing a Value
`s.value = v` on a `MutableState<T>` MUST move `v` into the cell and drop the old value, for every `T`, outside composable contexts. For a `Copy` `T`, a compound assignment `s.value op= e` MUST read the value, compute `value op e` and write the result through the same handle, evaluating the handle once.

#### Scenario: Compound assignment through a handle
- **WHEN** `fun main() { val count = mutableStateOf(1); count.value += 2; count.value *= 5; println(count.value) }` runs
- **THEN** stdout is `15`

#### Scenario: Writing a string drops the old one
- **WHEN** `fun main() { val name = mutableStateOf("a" + "b"); name.value = "c" + "d"; name.read { s -> println(s) } }` runs with `--check-allocations`
- **THEN** stdout is `cd`, and the exit status is 0

### Requirement: Borrowing a Non-Copy Value in Composable Contexts
For a non-`Copy` `T`, `s.value` MUST be a place that a composable context can borrow shared, explicitly or implicitly, and read `Copy` parts of. A borrow of it MUST be valid until the end of the body that took it. Moving it out MUST be rejected with `E4017`.

#### Scenario: Iterating a list state in a composable
- **WHEN** `struct Todo { val title: String }` and `compose fun Todos(todos: State<List<Todo>>) { println("count ${todos.value.size}"); for (todo in todos.value) { println(todo.title) } }` are declared and `fun main() { val todos = mutableStateOf(listOf(Todo("a"), Todo("b"))); val c = composition { Todos(todos) } }` runs with `--check-allocations`
- **THEN** stdout is `count 2`, `a` and `b`, one per line, and the exit status is 0

#### Scenario: Moving a state's value out rejected
- **WHEN** `compose fun C(s: State<String>) { val t = s.value }` is checked
- **THEN** the type checker emits `E4017`, saying that the value of a state cannot be moved out, with help suggesting a borrow or `.clone()`

### Requirement: State Places Outside Composable Contexts
Outside composable contexts, a non-`Copy` `s.value` MUST only be assigned, and no `s.value` MUST be borrowed mutably, used as a `mut fun` receiver or written in part. Any such use MUST be rejected with `E4033` (`StateValueOutsideComposable`), whose help names `read` for a shared use, `edit` for a mutable one and `update` for a compound assignment.

#### Scenario: Reading a non-Copy state in a handler rejected
- **WHEN** `compose fun T(todos: State<List<Int>>) { val onClick = { println(todos.value.size) } }` is checked
- **THEN** the type checker emits `E4033` at `todos.value`, with help suggesting `todos.read { value -> ... }`

#### Scenario: Changing a list state in place outside composition rejected
- **WHEN** `fun add(todos: MutableState<List<Int>>) { todos.value.add(1) }` is checked
- **THEN** the type checker emits `E4033`, with help suggesting `todos.edit { value -> ... }`

#### Scenario: Writing part of a Copy state rejected
- **WHEN** `struct Point : Copy { var x: Int }` and `fun f(p: MutableState<Point>) { p.value.x = 1 }` are checked
- **THEN** the type checker emits `E4033`, with help suggesting `p.edit { value -> ... }` or assigning the whole value

### Requirement: Reading a State With a Closure
`s.read(reader)` MUST call `reader` with a shared borrow of the cell's value and return what it returns, in any context. While `reader` runs, the cell MUST be borrowed shared.

#### Scenario: Reading a list's size in a handler
- **WHEN** `fun main() { val todos = mutableStateOf(listOf(1, 2, 3)); val n = todos.read { list -> list.size * 10 }; println(n) }` runs
- **THEN** stdout is `30`

### Requirement: Updating a State With a Closure
`s.update(transform)` on a `MutableState<T>` MUST move the value out of the cell, call `transform` with it, and move its result into the cell, outside composable contexts. While `transform` runs, the cell MUST hold no value.

#### Scenario: Updating a counter
- **WHEN** `fun main() { val count = mutableStateOf(4); count.update { n -> n * n }; println(count.value) }` runs
- **THEN** stdout is `16`

### Requirement: Editing a State in Place
`s.edit(editor)` on a `MutableState<T>` MUST call `editor` with a mutable borrow of the cell's value, outside composable contexts. While `editor` runs, the cell MUST be borrowed mutably.

#### Scenario: A list state edited from a handler
- **WHEN** `struct Todo { val title: String }` and `compose fun Todos(todos: State<List<Todo>>) { println("count ${todos.value.size}") }` are declared and `fun main() { val todos = mutableStateOf(listOf(Todo("a"))); val c = composition { Todos(todos) }; val onClick = { todos.edit { list -> list.add(Todo("b")) } }; onClick(); println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `count 1`, `count 2` and `true`, one per line, and the exit status is 0

### Requirement: Conflicting Accesses to a Borrowed Cell
While `read` borrows a cell, a write to it, `update` or `edit` of it MUST end the program with the runtime error `state accessed while borrowed`. While `update` or `edit` holds a cell, any access to it MUST end the program with that error.

#### Scenario: Reading a state while updating it
- **WHEN** `fun main() { val s = mutableStateOf(1); s.update { n -> n + s.value } }` runs
- **THEN** stderr is `panic: state accessed while borrowed` and the location of `s.value` in the closure, and the exit status is 101

### Requirement: Reads During a Pass Register Their Scope
A read of a state during a pass (`s.value`, a borrow of it, or `read`) MUST register the innermost restart scope of the pass as a reader of the cell, whether the read is in a composable, a content lambda or a plain function called from composition. A read outside a pass MUST register nothing.

#### Scenario: A counter write recomposes only its reader
- **WHEN** `compose fun Label(count: State<Int>) { println("label ${count.value}") }`, `compose fun Title(n: Int) { println("title $n") }` and `compose fun Screen(count: State<Int>) { println("screen"); Title(1); Label(count) }` are declared and `fun main() { val count = mutableStateOf(0); val c = composition { Screen(count) }; val onClick = { count.value += 1 }; onClick(); println(c.recompose()); println(c.recompose()) }` runs
- **THEN** stdout is `screen`, `title 1`, `label 0`, `label 1`, `true` and `false`, one per line

#### Scenario: A read in a plain function registers the calling scope
- **WHEN** `fun describe(s: State<Int>): String = "n = ${s.value}"`, `compose fun Show(s: State<Int>) { println(describe(s)) }` and `compose fun Root(s: State<Int>) { println("root"); Show(s) }` are declared and `fun main() { val count = mutableStateOf(0); val c = composition { Root(count) }; count.value = 1; println(c.recompose()) }` runs
- **THEN** stdout is `root`, `n = 0`, `n = 1` and `true`, one per line

#### Scenario: A read inside a content lambda invalidates the lambda's author
- **WHEN** `compose fun Column(content: compose () -> Unit) { content() }` and `compose fun Counter(c: State<Int>) { println("counter"); Column { println("count ${c.value}") } }` are declared and `fun main() { val n = mutableStateOf(0); val c = composition { Counter(n) }; n.value = 1; println(c.recompose()) }` runs
- **THEN** stdout is `counter`, `count 0`, `counter`, `count 1` and `true`, one per line

#### Scenario: Reads outside composition register nothing
- **WHEN** `compose fun Quiet() { println("quiet") }` and `fun main() { val count = mutableStateOf(0); val c = composition { Quiet() }; println(count.value); count.value = 5; println(c.recompose()) }` run
- **THEN** stdout is `quiet`, `0` and `false`, one per line

### Requirement: Writes Invalidate the Readers
A write of a state (`value =`, a compound assignment, `update`, `edit`) MUST invalidate every scope registered as a reader of its cell, so that the next pass recomposes them, and MUST then forget those registrations. A scope that has left its composition MUST be skipped.

#### Scenario: Two readers of one state
- **WHEN** `compose fun A(s: State<Int>) { println("a ${s.value}") }` and `compose fun B(s: State<Int>) { println("b ${s.value}") }` are declared and `fun main() { val s = mutableStateOf(0); val c = composition { A(s); B(s) }; s.value = 1; println(c.recompose()); s.value = 2; println(c.recompose()) }` runs
- **THEN** stdout is `a 0`, `b 0`, `a 1`, `b 1`, `true`, `a 2`, `b 2` and `true`, one per line

### Requirement: Equal Value Writes Invalidate Nothing
A `value =` write, or a compound assignment, whose new value is equal to the cell's value by `==` (for function values, by closure and captures, as for skipping) MUST keep the old value, drop the new one and invalidate nothing. A write of a type without `==`, `update` and `edit` MUST always invalidate.

#### Scenario: An equal write recomposes nothing
- **WHEN** `compose fun Show(n: State<Int>, s: State<String>) { println("show ${n.value} ${s.value}") }` is declared and `fun main() { val n = mutableStateOf(1); val s = mutableStateOf("a" + "b"); val c = composition { Show(n, s) }; n.value = 1; s.value = "a" + "b"; n.value += 0; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `show 1 ab` and `false`, and the exit status is 0

### Requirement: State Writes Are Rejected in Composable Contexts
A write of a state in a composable context, by `value =`, a compound assignment, `update`, `edit` or a mutable borrow of `s.value`, MUST be rejected with `E4031` (`StateWriteInComposable`), with a note that state is read-only during composition and help suggesting an event handler.

#### Scenario: Writing a state in a composable rejected
- **WHEN** `compose fun C(s: MutableState<Int>, l: MutableState<List<Int>>) { s.value = 1; s.value += 1; s.update { n -> n }; l.edit { v -> }; l.value.add(1) }` is checked
- **THEN** the type checker emits `E4031` five times, once at each write

#### Scenario: A handler inside a composable may write
- **WHEN** `fun later(f: () -> Unit) { }` and `compose fun C(s: MutableState<Int>) { later { s.value += 1 } }` are checked
- **THEN** the file typechecks

### Requirement: State Writes During a Pass End the Program
A write of a state, `update` or `edit`, run while a pass runs on the thread, MUST end the program with the runtime error `state written during composition`, at the write.

#### Scenario: A plain function writes during composition
- **WHEN** `fun reset(s: MutableState<Int>) { s.value = 0 }` and `compose fun Bad(s: MutableState<Int>) { reset(s) }` are declared and `fun main() { val s = mutableStateOf(1); val c = composition { Bad(s) } }` runs
- **THEN** stderr is `panic: state written during composition` and the location of `s.value = 0`, and the exit status is 101

### Requirement: Unremembered State Is Rejected in Composable Contexts
A call of `mutableStateOf` in a composable context MUST be rejected with `E4032` (`UnrememberedState`), with help wrapping the call in `remember { }`. A `remember` calculation is not a composable context.

#### Scenario: Unremembered state rejected
- **WHEN** `compose fun C() { val s = mutableStateOf(0) }` is checked
- **THEN** the type checker emits `E4032` at `mutableStateOf`, with help `remember { mutableStateOf(0) }`

#### Scenario: Remembered state accepted
- **WHEN** `compose fun C() { val s = remember { mutableStateOf(0) } }` is checked
- **THEN** the file typechecks

### Requirement: State Created During a Pass Outside Remember Ends the Program
Creating a state while a pass runs, outside a `remember` calculation and outside a top-level value's initializer, MUST end the program with the runtime error `state created during composition outside remember`, at the call of `mutableStateOf`.

#### Scenario: A plain function creates state during composition
- **WHEN** `fun make(): MutableState<Int> = mutableStateOf(0)` and `compose fun Bad() { val s = make() }` are declared and `fun main() { val c = composition { Bad() } }` runs
- **THEN** stderr is `panic: state created during composition outside remember` and the location of `mutableStateOf(0)`, and the exit status is 101

### Requirement: A Remembered Cell Belongs to Its Slot
A cell created while a `remember` calculation runs MUST belong to the slot in which `remember` keeps the calculation's result, and MUST be freed, its value dropped, when that slot is released: when its key changes, when its group is removed, or when its composition is disposed, at the end of the pass.

#### Scenario: Remembered state survives recomposition
- **WHEN** `compose fun Counter(tick: State<Int>, holder: MutableState<MutableState<Int>>) { val clicks = remember { mutableStateOf(0) }; println("tick ${tick.value} clicks ${clicks.value}"); SideEffect { holder.value = clicks } }` is declared and `fun main() { val tick = mutableStateOf(0); val holder = mutableStateOf(mutableStateOf(0)); val c = composition { Counter(tick, holder) }; holder.value.value += 1; tick.value += 1; println(c.recompose()) }` runs
- **THEN** stdout is `tick 0 clicks 0`, `tick 1 clicks 1` and `true`, one per line

#### Scenario: Forgotten state frees its value
- **WHEN** `compose fun Item(n: Int) { val label = remember(n) { mutableStateOf("item $n") }; println(label.value) }` and `fun screen(n: Int): compose () -> Unit = { Item(n) }` are declared and `fun main() { val c = composition(screen(1)); c.setContent(screen(2)); print(c.dump()) }` runs with `--check-allocations`
- **THEN** stdout starts with `item 1` and `item 2`, the dump shows `Item`'s group with one `cell String` line, and the exit status is 0

#### Scenario: A removed group frees its state
- **WHEN** `compose fun Child() { val s = remember { mutableStateOf("a" + "b") }; println(s.value) }` and `compose fun Parent(show: State<Bool>) { if (show.value) { Child() } }` are declared and `fun main() { val show = mutableStateOf(true); val c = composition { Parent(show) }; show.value = false; println(c.recompose()); print(c.dump()) }` runs with `--check-allocations`
- **THEN** stdout is `ab`, `true` and a dump with no `cell` line, and the exit status is 0

### Requirement: A Handler's Cell Belongs to Its Composition
A cell created outside a pass while a side effect or an event handler of a composition runs MUST belong to that composition, and MUST be freed when the composition is disposed.

#### Scenario: A side effect's state is freed with its composition
- **WHEN** `fun session(holder: MutableState<MutableState<String>>) { val c = composition { SideEffect { holder.value = mutableStateOf("a" + "b") } }; holder.value.read { s -> println(s) } }` and `fun main() { val holder = mutableStateOf(mutableStateOf("")); session(holder); holder.value.read { s -> println(s) } }` run
- **THEN** stdout is `ab`, stderr is `panic: stale state handle` and the location of the second `read`, and the exit status is 101

### Requirement: Other Cells Belong to the Program
A cell created outside a pass and outside any handler or side effect, or while a top-level value's initializer runs, MUST belong to the program. Program cells MUST be freed, their values dropped, before `--check-allocations` reports, and never earlier.

#### Scenario: A program state passes the allocation check
- **WHEN** `fun main() { val s = mutableStateOf(listOf("a" + "b")); s.edit { v -> v.add("c" + "d") } }` runs with `--check-allocations`
- **THEN** the program exits with status 0

### Requirement: A Cell Released While Borrowed Is Freed When the Borrow Ends
A cell whose owner releases it while `read`, `update` or `edit` holds it MUST become stale at once and MUST be freed, its value dropped, when that call ends; for `update`, the result of `transform` MUST be dropped instead of stored.

#### Scenario: Releasing a cell during an edit
- **WHEN** a Rust test edits a remembered cell whose editor recomposes the cell's composition so that its group is removed
- **THEN** the cell's value is dropped once, after the editor returns, and a later read of the handle is the error `stale state handle`

### Requirement: Stale Handles End the Program
Any use of a handle whose cell was freed MUST end the program with the runtime error `stale state handle`, at the use. A cell index whose generation reaches its maximum MUST be retired, never reused.

#### Scenario: Stale handle detected
- **WHEN** `compose fun Child(holder: MutableState<MutableState<Int>>) { val s = remember { mutableStateOf(5) }; SideEffect { holder.value = s } }` and `compose fun Parent(show: State<Bool>, holder: MutableState<MutableState<Int>>) { if (show.value) { Child(holder) } }` are declared and `fun main() { val show = mutableStateOf(true); val holder = mutableStateOf(mutableStateOf(0)); val c = composition { Parent(show, holder) }; val kept = holder.value; println(kept.value); show.value = false; println(c.recompose()); println(kept.value) }` runs
- **THEN** stdout is `5` and `true`, stderr is `panic: stale state handle` and the location of the last `kept.value`, and the exit status is 101

### Requirement: Side Effects Run After Their Pass
`SideEffect(effect)` MUST be a composable that queues `effect` each time a pass runs the call. After the pass ends and has released what it removed, the queued effects MUST run in the order they were queued, outside composition and as handlers of the composition, and each effect MUST then be dropped. An effect's writes MUST be recomposed by the next pass.

#### Scenario: Side effects run after composition
- **WHEN** `compose fun Log(n: Int) { println("compose $n"); SideEffect { println("effect $n") } }` and `fun main() { val c = composition { Log(1); Log(2) }; println("after") }` run with `--check-allocations`
- **THEN** stdout is `compose 1`, `compose 2`, `effect 1`, `effect 2` and `after`, one per line, and the exit status is 0

#### Scenario: A side effect may write state
- **WHEN** `compose fun Mark(seen: MutableState<Int>, n: State<Int>) { println("mark ${n.value}"); SideEffect { seen.value = 1 } }` is declared and `fun main() { val seen = mutableStateOf(0); val n = mutableStateOf(0); val c = composition { Mark(seen, n) }; println(seen.value); println(c.recompose()) }` runs
- **THEN** stdout is `mark 0`, `1` and `false`: `Mark` does not read `seen`, so the write invalidates nothing

### Requirement: Cells in Composition Dumps
A composition's dump MUST list each cell that a slot owns on its own line `cell <type name>`, one level deeper than the slot's line and in creation order. Cells owned by the composition or the program MUST NOT appear in it.

#### Scenario: A dump of a remembered state
- **WHEN** `compose fun Counter() { val count = remember { mutableStateOf(0) } }` and `fun main() { val c = composition { Counter() }; print(c.dump()) }` run
- **THEN** the dump shows, below `Counter`'s group, the `slot` line of the remembered handle and, one level deeper, the line `cell Int`

### Requirement: State Operations Take Constant Time
Creating, reading and writing a state MUST take amortized constant time in the size of the composition and the number of cells, plus time proportional to the readers a write invalidates. A reader MUST be registered at most once per cell however often it reads it, and the registrations of scopes that left their composition MUST NOT grow without bound.

#### Scenario: Many readers of one state
- **WHEN** `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-compose --test runtime_timing -- --ignored --nocapture` composes 1,000, 10,000 and 100,000 restart scopes that each read one state ten times per pass, writes it and recomposes
- **THEN** the median time of a read grows by less than a factor of 2 from the smallest composition to the largest, and each scope is registered once

#### Scenario: Steady-state reads allocate nothing
- **WHEN** `crates/prism-compose/tests/allocations.rs` recomposes 1,000 restart scopes that each read one state, 100 times, without writing it
- **THEN** the counting allocator records no allocation and no free during the last 98 passes

### Requirement: State Rules Cost Linear Compile Time
Checking and lowering state accesses MUST cost constant work per access: a member lookup, a marker lookup and a test of the composable context in the type checker, and a constant number of calls in MIR, with no analysis across bodies or files.

#### Scenario: State accesses scale linearly
- **WHEN** `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` runs the `state-accesses` shape at its three sizes
- **THEN** its time grows roughly linearly with its size
