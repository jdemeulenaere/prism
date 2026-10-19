# Compose Specification Delta

## Purpose
Compose is Prism's native support for declarative UI. This change adds the language side to the capability `compose-runtime` creates: how composables run in a composition, their groups and keys, positional memoization, stable parameters, skipping and restart scopes, and the compose prelude.

## ADDED Requirements

### Requirement: Composables Run in a Composition
`composition(content)` MUST start a composition whose root content is the composable function value `content`, compose it once, and return an owned `Composition`. Every composable that the content calls, directly or through composable lambdas, MUST run during that pass with the composition's composer.

#### Scenario: A composable runs in a composition
- **WHEN** `compose fun Hello(n: Int) { println("hello $n") }` and `fun main() { val c = composition { Hello(1) } }` run with `--check-allocations`
- **THEN** stdout is `hello 1` and a newline, and the exit status is 0

#### Scenario: A trailing composable lambda runs
- **WHEN** `compose fun Column(content: compose () -> Unit) { println("column"); content() }`, `compose fun Text(n: Int) { println("text $n") }` and `fun main() { val c = composition { Column { Text(1); Text(2) } } }` run
- **THEN** stdout is `column`, `text 1` and `text 2`, one per line

### Requirement: Compositions Are Owned Values
`Composition` MUST be neither `Copy` nor `Clone`. Dropping a composition MUST dispose it: every group it holds is removed and released, with its kept and remembered values, and its root content is released. Dropping a composition while a pass runs MUST be the misuse `composition operation during a pass` (compose, "One Pass at a Time").

#### Scenario: Dropping a composition frees everything
- **WHEN** `compose fun Plus(onClick: escaping () -> Unit) { println("plus") }` and `fun main() { val c = composition { val a = 1; val b = 2; Plus(onClick = { println(a + b) }) } }` run with `--check-allocations`
- **THEN** stdout is `plus`, and the program exits with status 0: the handler, the copy of it that `Plus` keeps and the root content are freed when `c` is dropped

#### Scenario: A composition is not copied
- **WHEN** `fun main() { val c = composition { }; val d = c; val e = c }` is checked
- **THEN** the borrow checker reports `E5001` for the second use of `c`

### Requirement: Setting the Content and Recomposing
`c.setContent(content)` MUST replace the composition's root content and recompose its root. `c.recompose()` MUST recompose the composition's invalid scopes and return `true` when any ran, and `false` without running anything otherwise.

#### Scenario: New content recomposes a composition's root
- **WHEN** `fun main() { val c = composition { println(1) }; c.setContent { println(2) }; println(c.recompose()) }` runs
- **THEN** stdout is `1`, `2` and `false`, one per line

### Requirement: Dumping a Composition From Prism
`c.dump()` MUST return the composition's text dump (compose, "Dumping a Composition"). A kept parameter's or remembered value's slot MUST be shown with the name of its type.

#### Scenario: A dump of a composable
- **WHEN** `compose fun Badge(n: Int) { }` and `fun main() { val c = composition { Badge(7) }; print(c.dump()) }` run
- **THEN** stdout is the line `root`, a `replaceable` line one level deeper for the content lambda, a `restart` line below it for `Badge`, the line `slot Int` below that, and the line `nodes`

### Requirement: Composition Operations Are Rejected in Composable Contexts
Calling `composition`, `setContent` or `recompose` in a composable context MUST be rejected with `E4030` (`CompositionInComposable`), at the call, with a note that compositions are started and driven outside composition. In a non-composable closure written inside a composable, they MUST be accepted.

#### Scenario: Starting a composition inside a composable
- **WHEN** `compose fun Screen(c: &Composition) { val d = composition { }; c.recompose() }` is checked
- **THEN** the type checker emits `E4030` at `composition` and at `recompose`

#### Scenario: A handler may start a composition
- **WHEN** `fun later(f: () -> Unit) { f() }` and `compose fun Screen() { later { val d = composition { } } }` are checked
- **THEN** the file typechecks

