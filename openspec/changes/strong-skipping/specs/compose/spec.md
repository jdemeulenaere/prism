## RENAMED Requirements

- FROM: `### Requirement: Stable Parameters`
- TO: `### Requirement: Parameters That Can Be Kept`

- FROM: `### Requirement: Skipping Composables With Stable Parameters`
- TO: `### Requirement: Every Restartable Composable Can Skip`

## ADDED Requirements

### Requirement: A Borrowed Argument's Identity Is Its Address and Its Source's Version
A borrowed argument of a composable call MUST carry its address and the version of its source: the current version of the state whose contents it borrows; a permanent version for a remembered value or a top-level `val`; the caller's own for a borrowed parameter of the caller, or a part of its referent; and none for anything else. Two identities MUST be the same exactly when their addresses and their versions are equal and the version is not none.

#### Scenario: A remembered value keeps its identity
- **WHEN** `compose fun Show(s: &String) { println("show $s") }` and `compose fun Screen(tick: &State<Int>) { val s = remember { "a" + "b" }; println("screen ${tick.value}"); Show(s) }` are declared and `fun main() { val tick = mutableStateOf(0); val c = composition { Screen(tick) }; tick.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `screen 0`, `show ab`, `screen 1` and `true`, one per line, and the exit status is 0

#### Scenario: A top-level value keeps its identity
- **WHEN** `val Greeting: String = "hello"`, `compose fun Show(s: &String) { println("show $s") }` and `compose fun Screen(tick: &State<Int>) { println("screen ${tick.value}"); Show(Greeting) }` are declared and `fun main() { val tick = mutableStateOf(0); val c = composition { Screen(tick) }; tick.value = 1; println(c.recompose()) }` runs
- **THEN** stdout is `screen 0`, `show hello`, `screen 1` and `true`, one per line

#### Scenario: A part of a borrowed parameter keeps its identity
- **WHEN** `struct Item { val id: Int, val title: String }`, `compose fun Show(s: &String) { println("show $s") }`, `compose fun ItemView(item: &Item, tick: &State<Int>) { println("item ${tick.value}"); Show(item.title) }` and `compose fun Screen(items: &State<List<Item>>, tick: &State<Int>) { ItemView(items.value[0], tick) }` are declared and `fun main() { val items = mutableStateOf(listOf(Item(1, "a" + "b"))); val tick = mutableStateOf(0); val c = composition { Screen(items, tick) }; tick.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `item 0`, `show ab`, `item 1` and `true`, one per line, and the exit status is 0: `ItemView` recomposes alone and passes its item's version down with `item.title`, so `Show` skips

#### Scenario: A composable lambda forwards its parameter's identity
- **WHEN** `compose fun Show(s: &String) { println("show $s") }`, `compose fun Each(items: &State<List<String>>, item: compose (&String) -> Unit) { for (s in items.value) { item(s) } }` and `compose fun Screen(items: &State<List<String>>, tick: &State<Int>) { println("screen ${tick.value}"); Each(items) { s -> Show(s) } }` are declared and `fun main() { val items = mutableStateOf(listOf("a", "b")); val tick = mutableStateOf(0); val c = composition { Screen(items, tick) }; tick.value = 1; println(c.recompose()) }` runs
- **THEN** stdout is `screen 0`, `show a`, `show b`, `screen 1` and `true`, one per line

#### Scenario: A borrow captured by a key lambda keeps its identity
- **WHEN** `compose fun Show(s: &String) { println("show $s") }` and `compose fun Screen(items: &State<List<String>>, tick: &State<Int>) { println("screen ${tick.value}"); for (s in items.value) { key(s.length) { Show(s) } } }` are declared and `fun main() { val items = mutableStateOf(listOf("a", "bb")); val tick = mutableStateOf(0); val c = composition { Screen(items, tick) }; tick.value = 1; println(c.recompose()) }` runs
- **THEN** stdout is `screen 0`, `show a`, `show bb`, `screen 1` and `true`, one per line

### Requirement: Every Write Gives a State a New Version
A state MUST get a version when it is created, and a new one whenever a write stores a value in it, `update` stores its result, an exclusive flag is taken on it, or it is retired. Versions MUST come from one counter per thread, so that no version is given twice on a thread, and MUST never be 0 or 1. A write of an equal value MUST keep the state's version.

#### Scenario: An item changed in place recomposes the rows
- **WHEN** `compose fun Row(s: &String) { println("row $s") }` and `compose fun Rows(items: &State<List<String>>) { println("rows"); for (s in items.value) { Row(s) } }` are declared and `fun main() { val items = mutableStateOf(listOf("a", "b")); val c = composition { Rows(items) }; items.value[0] = "z"; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `rows`, `row a`, `row b`, `rows`, `row z`, `row b` and `true`, one per line, and the exit status is 0

#### Scenario: Versions are never repeated
- **WHEN** a Rust test of `prism-compose` creates a cell, writes it, takes and ends an exclusive flag on it, retires it, lets the end of a pass free it, creates a second cell, and reads the cells' versions after each step
- **THEN** the versions it reads are all different and increasing, and none is 0 or 1

#### Scenario: An equal write keeps the version
- **WHEN** a Rust test writes a cell's value with an equal value and then with a different one
- **THEN** the cell's version is unchanged after the first write and changed after the second

### Requirement: A Call Whose Borrow Has No Identity Runs in a Replaceable Group
A call of a restartable composable one of whose borrowed arguments has no identity MUST run the callee's body in a replaceable group of the same key: never skipped, keeping nothing, and not a restart scope, so that a state it reads registers the nearest restart scope enclosing the call.

#### Scenario: A borrow of a local always recomposes
- **WHEN** `compose fun Show(s: &String) { println("show $s") }` and `compose fun Screen(tick: &State<Int>) { val s = "a" + "b"; println("screen ${tick.value}"); Show(s) }` are declared and `fun main() { val tick = mutableStateOf(0); val c = composition { Screen(tick) }; tick.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `screen 0`, `show ab`, `screen 1`, `show ab` and `true`, one per line, and the exit status is 0

#### Scenario: A composable given a borrow of a local is not a restart scope
- **WHEN** `compose fun Show(s: &String, n: &State<Int>) { println("show $s ${n.value}") }` and `compose fun Screen(n: &State<Int>) { val s = "a" + "b"; println("screen"); Show(s, n) }` are declared and `fun main() { val n = mutableStateOf(0); val c = composition { Screen(n) }; n.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `screen`, `show ab 0`, `screen`, `show ab 1` and `true`, one per line, and the exit status is 0: `Show`'s read registered `Screen`, which recomposes and calls it again

### Requirement: A Scope Restarted Alone Reads Its Kept Borrows
A restart scope recomposed alone MUST be called with the addresses and versions it kept. Whenever it is, each kept address MUST still be valid and its data unchanged: a write, move or removal of the borrowed data MUST make an enclosing scope recompose first, in the same pass, which passes the composable new arguments or removes it.

#### Scenario: A composable restarts alone from its kept borrow
- **WHEN** `compose fun Show(s: &String, n: &State<Int>) { println("show $s ${n.value}") }` and `compose fun Screen(items: &State<List<String>>, n: &State<Int>) { println("screen"); Show(items.value[0], n) }` are declared and `fun main() { val items = mutableStateOf(listOf("a" + "b")); val n = mutableStateOf(0); val c = composition { Screen(items, n) }; n.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `screen`, `show ab 0`, `show ab 1` and `true`, one per line, and the exit status is 0

#### Scenario: A borrow whose list grew is passed again by its parent
- **WHEN** the `Show` above and `compose fun Screen(items: &State<List<String>>, n: &State<Int>) { println("screen ${items.value.size}"); Show(items.value[0], n) }` are declared and `fun main() { val items = mutableStateOf(listOf("a" + "b")); val n = mutableStateOf(0); val c = composition { Screen(items, n) }; items.value.add("c" + "d"); n.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `screen 1`, `show ab 0`, `screen 2`, `show ab 1` and `true`, one per line, and the exit status is 0: the list may have moved, and `Screen` recomposed first and passed `Show` its new address

#### Scenario: A borrow whose item was removed is never restarted
- **WHEN** the `Show` above and `compose fun Screen(items: &State<List<String>>, n: &State<Int>) { println("screen ${items.value.size}"); if (items.value.size > 0) { Show(items.value[0], n) } }` are declared and `fun main() { val items = mutableStateOf(listOf("a" + "b")); val n = mutableStateOf(0); val c = composition { Screen(items, n) }; items.value.removeAt(0); n.value = 1; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `screen 1`, `show ab 0`, `screen 0` and `true`, one per line, and the exit status is 0

## MODIFIED Requirements

### Requirement: Parameters That Can Be Kept
A parameter, the receiver and context parameters included, MUST be keepable when its type is `Copy` and holds no borrow, by declared markers alone (a type parameter only when bounded by `Copy`), when it is a shared borrow `&T`, or a nullable of one, of any type, or when it is an `escaping` or `attached` non-composable function type, or a nullable of one. A function value MUST be kept only when its closure captures nothing or owns only `Copy` captures and weak handles.

#### Scenario: Which composables are skippable
- **WHEN** `prism dump-mir --composed` runs on a file declaring `struct Point : Copy { val x: Int }` and the composables `A(n: Int, p: Point)`, `B(s: String)`, `C(r: &Int)`, `D(content: compose () -> Unit)`, `<T> E(x: T)`, `<T : Copy> F(x: T)`, `G(onClick: escaping () -> Unit)`, `H(): Int`, `I(s: &State<Int>, m: &MutableState<String>)`, `J(onClick: attached () -> Unit)`, `K(s: MutableState<Int>)`, `L(s: (&String)?, p: &Point)` and `M(r: &mut Int)`
- **THEN** the headers of `A`, `C`, `F`, `G`, `I`, `J` and `L` are marked `skippable`, and those of the others are not

### Requirement: Every Restartable Composable Can Skip
A composable returning `Unit` whose parameters are all keepable MUST be skippable. It MUST keep its arguments in its group's slots, `Copy` values by copy, function values as a keepable copy, borrowed states as weak handles and other borrows as their identity, and MUST skip its body, dropping what it owns, when every argument is the same as the kept one (by `==`, by closure and captures, by identity, or as its caller says) and its scope is valid. Nothing MUST be cloned or compared by contents.

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

#### Scenario: A list's rows skip until the list changes
- **WHEN** `compose fun Row(s: &String) { println("row $s") }` and `compose fun Rows(items: &State<List<String>>, tick: &State<Int>) { println("rows ${tick.value}"); for (s in items.value) { Row(s) } }` are declared and `fun main() { val items = mutableStateOf(listOf("a", "b")); val tick = mutableStateOf(0); val c = composition { Rows(items, tick) }; tick.value = 1; println(c.recompose()); items.value.add("c"); println(c.recompose()); tick.value = 2; println(c.recompose()) }` runs with `--check-allocations`
- **THEN** stdout is `rows 0`, `row a`, `row b`, `rows 1`, `true`, `rows 1`, `row a`, `row b`, `row c`, `true`, `rows 2` and `true`, one per line, and the exit status is 0: the rows skip while their list keeps its version, and the append reruns each row once

### Requirement: Restart Scopes Recompose Alone
A skippable composable MUST be a restart scope, except in a call that runs it in a replaceable group. When its scope is invalid, a pass MUST recompose it alone, from its kept arguments, without running its callers. A composable that is not skippable, a call run in a replaceable group, and a composable lambda MUST NOT be a restart scope: invalidating the current scope inside one invalidates the nearest restart scope enclosing it.

#### Scenario: Only invalidated scopes recompose
- **WHEN** `compose fun Child(n: Int) { val s = currentRecomposeScope(); val first = remember { if (n == 1) { s.invalidate() }; 0 }; println("child $n") }`, `compose fun Parent() { println("parent"); Child(1); Child(2) }` and `fun main() { val c = composition { Parent() }; println(c.recompose()); println(c.recompose()) }` run
- **THEN** stdout is `parent`, `child 1`, `child 2`, `child 1`, `true` and `false`, one per line

#### Scenario: A composable that is not skippable recomposes its caller
- **WHEN** `compose fun Label(t: String) { val s = currentRecomposeScope(); val first = remember { s.invalidate(); 0 }; println(t) }`, `compose fun Screen() { println("screen"); Label("a") }` and `fun main() { val c = composition { Screen() }; println(c.recompose()) }` run
- **THEN** stdout is `screen`, `a`, `screen`, `a` and `true`

### Requirement: Callers Say What They Know of Each Argument
A call of a composable MUST tell the callee, for each argument, that it is static (a constant, a value read from a top-level `val`, or a borrow of one), that it forwards a parameter of the caller with what the caller knows of it (static, the same or different, as the caller was told or found by comparing), a borrow of a borrowed parameter's whole referent included, or that it is uncertain. A callee MUST trust static and same arguments without comparing them.

#### Scenario: A forwarded unchanged parameter is not compared again
- **WHEN** `compose fun Inner(n: Int) { println("inner") }`, `compose fun Outer(n: Int, s: String) { Inner(n) }`, `fun screen(s: String): compose () -> Unit = { Outer(5, s.clone()) }` and `fun main() { val c = composition(screen("a")); c.setContent(screen("b")) }` run
- **THEN** stdout is `inner` once: `Outer` runs again for its changed `String`, and passes `n` to `Inner` as static

### Requirement: Recomposition After a Reload Skips Unchanged Children
The pass after a reload MUST run each invalidated scope's body. A skippable composable it calls whose arguments did not change and whose own scope is not invalid MUST skip, as in any pass. In that pass, every argument a skippable composable keeps, borrowed identities included, MUST be compared with the kept one even when its change bits say same or static, since the call site may be new code passing another value: one whose argument changed MUST run its body with the new arguments and keep them.

#### Scenario: A sibling in an unaffected file skips
- **WHEN** a headless app run under `--watch` has a `Screen` in `app/main.prism` that calls `Badge(1)`, a skippable composable of `lib/badge.prism` that prints `badge`, and the script edits `app/main.prism` to change another text of `Screen`
- **THEN** `badge` is printed once, on the first composition, and not after the reload

#### Scenario: An edited argument reaches a skippable composable of another file
- **WHEN** a headless app run under `--watch` has a `Screen` in `app/main.prism` that calls `Label(5)`, a skippable composable of `lib/label.prism` showing `n = $n`, and the script dumps, edits the call to `Label(6)` and dumps again
- **THEN** the second dump holds the text node reading `n = 6`, although the constant's change bits are static

#### Scenario: Arguments forwarded in another order are compared after a reload
- **WHEN** a headless app run under `--watch` has `compose fun Outer(a: Int, b: Int) { Inner(a, b) }` in `app/main.prism`, `Inner` a skippable composable of `lib/inner.prism` showing `a = $a b = $b`, called as `Outer(c.value, 2)` with `c` at `1`, and the script edits the body of `Outer` to `Inner(b, a)`, dumps, clicks a button adding 10 to `c` and dumps
- **THEN** the dumps read `a = 2 b = 1` and then `a = 2 b = 11`

#### Scenario: An edited top-level value reaches a composable that borrows it
- **WHEN** a headless app run under `--watch` has `val Greeting: String = "hello"` and a `Screen` in `app/main.prism` that calls `Label(Greeting)`, where `compose fun Label(text: &String) { Text(text) }` is in `lib/label.prism`, and the script dumps, edits the initializer of `Greeting` to `"hi"` and dumps again
- **THEN** the first dump holds a text node reading `hello`, and the second one reading `hi`: the edited value has new storage, so `Label`'s kept identity differs

### Requirement: The Compose Transform Takes Linear Time
The compose transform MUST visit each composable body once, before instances are built, after one scan of the body's assignments, and its time MUST be proportional to the body's size: one key per group, one decision per parameter type, and one classification per assignment of a reference and per argument.

#### Scenario: Composable bodies scale linearly
- **WHEN** `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` runs the compose shapes, `borrowed-arguments` included, at their three sizes
- **THEN** the time of each shape grows roughly linearly with its size