### Requirement: The Compose Prelude
The bundled module `/prism/compose/runtime` MUST be a prelude module. It MUST declare `Composition`, `composition`, `setContent`, `recompose` and `dump`, `remember`, `key`, `RecomposeScope`, `currentRecomposeScope` and `invalidate`, and a file's own declaration or import of one of those names MUST shadow it.

#### Scenario: Compose primitives need no import
- **WHEN** a file without imports declares `compose fun Counter() { val id = remember { 7 }; val s = currentRecomposeScope() }` and `fun main() { val c = composition { Counter() } }`
- **THEN** it typechecks, `remember` and `currentRecomposeScope` resolve to `/prism/compose/runtime`, and `prism run` exits with status 0

#### Scenario: A file's own `key`
- **WHEN** a file declares `fun key(n: Int): Int = n` and `fun f(): Int = key(3)`
- **THEN** it typechecks, and the call resolves to the file's own `key`

### Requirement: A Composable Body Runs in One Group
Each call of a composable MUST run its body in one group of the current composition: a restart group when the composable is skippable ("Skipping Composables With Stable Parameters"), and a replaceable group otherwise. A call MUST NOT add a group of its own, so calls of one composable among the same siblings match their groups in order.

#### Scenario: Calls of one composable match in order
- **WHEN** `compose fun Leaf() { }`, `compose fun Tree(s: String) { Leaf(); Leaf() }` and `fun main() { val c = composition { Tree("a") }; print(c.dump()) }` run
- **THEN** the dump shows, below the content lambda's group, a `replaceable` group for `Tree` holding two `restart` groups with the same key

### Requirement: Group Keys Come From Declarations and Source Structure
A group key MUST be a 64-bit hash of the enclosing top-level declaration's path and of the path in the source structure from it to the group: a composable body by its declaration path alone, a composable lambda by the call it is an argument of, and a region by its construct and its ordinal among sibling constructs of the same kind. Keys MUST NOT depend on byte offsets, lines or columns.

#### Scenario: Unrelated edits keep keys
- **WHEN** `prism dump-mir --composed` runs on a file declaring `compose fun Text(n: Int) { }`, `compose fun Button(n: Int) { }`, `compose fun Column(content: compose () -> Unit) { content() }` and `compose fun Counter(on: Bool) { if (on) { Column { Text(1) } } }`, and again after a comment is added above `Counter`, the argument of `Text` is changed to `2`, and a call `Button(0)` is inserted before the `Column` call
- **THEN** the keys of `Counter`'s body group, of its `if` branch's group and of its content lambda's group are the same in both dumps

### Requirement: Conditional and Repeated Composable Code Runs in Its Own Groups
A branch of an `if` or `when`, the right operand of `&&` or `||`, the fallback of `?:`, the part of a safe call after its test, and a loop, around all its iterations, MUST run in a replaceable group of its own when it contains a composable call outside closures. Leaving such a region by any path, `return`, `break` and `continue` included, MUST end its group.

#### Scenario: Branches of a composable get their own groups
- **WHEN** `compose fun V(on: Bool) { if (on) { val a = remember { println("on"); 1 } } else { val b = remember { println("off"); 2 } } }`, `fun screen(on: Bool): compose () -> Unit = { V(on) }` and `fun main() { val c = composition(screen(true)); c.setContent(screen(false)); c.setContent(screen(true)) }` run
- **THEN** stdout is `on`, `off` and `on`: each switch removes the other branch's group with its slot

#### Scenario: A loop gets its own group
- **WHEN** `compose fun Rows(n: Int) { for (i in 1..n) { val r = remember { println("row $i"); i } }; val after = remember { println("after"); 0 } }`, `fun screen(n: Int): compose () -> Unit = { Rows(n) }` and `fun main() { val c = composition(screen(2)); c.setContent(screen(3)); c.setContent(screen(1)); print(c.dump()) }` run
- **THEN** stdout starts with `row 1`, `row 2`, `after` and `row 3`, and the dump shows `Rows`'s group with two slots, its kept `n` and `after`, and below them the loop's group with one slot

#### Scenario: An early return ends its groups
- **WHEN** `compose fun Leaf() { }`, `compose fun E(stop: Bool) { if (stop) { Leaf(); return }; Leaf() }`, `fun screen(stop: Bool): compose () -> Unit = { E(stop) }` and `fun main() { val c = composition(screen(true)); c.setContent(screen(false)); println("done") }` run
- **THEN** stdout is `done`, and the exit status is 0

### Requirement: Remembering a Value in a Composable
`remember(calculation)` MUST be a composable that runs `calculation` the first time its position in its group is composed, keeps the result in a slot, and returns a copy of the kept value on every later pass without running `calculation`. Its type parameter MUST be bounded by `Copy`, and `calculation` MUST NOT be a composable context.

#### Scenario: Remembered value survives recomposition
- **WHEN** `compose fun C(label: String) { val id = remember { println("computed"); 7 }; println(label + id.toString()) }`, `fun screen(): compose () -> Unit = { C("x") }` and `fun main() { val c = composition(screen()); c.setContent(screen()); c.setContent(screen()) }` run
- **THEN** stdout is `computed` followed by `x7` three times, one per line

#### Scenario: Remembering a non-Copy value rejected
- **WHEN** `compose fun C() { val s = remember { "a" + "b" } }` is checked
- **THEN** the type checker reports an unsatisfied `Copy` bound (`E4021`)

### Requirement: Remembering With Keys
`remember(key1, calculation)`, `remember(key1, key2, calculation)` and `remember(key1, key2, key3, calculation)` MUST keep each key in a slot before the value, and MUST run `calculation` again, forgetting the old value, when any key differs from the kept one by `==`. Each key MUST be of a `Copy` type without borrows: `E4021` for a key that is not `Copy`, `E4022` for one holding a borrow.

#### Scenario: A changed key recomputes
- **WHEN** `compose fun K(n: Int, label: String) { val v = remember(n) { println("calc $n"); n * 10 }; println(v) }`, `fun screen(n: Int): compose () -> Unit = { K(n, "x") }` and `fun main() { val c = composition(screen(1)); c.setContent(screen(1)); c.setContent(screen(2)); print(c.dump()) }` run
- **THEN** stdout starts with `calc 1`, `10`, `10`, `calc 2` and `20`, and the dump shows `K`'s group holding exactly the slots `slot Int` for the key and `slot Int` for the value

#### Scenario: Keys that cannot be kept rejected
- **WHEN** `compose fun K(s: String, r: (&Int)?) { val a = remember(s) { 1 }; val b = remember(r) { 2 } }` is checked
- **THEN** the type checker reports `E4021` for `s` and `E4022` for `r`

#### Scenario: A borrowed key is copied out
- **WHEN** `compose fun K(r: &Int) { val a = remember(r) { 1 } }` is checked
- **THEN** it typechecks: the key is the `Int` copied out of `r`, as for any argument of a type parameter (references, "Copy-Out From References")

### Requirement: Keyed Groups
`key(value) { content }` MUST compose `content` in a movable group identified by its own key and by the `Int` `value`, so that keyed groups reordered among their siblings keep their groups, slots and nodes.

#### Scenario: Reordered keyed items keep state
- **WHEN** `compose fun Row(id: Int) { val v = remember { println("computed $id"); id * 10 }; println("row $id $v") }`, `compose fun Rows(order: Int) { for (i in 1..3) { val id = if (order == 0) i else (i + 1) % 3 + 1; key(id) { Row(id) } } }`, `fun screen(order: Int): compose () -> Unit = { Rows(order) }` and `fun main() { val c = composition(screen(0)); c.setContent(screen(1)); print(c.dump()) }` run
- **THEN** each `computed` line is printed once, the `row` lines only on the first pass, and the dump lists the movable groups with data keys 3, 1 and 2, in that order

#### Scenario: A key of another type rejected
- **WHEN** `compose fun R(name: String) { key(name) { } }` is checked
- **THEN** the type checker reports an argument type mismatch (`E4004`)

### Requirement: Stable Parameters
A parameter, the receiver included, MUST be stable when its type is `Copy` and holds no borrow, by declared markers alone (a type parameter only when bounded by `Copy`), or when it is an `escaping` non-composable function type, or a nullable of one. A function value MUST be kept only when it is keepable: its closure captures nothing or owns only `Copy` captures.

#### Scenario: Which composables are skippable
- **WHEN** `prism dump-mir --composed` runs on a file declaring `struct Point : Copy { val x: Int }` and the composables `A(n: Int, p: Point)`, `B(s: String)`, `C(r: &Int)`, `D(content: compose () -> Unit)`, `<T> E(x: T)`, `<T : Copy> F(x: T)`, `G(onClick: escaping () -> Unit)` and `H(): Int`
- **THEN** the headers of `A`, `F` and `G` are marked `skippable`, and those of the others are not

### Requirement: Callers Say What They Know of Each Argument
A call of a composable MUST tell the callee, for each argument, that it is static (a constant, or a value read from a top-level `val`), that it forwards a parameter of the caller with what the caller knows of it (static, the same or different, as the caller was told or found by comparing), or that it is uncertain. A callee MUST trust static and same arguments without comparing them.

#### Scenario: A forwarded unchanged parameter is not compared again
- **WHEN** `compose fun Inner(n: Int) { println("inner") }`, `compose fun Outer(n: Int, s: String) { Inner(n) }`, `fun screen(s: String): compose () -> Unit = { Outer(5, s.clone()) }` and `fun main() { val c = composition(screen("a")); c.setContent(screen("b")) }` run
- **THEN** stdout is `inner` once: `Outer` runs again for its changed `String`, and passes `n` to `Inner` as static

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

#### Scenario: A handler owning a String is never kept
- **WHEN** `compose fun Plus(onClick: escaping () -> Unit) { println("plus") }`, `fun screen(): compose () -> Unit = { val s = "a" + "b"; Plus(onClick = { println(s) }) }` and `fun main() { val c = composition(screen()); c.setContent(screen()) }` run with `--check-allocations`
- **THEN** `plus` is printed twice, and the exit status is 0

#### Scenario: String parameters are always changed
- **WHEN** `compose fun Title(s: String) { println("title") }`, `fun screen(): compose () -> Unit = { Title("a") }` and `fun main() { val c = composition(screen()); c.setContent(screen()) }` run
- **THEN** `title` is printed twice

### Requirement: Restart Scopes Recompose Alone
A skippable composable MUST be a restart scope. When its scope is invalid, a pass MUST recompose it alone, from its kept arguments, without running its callers. A composable that is not skippable, and a composable lambda, MUST NOT be a restart scope: invalidating the current scope inside it invalidates the nearest restart scope enclosing it.

#### Scenario: Only invalidated scopes recompose
- **WHEN** `compose fun Child(n: Int) { val s = currentRecomposeScope(); val first = remember { if (n == 1) { s.invalidate() }; 0 }; println("child $n") }`, `compose fun Parent() { println("parent"); Child(1); Child(2) }` and `fun main() { val c = composition { Parent() }; println(c.recompose()); println(c.recompose()) }` run
- **THEN** stdout is `parent`, `child 1`, `child 2`, `child 1`, `true` and `false`, one per line

#### Scenario: A composable that is not skippable recomposes its caller
- **WHEN** `compose fun Label(t: String) { val s = currentRecomposeScope(); val first = remember { s.invalidate(); 0 }; println(t) }`, `compose fun Screen() { println("screen"); Label("a") }` and `fun main() { val c = composition { Screen() }; println(c.recompose()) }` run
- **THEN** stdout is `screen`, `a`, `screen`, `a` and `true`

### Requirement: The Current Recompose Scope
`currentRecomposeScope()` MUST be a composable returning a `RecomposeScope`, a `Copy` handle of the innermost restart scope being composed, or of the composition's root scope when there is none. `s.invalidate()` MUST invalidate that scope, so that the next pass recomposes it, and do nothing when the scope has left its composition.

#### Scenario: The root scope outside restart scopes
- **WHEN** `fun main() { val c = composition { val s = currentRecomposeScope(); val first = remember { s.invalidate(); 0 }; println("root") }; println(c.recompose()); println(c.recompose()) }` runs
- **THEN** stdout is `root`, `root`, `true` and `false`: the content lambda is not a restart scope, so its current scope is the root's

### Requirement: Composable Lambdas
A composable closure MUST run its body in a replaceable group keyed by the call it is an argument of, or, when it is not an argument, by its ordinal among the closure values of its region. A call of a composable function value MUST pass the caller's composer and what it knows of each argument.

#### Scenario: A content lambda's invalidation recomposes its author
- **WHEN** `compose fun Column(content: compose () -> Unit) { content() }`, `compose fun Screen() { println("screen"); Column { val s = currentRecomposeScope(); val first = remember { s.invalidate(); 0 }; println("content") } }` and `fun main() { val c = composition { Screen() }; println(c.recompose()) }` run
- **THEN** stdout is `screen`, `content`, `screen`, `content` and `true`

#### Scenario: A composable lambda with a parameter
- **WHEN** `compose fun Each(n: Int, item: compose (Int) -> Unit) { for (i in 1..n) { item(i) } }` and `fun main() { val c = composition { Each(2) { i -> println("item $i") } } }` run
- **THEN** stdout is `item 1` and `item 2`

#### Scenario: A composable receiver lambda runs
- **WHEN** `struct ColumnScope : Copy { val depth: Int }`, `compose fun ColumnScope.Item(n: Int) { println("item $n at depth $depth") }`, `compose fun Column(content: compose ColumnScope.() -> Unit) { val scope = ColumnScope(1); scope.content() }` and `fun main() { val c = composition { Column { Item(1); Item(2) } } }` run
- **THEN** stdout is `item 1 at depth 1` and `item 2 at depth 1`: the lambda's receiver is its first parameter, before the composer and its change word

### Requirement: Kept Values Are Released With Their Group
The kept arguments of a skippable composable and the values of `remember` MUST be released with the slot that holds them: when their group is removed, when a changed key forgets them, or when the composition is disposed, after the pass ends.

#### Scenario: A removed branch releases its kept handler
- **WHEN** `compose fun Plus(onClick: escaping () -> Unit) { }`, `fun screen(show: Bool): compose () -> Unit = { if (show) { val a = 1; val b = 2; Plus(onClick = { println(a + b) }) } }` and `fun main() { val c = composition(screen(true)); c.setContent(screen(false)); print(c.dump()) }` run with `--check-allocations`
- **THEN** the dump shows the content lambda's group with no group below it, and the program exits with status 0

### Requirement: Reading the Kept Slots of a Restarted Scope
While a restart function runs and before it starts a group, the runtime MUST give generated code the address of slot `i` of the scope's group when that slot holds the requested slot type. Reading a slot that is missing or of another type, or reading outside a restart, MUST be the misuse `no slot to read`.

#### Scenario: A restart reads its kept slots
- **WHEN** a Rust test keeps two word-sized slots in a restart group, invalidates its scope, and its restart function reads slot 1 and then slot 0 before starting a group
- **THEN** it reads the second value and then the first, and reading slot 2 is the misuse `no slot to read`

### Requirement: The Compose Transform Takes Linear Time
The compose transform MUST visit each composable body once, before instances are built, and its time MUST be proportional to the body's size: one key per group and one stability decision per parameter type.

#### Scenario: Composable bodies scale linearly
- **WHEN** `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` runs the compose shapes at their three sizes
- **THEN** the time of each shape grows roughly linearly with its size
