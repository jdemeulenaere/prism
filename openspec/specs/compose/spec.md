# Compose Specification

## Purpose
Compose is Prism's native support for declarative UI. This capability covers the composition runtime that composable code runs on: compositions and their passes, the slot table of keyed groups and typed slots, deferred removals, restart scopes and invalidation, node groups and the applier that owns nodes, the validation of handles and the errors for misuse, the composer protocol of `/prism/composer`, text dumps, and the runtime's costs. The language side (the `compose fun` transform, `remember`, state handles) is added to this capability by later changes.

## Requirements

### Requirement: The Composition Runtime Is a UI-Agnostic Crate
The composition runtime MUST be the crate `prism-compose`, which depends on no crate but `prism-runtime` and knows nothing about UI. Generated code MUST reach it only through its `extern "C"` entry points, named `prism_compose_*` and listed with their machine signatures in one table.

#### Scenario: Every entry point is listed once
- **WHEN** `cargo test -p prism-compose` runs
- **THEN** a test checks that every `extern "C"` function the crate's sources define is listed exactly once in its table, with a non-null address, and that the crate's manifest has no dependency but `prism-runtime`

### Requirement: Compose Entry Points Are Checked Like Runtime Entry Points
The JIT MUST resolve the compose entry points by symbol, as it resolves the runtime's functions. The backend MUST find every external that names one in the compose table, and MUST compare its machine signature with the table's as for the runtime's functions (code-generation, "Externals Match the Runtime").

#### Scenario: A compose entry point declared with the wrong signature
- **WHEN** a library test module declares `external fun prism_compose_recompose(composition: Double): Bool` and a program calls it
- **THEN** compiling the program reports `E6005`, saying that the entry point `prism_compose_recompose` takes `i64` where the declaration passes `f64`

#### Scenario: The protocol module's externals match the entry points
- **WHEN** `cargo test -p prism-cli --test library_tests` compiles every bundled module
- **THEN** every external of `/prism/composer` is found among the compose entry points with the same machine signature

### Requirement: The Runtime Runs Program Code Only Through Function Values
The runtime MUST run program code only by calling function values through their descriptors' slots, and by calling the drop and equality entries that slot types and descriptors name. It MUST NOT keep a code address of its own, so a function value it holds always runs the code its slot holds when it is called.

#### Scenario: A restart function reaches its closure's current code
- **WHEN** a Rust test keeps a restart function whose descriptor names a slot, points that slot at other code between two passes, and invalidates the scope
- **THEN** the second pass runs the other code

### Requirement: Starting a Composition Composes It Once
Starting a composition MUST create a composition that owns the given content function, run one pass that composes the content as the body of the composition's root restart group, and return the composition's handle. The content MUST be called with the composer of that pass.

#### Scenario: Starting a composition composes its content once
- **WHEN** `import /prism/composer` and `fun main() { val c = startComposition { k -> println("composed") }; println(recomposeComposition(c)); disposeComposition(c) }` run with `--check-allocations`
- **THEN** stdout is `composed` and `false`, each followed by a newline, and the exit status is 0

### Requirement: Recomposing Runs a Pass Only When a Scope Is Invalid
Recomposing a composition MUST run a pass when at least one of its scopes is invalid, and return `true`. Otherwise it MUST run nothing and return `false`. A pass MUST recompose the invalid scopes ("Invalid Scopes Recompose in Tree Order"), then release what it removed, forgot or replaced ("Removals Are Deferred to the End of the Pass"), and then end.

#### Scenario: A remembered value survives recomposition
- **WHEN** `fun main() { val c = startComposition { k -> val n = rememberInt(k, 0) { println("computed"); 7 }; println(n) }; invalidate(rootScope(c)); println(recomposeComposition(c)); invalidate(rootScope(c)); println(recomposeComposition(c)); disposeComposition(c) }` runs with `import /prism/composer`
- **THEN** stdout is `computed`, `7`, `7`, `true`, `7` and `true`, one per line: the calculation runs once

### Requirement: Setting the Content Recomposes the Root
Setting a composition's content MUST replace its root content function, invalidate its root scope and run a pass. The old content MUST be released at the end of that pass.

#### Scenario: New content recomposes the root
- **WHEN** `import /prism/composer` and `fun main() { val c = startComposition { k -> println(1) }; setCompositionContent(c) { k -> println(2) }; println(recomposeComposition(c)); disposeComposition(c) }` run
- **THEN** stdout is `1`, `2` and `false`, one per line

### Requirement: Groups Have a Kind and a Key
Each group MUST have a key, a 64-bit integer, and a kind: restart, replaceable, movable or node. A movable group MUST also have a data key, a second 64-bit integer. A group MUST hold its parent, its ordered children and its ordered slots, and a node group its node.

#### Scenario: Group kinds in a dump
- **WHEN** a composition's content starts a restart, a replaceable, a movable and a node group, each inside the previous one, and is dumped
- **THEN** the dump shows the four groups nested in that order, each with its kind and key, and the movable group with its data key

### Requirement: Starting a Group Reuses a Matching Old Sibling
Starting a group MUST reuse the first old child of the current group that has not been visited in this pass and has the same kind and key, and for a movable group the same data key, so that equal keys match in order. When none matches, it MUST create a new group. A reused group MUST keep its slots, children and node, and take its new position among its siblings.

#### Scenario: Reordered movable groups keep their slots
- **WHEN** `import /prism/composer`, `fun Item(k: Int, id: Int) { startMovableGroup(k, 1, id); val n = rememberInt(k, 0) { println("computed $id"); id * 10 }; endGroup(k) }` and `fun Items(k: Int, order: Int) { if (order == 0) { Item(k, 1); Item(k, 2); Item(k, 3) } else { Item(k, 3); Item(k, 1); Item(k, 2) } }` are declared, and `main` starts a composition of `{ k -> Items(k, 0) }`, sets its content to `{ k -> Items(k, 1) }`, prints its dump and disposes it
- **THEN** `computed 1`, `computed 2` and `computed 3` are each printed once, before the dump, and the dump lists the movable groups with data keys 3, 1 and 2, in that order, holding the words 30, 10 and 20

#### Scenario: Equal keys match in order
- **WHEN** `fun Cell(k: Int, n: Int) { startGroup(k, 5); val v = rememberInt(k, 0) { println("cell $n"); n }; endGroup(k) }` is declared, and a composition of `{ k -> Cell(k, 1); Cell(k, 2) }` gets the content `{ k -> Cell(k, 1); Cell(k, 2); Cell(k, 3) }`
- **THEN** stdout is `cell 1`, `cell 2` and `cell 3`: the first two groups are reused in order and only the third is created

### Requirement: Unvisited Children Are Removed When Their Group Ends
When a group ends, each of its old children not visited in this pass MUST be removed, with everything below it.

#### Scenario: Branches get their own groups
- **WHEN** `fun View(k: Int, on: Bool) { if (on) { startGroup(k, 1); val a = rememberInt(k, 0) { println("on"); 1 }; endGroup(k) } else { startGroup(k, 2); val b = rememberInt(k, 0) { println("off"); 2 }; endGroup(k) } }` is declared, and a composition of `{ k -> View(k, true) }` gets the content `{ k -> View(k, false) }` and then `{ k -> View(k, true) }`
- **THEN** stdout is `on`, `off` and `on`: each switch removes the other branch's group with its slot, and creates the new branch's group

### Requirement: Slots Are Read and Written in Order
A group's slots MUST be read and written in order by a cursor that starts at its first slot when the group starts, and every read or write that finds or puts a slot MUST move the cursor past it. When a group ends, its slots after the cursor MUST be forgotten.

#### Scenario: Slots past the cursor are forgotten
- **WHEN** `fun Pair(k: Int, both: Bool) { startGroup(k, 1); val a = rememberInt(k, 0) { 1 }; if (both) { val b = rememberInt(k, 0) { 2 } }; endGroup(k) }` is declared, and a composition of `{ k -> Pair(k, true) }` gets the content `{ k -> Pair(k, false) }` and is dumped
- **THEN** the dump shows the group `replaceable 1` holding the one slot `word 0 = 1`

### Requirement: Slots Record Their Value's Type
A slot MUST hold one value together with its slot type: the value's size, alignment, drop entry, equality entry and type fingerprint. A read with a slot type whose fingerprint differs from the slot's MUST find no slot, and a write there MUST forget the old slot and put the new value in its place. A word slot MUST hold a 64-bit value and a type tag, and MUST match only a read with the same tag.

#### Scenario: A slot of another type is recreated
- **WHEN** `fun Value(k: Int, tag: Int) { startGroup(k, 1); val v = rememberInt(k, tag) { println("tag $tag"); tag }; endGroup(k) }` is declared, and a composition of `{ k -> Value(k, 1) }` gets the content `{ k -> Value(k, 2) }`
- **THEN** stdout is `tag 1` and `tag 2`, and a dump taken afterwards shows the group holding only `word 2 = 2`

### Requirement: Keys Are Not Trusted for Memory Safety
The runtime MUST NOT rely on group keys for memory safety. A group matched by mistake, as after an edit under hot reload, MUST only ever give back values of the slot type the reader asks for, and a matched node group MUST only ever give back a node of the kind it is started with.

#### Scenario: A typed slot read with another slot type
- **WHEN** a Rust test stores a value of a 16-byte slot type with a recording drop entry, and the next pass reads the same position with an 8-byte slot type of another fingerprint and writes a value there
- **THEN** the read finds no slot, the old value is dropped once at the end of that pass, and the next pass reads the 8-byte value

### Requirement: Remembering a Value
Remembering a value MUST return the value of the slot at the cursor when that slot has the requested type, without running the calculation. Otherwise it MUST run the calculation, store its result in a new slot at the cursor, and return it.

#### Scenario: A calculation runs only for a new slot
- **WHEN** a composition's content prints `rememberInt(k, 0) { println("calc"); 4 }` and is recomposed through its root scope twice
- **THEN** stdout is `calc` followed by `4` three times

### Requirement: Comparing a Value With a Slot
Comparing a value with a slot MUST return `false` when the slot at the cursor has the value's type and its equality entry finds the two values equal, and then release the new value. Otherwise it MUST store the new value at the cursor, forgetting any old slot there, and return `true`. A slot type without an equality entry MUST compare as always changed. No value MUST be cloned to compare it.

#### Scenario: Comparing a value stores it
- **WHEN** `fun main() { val c = startComposition { k -> println(changedInt(k, 0, 1)) }; setCompositionContent(c) { k -> println(changedInt(k, 0, 1)) }; setCompositionContent(c) { k -> println(changedInt(k, 0, 2)) }; disposeComposition(c) }` runs with `import /prism/composer`
- **THEN** stdout is `true`, `false` and `true`, one per line

### Requirement: Removals Are Deferred to the End of the Pass
Removing a group, forgetting a slot, and replacing a restart function or a composition's content MUST NOT release anything until the pass ends. Every slot address and every value the runtime gives out during a pass MUST stay valid until the pass ends.

#### Scenario: A removed group's slots are released after the pass
- **WHEN** a Rust test's content stops starting a group whose slot has a recording drop entry, and records a mark after the group's parent has ended
- **THEN** the drop is recorded after the mark, once the pass's last body has returned

### Requirement: Release Order at the End of a Pass
At the end of a pass, the runtime MUST release what the pass removed, forgot or replaced, in the order it was removed, forgotten or replaced. A removed group's subtree MUST be released children before parents, the last child first. Within a group, its slots MUST be released last to first, then its restart function, then its node.

#### Scenario: Release order of a removed subtree
- **WHEN** a Rust test removes a group holding two slots, a restart function, and a child group with one slot and a restart function, all with recording drop entries
- **THEN** they are released in the order: the child's slot, the child's restart function, the group's second slot, its first slot, then its restart function

### Requirement: Restart Groups Keep a Restart Function
A restart group MUST be a restart scope, and so MUST a composition's root group. Ending a restart group MUST take a restart function, an owned function value of type `(Int) -> Unit`, which the group keeps until it next ends, when the new one replaces it. The runtime MUST NOT interpret the change words that composables pass each other.

#### Scenario: An invalidated scope recomposes alone
- **WHEN** `fun Ticker(k: Int) { startRestartGroup(k, 20); val first = changedInt(k, 0, 1); println("ticker"); if (first) { invalidate(currentScope(k)) }; endRestartGroup(k) { c -> Ticker(c) } }` is declared, and `main` starts a composition of `{ k -> println("root"); Ticker(k) }` and prints `recomposeComposition(c)` twice
- **THEN** stdout is `root`, `ticker`, `ticker`, `true` and `false`, one per line

### Requirement: Recomposing a Scope Alone
Recomposing a scope MUST call its restart function, or the root content for the root scope, with the current pass's composer, in a position where the first group it starts is matched against the scope's group alone. Groups it starts after that one MUST be new siblings after it, and the scope's group MUST be removed if it starts no group there.

#### Scenario: A restart function that starts another key
- **WHEN** a Rust test's restart function starts a group with another key than its scope's group
- **THEN** the old group is removed at the end of the pass, and the new group takes its place among its siblings

#### Scenario: A scope below a skipped scope recomposes alone
- **WHEN** `fun Child(k: Int, root: Int, name: String) { startRestartGroup(k, 2); val n = rememberInt(k, 0) { println("computed"); 5 }; println("child $name $n"); if (changedInt(k, 0, 1)) { invalidate(currentScope(k)); invalidate(root) }; endRestartGroup(k) { c -> Child(c, root, name.clone()) } }` and `fun Parent(k: Int, root: Int) { startRestartGroup(k, 1); if (skipping(k)) { skipToGroupEnd(k) } else { println("parent"); Child(k, root, "a" + "b") }; endRestartGroup(k) { c -> Parent(c, root) } }` are declared, and `main` starts a composition of `{ k -> println("root"); Parent(k, currentScope(k)) }`, prints `recomposeComposition(c)` and the dump, and disposes it, with `--check-allocations`
- **THEN** stdout is `root`, `parent`, `computed`, `child ab 5`, `root`, `child ab 5` and `true`, then the dump: the second pass skips `Parent`, and recomposing the child alone reuses its group, its slots and its restart function's string, which is freed once

### Requirement: The Current Scope
The current scope of a composer MUST be the innermost restart group open in it, or the composition's root scope when none is open. A scope MUST be named by a 64-bit generational handle.

#### Scenario: The root is the current scope outside restart groups
- **WHEN** a composition's content prints `currentScope(k) == rootScope(c)` for the composition `c`
- **THEN** it prints `true`

### Requirement: Skipping a Restart Group's Body
While the innermost open group is a restart group, `skipping` MUST be `true` exactly when that group existed before the pass and its scope is not invalid, and `false` otherwise. Skipping to the group's end MUST keep the group's remaining slots, children and nodes as they are.

#### Scenario: Equal arguments skip a restart scope
- **WHEN** `fun Label(k: Int, n: Int) { startRestartGroup(k, 10); if (changedInt(k, 0, n) || !skipping(k)) { println("label $n") } else { skipToGroupEnd(k) }; endRestartGroup(k) { c -> Label(c, n) } }` is declared, and `main` starts a composition of `{ k -> println("root"); Label(k, 1) }`, invalidates its root scope and recomposes it
- **THEN** stdout is `root`, `label 1` and `root`: the second pass skips `Label`'s body

### Requirement: Invalidating a Scope
Invalidating a scope MUST mark it invalid in constant time. Invalidating a scope whose group was removed, or a handle that names no scope, MUST do nothing. A scope invalidated during a pass MUST be recomposed by the next pass.

#### Scenario: Invalidating a stale scope does nothing
- **WHEN** `fun main() { val c = startComposition { k -> }; val s = rootScope(c); disposeComposition(c); invalidate(s); invalidate(12345) }` runs with `import /prism/composer`
- **THEN** the program prints nothing and exits with status 0

### Requirement: Invalid Scopes Recompose in Tree Order
A pass MUST recompose its invalid scopes in tree order as of the pass's start, parents before their descendants, each at most once. A scope that its parent's recomposition has already composed MUST NOT be recomposed again, and a scope whose group the pass removed MUST NOT be recomposed.

#### Scenario: Parents recompose before their children
- **WHEN** `fun Child(k: Int, parent: Int) { startRestartGroup(k, 2); val first = changedInt(k, 0, 1); println("child"); if (first) { invalidate(currentScope(k)); invalidate(parent) }; endRestartGroup(k) { c -> Child(c, parent) } }` and `fun Parent(k: Int) { startRestartGroup(k, 1); println("parent"); Child(k, currentScope(k)); endRestartGroup(k) { c -> Parent(c) } }` are declared, and a composition of `{ k -> Parent(k) }` is recomposed once
- **THEN** stdout is `parent`, `child`, `parent` and `child`: the second pass recomposes the parent, which composes the child, and the child is not recomposed again

#### Scenario: A scope removed by its parent is not recomposed
- **WHEN** `fun Child(k: Int) { startRestartGroup(k, 2); val first = changedInt(k, 0, 1); println("child"); if (first) { invalidate(currentScope(k)) }; endRestartGroup(k) { c -> Child(c) } }` and `fun Screen(k: Int, show: Bool) { println("screen $show"); if (show) { Child(k) } }` are declared, and a composition of `{ k -> Screen(k, true) }` gets the content `{ k -> Screen(k, false) }` and is then recomposed
- **THEN** stdout is `screen true`, `child` and `screen false`, and the recomposition returns `false`

#### Scenario: A scope inside a removed group is not recomposed
- **WHEN** the scenario above's `Screen` wraps `Child(k)` in `startGroup(k, 9)` and a node group, and the same `main` runs and then prints the dump
- **THEN** stdout is `screen true`, `child`, `screen false` and `false`, then a dump holding only `root` and `nodes`: removing the outer group removes the scope below it from the pass

### Requirement: Node Groups Own Nodes Through an Applier
A composition MUST have an applier, which creates, places, removes and releases the nodes of its node groups. A node group MUST create its node, of the kind it is started with, through the applier the first time it is composed, and reuse that node while it is matched. A matched node group started with another kind MUST create a new node, and the old one MUST be released at the end of the pass.

#### Scenario: Removed node groups remove their nodes after the pass
- **WHEN** a Rust test with a recording applier stops starting a node group
- **THEN** the applier records the node's removal and release after the pass's last body has returned, and no other node is placed again

### Requirement: Nodes Follow Their Groups
Each node MUST be a child of the node of the nearest enclosing node group, or of the applier's root node when there is none, and nodes MUST be ordered among their siblings as their groups are. Placing and moving nodes MUST happen during composition. The node of a removed group MUST be removed from its parent at the end of the pass, and nodes MUST be released children before parents.

#### Scenario: Moved node groups move their nodes
- **WHEN** `fun Item(k: Int, id: Int) { startMovableGroup(k, 1, id); val node = startNodeGroup(k, 2, id); endGroup(k); endGroup(k) }` is declared, and a composition emitting `Item` for 1, 2 and 3 is set to emit them for 3, 1 and 2 and dumped
- **THEN** the node tree lists the same three node numbers as before, now in the order of kinds 3, 1 and 2, and no node was created on the second pass

### Requirement: The Node Tree Applier
A composition started through the entry points MUST use the built-in node tree applier, which keeps each node's kind and children and nothing else. It MUST number nodes from 1 in the order it creates them, never reusing a number.

#### Scenario: Node numbers are not reused
- **WHEN** a composition creates a node group, then a pass removes it and another pass creates a node group again
- **THEN** a dump after the third pass shows the new node as `#2`

### Requirement: Disposing a Composition Releases Everything
Disposing a composition MUST remove every group it holds and release them as the end of a pass does. It MUST then release its root content function and its applier, and make its handle stale.

#### Scenario: Disposing a composition frees what it holds
- **WHEN** `fun Greeting(k: Int, name: String) { startRestartGroup(k, 3); println("hello " + name); endRestartGroup(k) { c -> Greeting(c, name.clone()) } }` is declared (a closure never gives away what it captures, so the restart function clones the name it owns), and `fun main() { val c = startComposition { k -> Greeting(k, "a" + "b") }; disposeComposition(c) }` runs with `--check-allocations` and `import /prism/composer`
- **THEN** stdout is `hello ab`, and the program exits with status 0: the restart function and the string it owns are freed

### Requirement: Handles Are Validated
Every entry point MUST validate the handles it is given in constant time. A composer MUST be valid only during the pass that passed it. Compositions and scopes MUST be 64-bit generational handles of a 32-bit index and a 32-bit generation, and an index whose generation reaches its maximum MUST be retired, never reused.

#### Scenario: A composer used after its pass
- **WHEN** `fun Label(k: Int) { startRestartGroup(k, 1); if (changedInt(k, 0, 1)) { invalidate(currentScope(k)) }; endRestartGroup(k) { c -> Label(k) } }` is declared, its restart function calling `Label` with the composer it captured instead of `c`, and `fun main() { val c = startComposition { k -> Label(k) }; recomposeComposition(c) }` runs
- **THEN** the program ends with `panic: composition misuse: composer used outside its pass` and status 101

#### Scenario: Disposing a composition twice
- **WHEN** `fun main() { val c = startComposition { k -> }; disposeComposition(c); disposeComposition(c) }` runs
- **THEN** the program ends with `panic: composition misuse: stale composition handle` and status 101

#### Scenario: A composition used after it is disposed
- **WHEN** `fun main() { val c = startComposition { k -> }; disposeComposition(c); recomposeComposition(c) }` runs, or the last call is `setCompositionContent(c) { k -> }`, `invalidate(rootScope(c))` or `print(dumpComposition(c))`
- **THEN** each form ends with `panic: composition misuse: stale composition handle` and status 101

### Requirement: Composition Misuse Is a Runtime Error
Misusing the composition protocol MUST end the program with the runtime error `composition misuse: <detail>`, which writes no location (execution, "Runtime Errors End the Program"). The details are `composer used outside its pass` and `stale composition handle` ("Handles Are Validated"), `composition operation during a pass` ("One Pass at a Time"), those of "Groups Are Balanced" and "Slots Belong to an Open Group", and `no slot to read`, for a read that must find a slot and finds none.

#### Scenario: Reading a slot that is not there
- **WHEN** a Rust test calls the entry point that reads a word slot where the cursor is at no slot
- **THEN** the runtime error `composition misuse: no slot to read` ends the test's child process with status 101

### Requirement: Slots Belong to an Open Group
A slot operation MUST read or write the slots of the innermost group open in the composer, or of the root group in the root content. A restart function that reads or writes a slot before it starts a group has no group of its own open, and the operation MUST be the misuse `slot outside a group`.

#### Scenario: A slot read before a restart function's group
- **WHEN** a Rust test's restart function asks whether a word slot is at the cursor before it starts its group, and its scope is recomposed
- **THEN** the operation fails with the misuse `slot outside a group`

#### Scenario: A remembered value read before a restart function's group
- **WHEN** `fun Label(k: Int) { startRestartGroup(k, 1); if (changedInt(k, 0, 1)) { invalidate(currentScope(k)) }; endRestartGroup(k) { c -> val n = rememberInt(c, 0) { 1 }; Label(c) } }` is declared, and `fun main() { val c = startComposition { k -> Label(k) }; recomposeComposition(c) }` runs
- **THEN** the program ends with `panic: composition misuse: slot outside a group` and status 101

### Requirement: Groups Are Balanced
Ending a group when none is open in the current scope MUST be the misuse `end of a group that was not started`. `endGroup` closing a restart group, or `endRestartGroup` closing a group of another kind, MUST be the misuse `group ended with the wrong kind`. A content or restart function returning while a group it started is open MUST be the misuse `group left open`.

#### Scenario: Ending a group that was not started
- **WHEN** `fun main() { val c = startComposition { k -> endGroup(k) } }` runs
- **THEN** stderr is exactly `panic: composition misuse: end of a group that was not started` and a newline, and the exit status is 101

#### Scenario: A group left open
- **WHEN** `fun main() { val c = startComposition { k -> startGroup(k, 1) } }` runs
- **THEN** the program ends with `panic: composition misuse: group left open` and status 101

#### Scenario: A group ended with the wrong kind
- **WHEN** `fun main() { val c = startComposition { k -> startRestartGroup(k, 1); endGroup(k) } }` runs, or its content is `{ k -> startGroup(k, 1); endRestartGroup(k) { c -> } }`
- **THEN** each form ends with `panic: composition misuse: group ended with the wrong kind` and status 101

### Requirement: One Pass at a Time
A thread MUST run at most one pass at a time. Starting, recomposing, setting the content of or disposing any composition while a pass runs on the thread MUST end the program with `composition misuse: composition operation during a pass`.

#### Scenario: A composition operation during a pass
- **WHEN** `fun main() { val c = startComposition { k -> }; setCompositionContent(c) { k -> println(recomposeComposition(c)) } }` runs
- **THEN** the program ends with `panic: composition misuse: composition operation during a pass` and status 101

#### Scenario: Starting or disposing a composition during a pass
- **WHEN** a composition's new content, set by `setCompositionContent`, starts another composition, or disposes its own composition
- **THEN** each form ends with `panic: composition misuse: composition operation during a pass` and status 101

### Requirement: Dumping a Composition
Dumping a composition MUST give its group tree as text, one line per group and per slot, each indented by two spaces per level of depth, starting with the line `root`. A group's line MUST be `restart <key>`, `replaceable <key>`, `movable <key> (<data key>)` or `node <key> #<node> kind <kind>`, followed by ` invalid` for an invalid restart scope. A group's slots MUST follow its line, one level deeper and before its children: `word <tag> = <value>` or `slot <type name>`.

#### Scenario: A dump of groups, slots and nodes
- **WHEN** `fun main() { val c = startComposition { k -> startRestartGroup(k, 10); val n = rememberInt(k, 0) { 3 }; startGroup(k, 20); val node = startNodeGroup(k, 30, 5); endGroup(k); endGroup(k); endRestartGroup(k) { c -> } }; print(dumpComposition(c)); disposeComposition(c) }` runs
- **THEN** stdout is exactly the lines `root`, `  restart 10`, `    word 0 = 3`, `    replaceable 20`, `      node 30 #1 kind 5`, `nodes` and `  #1 kind 5`

### Requirement: Dumping the Node Tree
A composition's dump MUST end with its applier's own dump. The node tree applier's dump MUST be a line `nodes`, then one line `#<node> kind <kind>` per node, in tree order, indented by two spaces per level below its root node.

#### Scenario: Nested nodes in a dump
- **WHEN** a composition's content starts a node group of kind 1 holding node groups of kinds 2 and 3, and is dumped
- **THEN** the dump ends with the lines `nodes`, `  #1 kind 1`, `    #2 kind 2` and `    #3 kind 3`

### Requirement: The Composer Protocol Module
The bundled library module `/prism/composer`, outside the prelude, MUST declare the composer protocol for code written by hand in the form composables are lowered to. Composers, compositions and scopes MUST be `Int` words there. Each public function MUST behave as the requirement it names describes, through a private external of the compose runtime.

#### Scenario: The protocol module is imported
- **WHEN** a file without imports declares `fun main() { val c = startComposition { k -> } }`
- **THEN** the type checker emits `E4001` for `startComposition`, and the file runs once it declares `import /prism/composer` and disposes `c`

### Requirement: Composition Functions of the Protocol
`/prism/composer` MUST declare `startComposition(content: escaping (Int) -> Unit): Int`, `setCompositionContent(composition: Int, content: escaping (Int) -> Unit)`, `recomposeComposition(composition: Int): Bool`, `disposeComposition(composition: Int)`, `rootScope(composition: Int): Int` and `dumpComposition(composition: Int): String`.

#### Scenario: The root scope of a composition
- **WHEN** a program prints `rootScope(c) == rootScope(c)` for a started composition `c`, invalidates it and recomposes
- **THEN** it prints `true`, and the recomposition runs the root content again and returns `true`

### Requirement: Group Functions of the Protocol
`/prism/composer` MUST declare `startGroup(composer: Int, key: Int)`, `startMovableGroup(composer: Int, key: Int, dataKey: Int)`, `startRestartGroup(composer: Int, key: Int)`, `startNodeGroup(composer: Int, key: Int, kind: Int): Int`, which returns the group's node, `endGroup(composer: Int)`, which ends replaceable, movable and node groups, and `endRestartGroup(composer: Int, restart: escaping (Int) -> Unit)`.

#### Scenario: A node group returns its node
- **WHEN** a content prints the result of `startNodeGroup(k, 1, 7)` on two passes
- **THEN** it prints the same number twice

### Requirement: Scope Functions of the Protocol
`/prism/composer` MUST declare `skipping(composer: Int): Bool`, `skipToGroupEnd(composer: Int)`, `currentScope(composer: Int): Int` and `invalidate(scope: Int)`.

#### Scenario: Skipping is false on the first pass
- **WHEN** a content starts a restart group and prints `skipping(k)` in it, and is recomposed through its root scope
- **THEN** it prints `false` and then `true`

### Requirement: Slot Functions of the Protocol
`/prism/composer` MUST declare `rememberInt(composer: Int, typeTag: Int, calc: () -> Int): Int`, which remembers a word slot of tag `typeTag`, and `changedInt(composer: Int, typeTag: Int, value: Int): Bool`, which compares `value` with a word slot of tag `typeTag`.

#### Scenario: Two tags at two positions
- **WHEN** a group remembers `rememberInt(k, 1) { 10 }` and then `rememberInt(k, 2) { 20 }`, and is recomposed
- **THEN** both values are returned again without running either calculation, and the dump shows `word 1 = 10` and `word 2 = 20`

### Requirement: Composition Operations Take Constant Time
Starting and ending a group, reading, writing and comparing a slot, and invalidating a scope MUST each take amortized constant time in the size of the composition. A pass MUST take time proportional to the bodies it runs and the groups and slots it visits, removes and releases, plus O(k log k · d) to order its k invalid scopes in a tree of depth d.

#### Scenario: Recomposing one scope does not depend on the composition's size
- **WHEN** `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-compose --test runtime_timing -- --ignored --nocapture` recomposes one invalid leaf scope in compositions of 1,000, 10,000 and 100,000 groups
- **THEN** the median time of that pass grows by less than a factor of 2 from the smallest composition to the largest, and so does the first composition's time per group

### Requirement: Steady-State Passes Allocate Nothing
Once its buffers have grown to fit the composition, a pass that creates no group and no slot MUST allocate and free no memory that the runtime owns.

#### Scenario: A steady-state pass allocates nothing
- **WHEN** `crates/prism-compose/tests/allocations.rs` composes 1,000 restart groups holding a word slot each, then runs 100 passes that each recompose the root, with every child skipping, and one invalid leaf scope
- **THEN** the counting allocator of the test records no allocation and no free during the last 98 passes

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
The bundled module `/prism/compose/runtime` MUST be a prelude module. It MUST declare `Composition`, `composition`, `setContent`, `recompose` and `dump`, `remember`, `key`, `RecomposeScope`, `currentRecomposeScope` and `invalidate`, and the state declarations `State`, `MutableState`, `mutableStateOf`, `value`, `read`, `update`, `edit` and `SideEffect`, and a file's own declaration or import of one of those names MUST shadow it.

#### Scenario: Compose primitives need no import
- **WHEN** a file without imports declares `compose fun Counter() { val id = remember { 7 }; val s = currentRecomposeScope() }` and `fun main() { val c = composition { Counter() } }`
- **THEN** it typechecks, `remember` and `currentRecomposeScope` resolve to `/prism/compose/runtime`, and `prism run` exits with status 0

#### Scenario: A file's own `key`
- **WHEN** a file declares `fun key(n: Int): Int = n` and `fun f(): Int = key(3)`
- **THEN** it typechecks, and the call resolves to the file's own `key`

#### Scenario: A file's own mutableStateOf
- **WHEN** a file declares `fun mutableStateOf(n: Int): Int = n + 1` and `fun main() { println(mutableStateOf(1)) }`
- **THEN** it runs and prints `2`: the call resolves to the file's own `mutableStateOf`

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

### Requirement: A When With Composable Tests Runs in Its Own Group
A `when` whose patterns or guards contain a composable call outside closures MUST run, as a whole, in a replaceable group of its own around its tests and its branches' groups, so that which tests ran never changes how the calls after the `when` match their groups.

#### Scenario: Composable tests of a when run in its group
- **WHEN** `compose fun Inner(n: Int) { println("inner $n") }`, `compose fun Check(n: Int, limit: Int): Bool { Inner(n); return n > limit }`, `compose fun Pick(limit: Int) { when { Check(1, limit) -> println("first"); Check(2, limit) -> println("second"); else -> println("none") }; val last = Check(3, limit) }`, `fun screen(limit: Int): compose () -> Unit = { Pick(limit) }` and `fun main() { val c = composition(screen(0)); println("--"); c.setContent(screen(1)); print(c.dump()) }` run
- **THEN** after `--`, stdout is `inner 2` and `second`, and the dump shows `Pick`'s group holding the `when`'s group, with the two groups of `Check` that the second pass ran, and after it the group of the last `Check`

### Requirement: Calls in a Loop With a Continue Run in Call-Site Groups
In a loop that a `continue` targets, each composable call and each `remember` made directly in the loop's group, outside its branches' and inner loops' groups, MUST run in a replaceable group of its own, keyed by its ordinal among them in the loop. Calls at different call sites MUST NOT match each other's groups or slots. `key` calls MUST NOT get such a group, so keyed items still reorder across iterations.

#### Scenario: A continue keeps calls apart
- **WHEN** `compose fun Inner(n: Int) { println("inner $n") }`, `compose fun Outer(skip: Int) { for (i in 0..1) { val a = remember { 100 + i }; Inner(1); if (i == skip) { continue }; val b = remember { println("remember $i"); 200 + i }; Inner(2); println("$i $a $b") } }`, `fun screen(skip: Int): compose () -> Unit = { Outer(skip) }` and `fun main() { val c = composition(screen(0)); println("--"); c.setContent(screen(5)) }` run
- **THEN** after `--`, stdout is `0 100 201`, `remember 1`, `inner 2` and `1 101 201`: each `remember` and each call of `Inner` matches only the groups of its own call site, in order, so no value moves from one call site to another and no call trusts change bits given at another

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

### Requirement: A Mutably Borrowed Local Forwards Nothing
A local that is borrowed mutably, by a borrow or a closure, or assigned anything but a copy of a parameter, a constant or a top-level value, MUST NOT count as forwarding what it copied from then on: an argument read from it is uncertain.

#### Scenario: A value written through a borrow is not forwarded
- **WHEN** `compose fun Inner(n: Int) { println("inner $n") }`, `compose fun Outer(n: Int, m: Int) { var x = n; val r = &mut x; *r = m; Inner(x) }`, `fun screen(m: Int): compose () -> Unit = { Outer(5, m) }` and `fun main() { val c = composition(screen(1)); c.setContent(screen(2)) }` run
- **THEN** stdout is `inner 1` and `inner 2`: a local borrowed mutably is no longer known as a copy of `n`, so its argument is uncertain and compared

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

#### Scenario: Taking a state's value apart rejected
- **WHEN** `struct U { val name: String, val n: Int }` and `compose fun C(u: State<U>) { val (name, n) = u.value }` are checked
- **THEN** the type checker emits `E4017`, saying that the value of the state `u` cannot be taken apart, with help destructuring its borrow `&u.value`

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
- **WHEN** `fun main() { val todos = mutableStateOf(listOf(1, 2, 3)); val n = todos.read { it.size * 10 }; println(n) }` runs
- **THEN** stdout is `30`

### Requirement: Updating a State With a Closure
`s.update(transform)` on a `MutableState<T>` MUST move the value out of the cell, call `transform` with it, and move its result into the cell, outside composable contexts. While `transform` runs, the cell MUST hold no value.

#### Scenario: Updating a counter
- **WHEN** `fun main() { val count = mutableStateOf(4); count.update { it * it }; println(count.value) }` runs
- **THEN** stdout is `16`

### Requirement: Editing a State in Place
`s.edit(editor)` on a `MutableState<T>` MUST call `editor` with a mutable borrow of the cell's value, outside composable contexts. While `editor` runs, the cell MUST be borrowed mutably.

#### Scenario: A list state edited from a handler
- **WHEN** `struct Todo { val title: String }` and `compose fun Todos(todos: State<List<Todo>>) { println("count ${todos.value.size}") }` are declared and `fun main() { val todos = mutableStateOf(listOf(Todo("a"))); val c = composition { Todos(todos) }; val onClick = { todos.edit { it.add(Todo("b")) } }; onClick(); println(c.recompose()) }` runs with `--check-allocations`
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

### Requirement: A Reload Recomposes the Scopes of Affected Composables
Applying a reload MUST mark invalid, in every live composition of the program thread, the nearest restart scope enclosing each group of an affected composable body, or the composition's root when no restart scope encloses it. A composable body, function or closure, MUST be affected when its code is in a unit the reload defines, or when its file imports a reloaded file, directly or through other files.

#### Scenario: Editing a composable body recomposes it with its state
- **WHEN** the counter screen, run headless under `--watch` with `--check-allocations`, gets the script `click "+"` three times, `edit main.prism main.prism.v2`, where `main.prism.v2` changes `Text("Count: ${c.value}")` to `Text("Clicks: ${c.value}")`, and `dump`
- **THEN** the dump holds `text #2 16,16 72x20 "Clicks: 3"`: the scope reran the new body, the count survived, and the text node is the one created first

#### Scenario: Editing a helper of another file recomposes its callers
- **WHEN** a headless app run under `--watch` has a `Screen` composable in `app/main.prism` that shows `Text(label(2))`, where `lib/labels.prism` declares `public fun label(n: Int): String = "n = $n"`, and the script is `edit lib/labels.prism lib/labels.prism.v2`, which makes it return `"value $n"`, then `dump`
- **THEN** the dump holds a text node reading `value 2`, although no composable's code was compiled again

### Requirement: Recomposition After a Reload Skips Unchanged Children
The pass after a reload MUST run each invalidated scope's body. A skippable composable it calls whose arguments did not change and whose own scope is not invalid MUST skip, as in any pass. In that pass, every argument a skippable composable keeps MUST be compared with the kept one, even when its call site's change bits call it same or static, since the call site may be new code passing another value: one whose argument changed MUST run its body with the new arguments and keep them.

#### Scenario: A sibling in an unaffected file skips
- **WHEN** a headless app run under `--watch` has a `Screen` in `app/main.prism` that calls `Badge(1)`, a skippable composable of `lib/badge.prism` that prints `badge`, and the script edits `app/main.prism` to change another text of `Screen`
- **THEN** `badge` is printed once, on the first composition, and not after the reload

#### Scenario: An edited argument reaches a skippable composable of another file
- **WHEN** a headless app run under `--watch` has a `Screen` in `app/main.prism` that calls `Label(5)`, a skippable composable of `lib/label.prism` showing `n = $n`, and the script dumps, edits the call to `Label(6)` and dumps again
- **THEN** the second dump holds the text node reading `n = 6`, although the constant's change bits are static

#### Scenario: Arguments forwarded in another order are compared after a reload
- **WHEN** a headless app run under `--watch` has `compose fun Outer(a: Int, b: Int) { Inner(a, b) }` in `app/main.prism`, `Inner` a skippable composable of `lib/inner.prism` showing `a = $a b = $b`, called as `Outer(c.value, 2)` with `c` at `1`, and the script edits the body of `Outer` to `Inner(b, a)`, dumps, clicks a button adding 10 to `c` and dumps
- **THEN** the dumps read `a = 2 b = 1` and then `a = 2 b = 11`

### Requirement: State Whose Type Is Unchanged Survives a Reload
A group whose key matches after a reload MUST keep its slots. A remembered value or state cell whose type fingerprint is unchanged MUST keep its value, even when its calculation was edited.

#### Scenario: Editing a remember calculation keeps the value
- **WHEN** the counter screen, run headless under `--watch`, gets the script `click "+"` twice, then an edit of `remember { mutableStateOf(0) }` to `remember { mutableStateOf(10) }`, then `dump`
- **THEN** the dump holds the text node reading `Count: 2`

### Requirement: State Whose Type Changed Is Reset
After a reload, a slot whose type fingerprint differs from the type the new code reads MUST be discarded at the end of the pass, dropping its value and freeing the state cells it owns, and the new code MUST compute its value again. Every other slot MUST keep its value.

#### Scenario: Changing a remembered type resets that state only
- **WHEN** `compose fun Screen() { Column { Counter(); Likes() } }` runs headless under `--watch` and `--check-allocations`, `Likes` remembering its own `mutableStateOf(0)` shown as `Likes: <n>` with a `like` button, the script clicks `+` twice and `like` once, edits `Counter`'s state to `remember { mutableStateOf("") }` shown as `Name: ${c.value}`, and dumps
- **THEN** the dump shows `Name: ` and `Likes: 1`, and the session exits with status 0: the old cell was freed

### Requirement: Inserted Composable Calls Keep Their Siblings' State
After a reload that adds a composable call, the new call MUST create its groups, and its siblings MUST keep their groups, slots and nodes wherever their keys match.

#### Scenario: Adding a composable call
- **WHEN** the counter screen, run headless under `--watch`, gets the script `click "+"` three times, then an edit that declares `compose fun Hint() { Text("Tip: press +") }` and calls `Hint()` between the `Text` and the `Button`, then `dump`
- **THEN** the dump shows `Count: 3` as node `#2`, then a new text node reading `Tip: press +`, then the button as node `#3`

### Requirement: A Scope Whose Composable's Signature Changed Recomposes From Its Parent
When a reload changes the parameter types of a skippable composable, the restart scopes of its old code MUST not be restarted by their old restart functions. Each MUST be recomposed by the nearest enclosing scope whose restart function is current, and its own kept arguments and remembered values MUST be reset, while its child groups keep their state. A retired scope whose group is started during the pass in which the reload was applied MUST be reset when it starts.

#### Scenario: A composable whose parameters changed restarts with the new code
- **WHEN** a headless app run under `--watch` shows `Label(c)`, where `compose fun Label(count: State<Int>) { Text("n = ${count.value}") }` reads the state itself, the script clicks `+` three times, edits `Label` to `compose fun Label(count: State<Int>, times: Int)` showing `count.value * times` and the call to `Label(c, 10)`, dumps, clicks `+` and dumps again
- **THEN** the first dump shows `n = 30` and the second `n = 40`: the scope restarted by the click runs the new `Label` with its new arguments

#### Scenario: A scope whose signature changed during a pass is reset
- **WHEN** a plain composition's root reads a state and calls `Wrapper("w")`, a non-skippable composable that calls `Label(5)`, where `compose fun Label(n: Int)` remembers `1` and `2` and prints them, and the program, run under `--watch` with `after line 4: edit main.prism main.prism.v2`, writes the state and recomposes twice; the edit makes `Label(n: Int, times: Int)` remember `10` and `20` and print `label 2: $n $times $a $b`, and `Wrapper` call `Label(5, 2)`; line 4 is printed by the root during the first recompose
- **THEN** that recompose prints `label 2: 5 2 10 20`: the new `Label` started the old scope's group during the pass in which the reload was applied, reset its slots, and read no slot the old code wrote

### Requirement: A Reload During a Pass Is Handled When the Pass Ends
A reload applied while a pass runs, as one applied at a function entry inside a plain composition can be, MUST invalidate its scopes when that pass ends, so that they recompose in the next pass.

#### Scenario: A plain composition recomposes reloaded scopes at its next recompose
- **WHEN** `compose fun Show() { println("show 1") }` and `fun main() { val c = composition { Show() }; println("ready"); println(c.recompose()) }` run under `--watch` with the edit script `after line 2: edit main.prism main.prism.v2`, where `Show` prints `show 2`
- **THEN** stdout holds `show 1`, `ready`, `show 2` and `true`

### Requirement: Finding the Scopes of a Reload Takes Linear Time
Finding the scopes a reload invalidates MUST take time linear in the live groups of the program thread's compositions plus the affected bodies, and MUST add no work to passes without a reload.

#### Scenario: Invalidating after a reload in a large composition
- **WHEN** a Rust test of `prism-compose` applies reloads naming one affected body to compositions of 1,000, 10,000 and 100,000 groups
- **THEN** the time to find and invalidate their scopes grows roughly linearly with the groups

## Design Rationale

Decisions from `compose-runtime` (planned autonomously on 2026-10-18, implemented and reviewed on 2026-10-18 and 2026-10-19, and logged in `docs/decisions-for-review.md`). It implements the runtime side of the compose plan of `docs/architecture.md` §5 (`compose-architecture` D1, D6, D7, D10, D11 and D23).

- **The boundary with the other compose changes** (D1). The runtime treats group keys as opaque words and never reads change words: `compose-transform` computes keys from declaration and lexical paths, generates the change words (two bits per parameter and a force bit), the restart functions and the slot type records, and passes its own force bit to the composable a restart function reruns. `state-handles` adds cells and readers over the current scope, `ui-toolkit` an applier, `compose-hot-reload` the reload policy over `Composition::invalidate_where`, and `reactive-app-verification` the counts of live groups, cells and nodes. Rejected: a force word passed by the runtime (it would fix the change-word layout here), and `remember` and `key` here (compiler intrinsics that need the transform).
- **A UI-agnostic crate with a Rust API and a C entry table** (D2). `prism-compose` depends on `prism-runtime` only. Its `COMPOSE_FUNCTIONS` reuses `RuntimeFunction`, with the source-reading test of `RUNTIME_FUNCTIONS`; `prism-codegen` depends on it, `externals::entry_point` searches both tables for the external check, external calls and runtime imports, and `Linker::new` registers both with the JIT. Calling Prism code from Rust goes through `prism_runtime::FunctionValue`, the layout of `closures`. Rejected: tables passed in by the CLI (every lowering entry point would need them), a registry filled at startup, and the compose runtime inside `prism-runtime` (Tier 2 programs without UI keep a minimal runtime).
- **Generational handles; the composer is the pass's identifier** (D3, revising `compose-architecture` D4). Compositions and scopes are 64-bit handles of a 32-bit index and generation, as the state-handle invariant gives `State<T>`, and a scope's handle is its restart group's. Groups of every composition of a thread share one arena. The composer is the identifier of the pass in progress, from a per-thread counter, so validating it is one compare and a composer kept past its pass is caught deterministically. It costs one load and compare per entry point, and makes the protocol memory-safe for any caller. Rejected: raw composer addresses (forgeable), the composition's handle as the composer (a captured one would pass in later passes), and a thread-local current composer without a parameter.
- **One pass at a time per thread** (D4). The state is a `const` thread local, borrowed by each operation and let go before any program code runs (content, restart, drop and equality entries), so re-entrant calls never alias it. Composition operations during a pass are misuse; a drop entry run by the releases after a pass may dispose another composition. Scopes invalidated during a pass go to the next one, so a pass's set of scopes is fixed when it starts. Rejected: a global state behind a lock, nested passes now (only subcompositions need them), and Jetpack's recomposition of scopes invalidated during the pass (an order that depends on timing).
- **The group tree and keyed matching** (D5, implementing `compose-architecture` D6). A start compares the next unvisited old sibling; on the first miss under a parent it builds a reused map from (kind, key, data key) to that parent's unvisited old children in order, so starts are amortized O(1) and equal keys match in order. A group's end commits its new children from a scratch vector, renumbers them and removes the unvisited old ones. Rejected: Jetpack's gap buffer, intrusive child lists, and a linear search on a miss (quadratic on reversed keyed lists).
- **Typed slots through static records; word slots for hand-written code** (D6, revised at apply time). A slot type record holds the 256-bit `hot-reload` fingerprint, size, alignment, drop and equality entries and a name; reads match record pointers, then fingerprint, size and alignment, so a key matched by mistake never yields a value of another layout. Every typed value lives in a block of its own, so its address stays valid until the end of the pass whatever happens to the slot vector; a write of a drop-less value over a slot of its type overwrites it in place, so `changed` allocates nothing in steady state. Word slots hold a 64-bit value and a tag, for `/prism/composer`. Rejected: separate size, alignment and drop arguments (five words a call), 64-bit fingerprints (a collision is a memory-safety bug), inline storage (addresses would move), and interleaving slots with groups.
- **Removals deferred to the end of the pass** (D7). Removed groups, forgotten slots and replaced restart functions and contents are released after the pass's last body returns, in the order they were removed, a removed subtree by an iterative post-order walk: children last first, then slots last first, the restart function and the node. Releasing a replaced restart function at once would free the environment of the code that is running.
- **Restart scopes and their order** (D8). A restart group keeps the owned `(Int) -> Unit` function its last end passed; recomposing a scope calls it in a window of exactly its group within its parent, so a matching start reuses the group and a different one replaces it. A pass sorts its invalid scopes by their paths of positions from the root (O(k log k · d)) and skips a scope that its parent's recomposition already composed or that lies below a group the pass removed, found by walking up from the scope. A parent that skips keeps its children, and a scope below it that recomposes alone in the same pass still matches its own group (both from the review). Rejected: recomposing from the root on every invalidation, ordering by depth only, and building the restart function only once (a reloaded body would keep old captures).
- **Node groups through an applier** (D9, implementing `compose-architecture` D7, refined at apply time). The `Applier` creates, places relative to the previous sibling, removes at the end of the pass and releases nodes, and numbers them for dumps; placement is relative because removals are deferred, so indexes would be ambiguous during a pass. A node context stays *in order* while matches only skip unvisited siblings, so a reused node is not placed again; after a reorder, a kind change or a skip behind a placed node, every node is placed again. A kind change creates a new node and releases the old one. `NodeTree` is the built-in applier for compositions started through the entry points. A model-checked randomized test edits whole trees and compares the node tree after every pass. Rejected: index-based placement, no node groups before `ui-toolkit`, and compositions without an applier.
- **`/prism/composer`, a protocol module outside the prelude** (D10). Private externals and public wrappers over `Int` words, exactly the machine form generated code passes, so executable fixtures write composables as the transform will lower them and pin the runtime from real `.prism` source before the transform exists. Rejected: library test modules in executable fixtures (`standard-library` D13), testing from Rust only, creating `/prism/compose/runtime` here, and wrapper structs (passed by address).
- **Compositions are `Copy` handles disposed explicitly** (D11); the owned `Composition` with drop glue is `compose-transform`'s. **Misuse is one runtime error kind** (D12), `composition misuse: <detail>` without a location, since an entry point has none; invalidating a stale scope is not misuse, because a state's readers may have left the composition. **The allocation check counts undisposed compositions** (D13) through a finish report that `prism-runtime` calls before its leak report. **Dumps** (D14) show keys, kinds, tags, values and node numbers, never addresses or generations.
- **Hot reload interplay** (D15). Keys only decide which slots a group may reuse; slot types and node kinds are checked on every read and start. The runtime holds no code address, so a restart function reaches reloaded code through its descriptor's slot. `Composition::invalidate_where` and `Composition::recompose_forced`, a pass in which nothing skips, were planned for reloads and removed by `compose-hot-reload` (its D6): a reload finds its scopes by body keys and reruns them through the force bit.
- **Costs** (D16). Protocol operations are amortized O(1), a pass costs its bodies, the groups and slots it visits and releases, and the ordering; the runtime's buffers belong to the thread and are reused, so a steady-state pass allocates nothing (`tests/allocations.rs`). The benchmark `tests/runtime_timing.rs` measured about 54 ns per group for a first composition and 83 ns for one invalid leaf scope, flat from 1,000 to 100,000 groups. Startup is unchanged for programs that do not use the runtime. No compiler stage is added.

Decisions from `compose-transform` (planned autonomously on 2026-10-18, reconciled with `compose-runtime`, `receiver-lambdas` and `compose-hot-reload` and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`). It implements the compiler half of the compose plan of `docs/architecture.md` §5 (`compose-architecture` D3 to D5 and D8 to D11) on this capability's runtime.

- **The boundary** (D1). The transform owns the calling convention, groups and keys, stability, change words, kept arguments, restart closures, slot type records and `/prism/compose/runtime` (`Composition`, `composition`, `setContent`, `recompose`, `dump`, `remember`, `key`, `RecomposeScope`, `currentRecomposeScope`, `invalidate`); the runtime gained only `prism_compose_kept_slot` and, at review, `prism_compose_keep`. `state-handles` adds `State<T>` and its reads and writes, `ui-toolkit` nodes, `compose-hot-reload` the recomposition of reloaded scopes. Rejected: leaving `currentRecomposeScope` to `state-handles` (no restart fixture could run before it).
- **An explicit composer and change words** (D3, D4). Every composable takes, after its declared parameters (receiver and context first), the pass identifier as `$composer` and `max(1, ⌈n / 31⌉)` change words: two bits per parameter (uncertain, same, different, static) and a force bit (bit 0 of the first word), set by restart closures and root content adapters. Callers pass static for constants and copies of top-level values, forward their own knowledge of a parameter they pass on, and uncertain otherwise, from a block-local map of what each local copies; a local borrowed mutably, by a borrow or a closure, forgets it (review). Defaults stay caller-evaluated, so no default mask is passed. Cost: constant work per argument. Rejected: a thread-local current pass (hides which composition a call belongs to), a separate force argument, Jetpack's three bits (stability here is static), a default mask.
- **One group per body, groups for regions, no call groups** (D5). A call runs its callee's body in one group the callee starts: a restart group when skippable, a replaceable one otherwise. Keys are 64-bit BLAKE3 hashes chained from the declaration path through construct kinds and ordinals (`if`, `when`, `and`, `or`, `elvis`, `safe call`, `loop`, `key`, closures by the call and parameter they are written for), never offsets, so comments, formatting, edited arguments and inserted calls of other composables keep every key. Same-callee siblings share a key and match in order, which is the ordinal of §5 at no cost, and lets a restart recompose a group without being told its key. A loop's group wraps all its iterations, so `key` groups of different iterations are siblings and reorder. One hash per group, linear whatever the depth. Rejected: a group per call (two groups per call, and the restart function would need the key), a `$key` parameter, source offsets, a group per iteration (defeats `key`), hashing whole lexical paths (quadratic in depth).
- **Calls keep their call site** (D26, from the review). In-order matching of same-key siblings is only right while the `i`-th group of a key always comes from the same call site, which straight-line code, regions, `return` and `break` keep. A `continue` breaks it: in a loop it targets, each composable call and `remember` made directly in the loop's group runs in a call-site group keyed by its ordinal (`H(loop key, "call site", ordinal)`), one group per such call per iteration, nothing elsewhere; `key` calls get none, so keyed items still reorder. A `when` whose patterns or guards compose is a group as a whole (`when 0 tests`). Rejected: a group per iteration or after each `continue` (`if (hidden) { continue }; key(id) { ... }` would lose every item's state on a reorder), uncertain bits in such loops (fixes skipping but not `remember` slots), call-site groups for `when` tests (the transform cannot tell a test apart without another marker).
- **Stability by declared markers, keepability at run time** (D7). A parameter is stable when its type is `Copy` and holds no borrow by declared markers alone (a type parameter by its `Copy` bound), or is an `escaping` non-composable function type or a nullable of one. A composable is skippable when it returns `Unit` and every parameter is stable; `String`, references, content lambdas and unbounded type parameters make it unskippable, and a shared receiver is a reference. A function value is kept only when its descriptor says it is keepable (captures nothing or owns only `Copy` captures); otherwise that call runs in a replaceable group of the same key, which never matches the restart group. One marker lookup per parameter type, no analysis of fields or other files. Rejected: inferring stability from fields (whole-program), treating every function parameter as unstable, rejecting unkeepable handlers, stable shared borrows now.
- **Kept arguments in the restart group's first slots** (D8). Each argument is kept in parameter order, a function value as a duplicate so the body keeps the original: uncertain compares through the slot type's equality glue, different stores, same and static read past the kept slot and store only on a first composition. `Copy` arguments are kept by one call of `prism_compose_keep(composer, record, value, bits)`, which makes that choice and returns the bits the body now knows (review: half the backend time of a switch per parameter); function arguments keep the switch, since they are duplicated only when stored or compared. Rejected: comparisons expanded at call sites (code per site), arguments kept in the restart closure's environment (an allocation per scope end), a compare-without-store entry before measuring.
- **Restart closures read the kept slots** (D10). `F#restart`, `(Int) -> Unit` capturing nothing, reads each kept argument with `prism_compose_kept_slot` in the restart window and calls `F` with the force bit and every parameter same, so ending a restart group allocates nothing. Its hot-reload contract is hashed with `F`'s (`compose-hot-reload` D8), so the two split together. Rejected: captured arguments, a separate `F#body`, dummy arguments with a restart bit.
- **The shape of a composed body** (D11). A keepability test, the restart group, the kept arguments, the skip test (no force, nothing changed, `skipping`) whose path drops the function arguments and skips to the group's end, then the body, its calls passing the composer and change words; the replaceable path shares the body's blocks through a `Bool` that the group's end switches on, so a body is never duplicated.
- **`remember` and `key`** (D12, D13). `remember<T : Copy>` is an intrinsic composable using the current group's slots after the kept arguments, with up to three `Copy`, borrow-free keys (rule-typed), each compared with `changed`; on a miss or a changed key the calculation runs as the only code between the decision and `set_slot`, the seam `state-handles` uses. `key(value: Int) { }` starts a movable group whose data key is the `Int`. Rejected: `vararg` keys (none yet), `remember1`-style names, a group per `remember`, keys of other types through a hash (collisions would share state silently).
- **Composable lambdas** (D14). A composable closure runs in a replaceable group keyed by where it is written and is never a restart scope, so an invalidation inside it reruns the nearest restart scope above, its author. Calls of composable function values pass the caller's composer and change words.
- **Entering composition** (D15, D16). `composition(content)` returns an owned `Composition`, an intrinsic struct whose drop glue disposes it; `setContent`, `recompose` and `dump` take a shared receiver, so `val c = composition { }` works as in Kotlin. A body calling `composition` or `setContent` gets one root content adapter, `<symbol>#root-content`, owning the content and calling it with the composer and the force bit. `RecomposeScope` is a `Copy` intrinsic struct. Rejected: one global composition started by `runApp`, exposing composer words, `mut` receivers, user-defined destructors, a library struct with an `Int` field (forgeable, no drop).
- **What is rejected** (D17). `E4030` for `composition`, `setContent` and `recompose` in a composable context (a handler written inside one is not), `E4008` for composable calls in default values (generators run in the caller, outside the callee's groups; composable defaults are a follow-up), `E4021`/`E4022` for `remember` values and keys and `E4004` for a non-`Int` `key`. Dropping a `Composition` during a pass stays a runtime misuse. `/prism/composer` stays public and unchanged as the protocol's hand-testing surface (D18).
- **Generics and hot reload** (D21, D22). A generic composable is composed once, before instantiation: stability from bounds, keys from its declaration path, slot types substituted per instance, so instances of one generic among the same siblings share a key and a reorder recreates only their slots. Keys never depend on positions, so a reloaded body keeps the state of every group whose construct path did not change; a reloaded scope is recomposed through `skipping`, with no bit of its own.
- **Costs** (D24). One linear walk per composable body before instances, a terminator scan for other bodies, one hash per group, one marker lookup per parameter type, one small restart body per skippable composable; nothing per instance or across files. `compile_timing.rs` times the transform as the `compose` stage on five shapes, all roughly linear; at 4096 skippable composables of four parameters it takes 38 ms, and `lower` and `clif` about 0.14 ms per composable. A skipped call costs one group start and end and one runtime call per stable parameter, and allocates nothing for `Copy` arguments and one-word handlers.

Decisions from `state-handles` (planned autonomously on 2026-10-18, reconciled with the archived compose changes and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`). It implements the state layer of the compose plan of `docs/architecture.md` §5 (`compose-architecture` D12 to D15 and the state half of D16):

- **The boundary** (D1). This capability's state layer owns the handle types, `mutableStateOf`, `value`, `read`, `update`, `edit`, the cell arena with its flags, readers and owners, the `remember` window that attaches cells to a slot, `SideEffect`, and a Rust API that runs code as a handler of a composition (`Composition::run_handler`). Writes only invalidate: recomposing stays explicit (`c.recompose()`, later `runApp`'s frame loop), event dispatch is `ui-toolkit`'s, and counting live cells as leaks is `reactive-app-verification`'s. Rejected: a "recompose after each handler" call here.
- **Kotlin's names, closures where Kotlin aliases** (D2). `mutableStateOf`, `State`, `MutableState` and `.value` are Jetpack Compose's; `update { it + 1 }` is `MutableStateFlow.update`'s shape. `read` and `edit` have no Kotlin counterpart: Kotlin reads a list state by aliasing it and changes one by rebuilding it, while single ownership needs a scope the runtime can guard to borrow a cell's value outside composition. The closures take plain one-parameter function types, so `todos.edit { it.add(Todo("x")) }` reads like Kotlin; a receiver lambda for `edit` was rejected because the receiver's members would shadow the handler's names. Left out, each a follow-up: `derivedStateOf`, `snapshotFlow`, `mutableStateListOf`, mutation policies and `by` delegation.
- **Handles are intrinsic structs with unconditional markers** (D3, `compose-architecture` D12). Both are one word, `Copy` for every `T` (the catalog's markers hold for every argument, unlike a declared generic struct's), invariant, and stable parameters. `MutableState<T>` converts to `State<T>` as a subtype step that keeps the word, and a lookup on a `MutableState` that misses is retried on `State`, as Kotlin's `MutableState<T> : State<T>` makes it. Rejected: one handle type (a `State<T>` parameter says the composable does not write), a general nominal subtyping mechanism (traits come later), duplicated declarations per handle, and conditional markers (a `State<String>` would not be `Copy`).
- **What `s.value` is** (D5, refining `compose-architecture` D13). For a `Copy` `T` it is a copy, anywhere, through `BuiltinFunction::StateValue`. For any other `T` it is a place only in composable contexts, borrowed shared in place (moving it out or taking it apart is `E4017`, naming "the value of the state"), and `E4033` elsewhere. It is sound because during a pass no state is written and no cell is freed, and a borrow taken in a composable context reaches only frames of that pass. Writes are whole values through a call taking the handle by value, so a handler's captured `val` handle writes (`escaping-closures` D7) and part writes go through `edit`. The rule costs one marker lookup and a test of the composable flag per access. Rejected: a place for `Copy` values too, run-time-checked place borrows released at their last use, a closure-only API, and Kotlin's silent `s.value.x = v`.
- **`read`, `update` and `edit` under run-time borrow flags** (D6). Each lowers to a begin call, the closure call and an end call; a cell is free, shared with a count, empty (while `update`'s transform holds its value) or exclusive, and a conflicting access is `state accessed while borrowed`, as `RefCell`'s. The begin calls' references hold no loan, and the closure types keep them from escaping (borrow-free `R`, `Unit` for `edit`). Rejected: a runtime that calls the closure itself (its machine signature depends on `T`) and static exclusivity (every copy of a handle aliases the cell).
- **Three codes, one per fix** (D7). `E4031` for a write in a composable context (move it to a handler or a `SideEffect`), `E4032` for `mutableStateOf` in one (wrap it in `remember`), `E4033` for a non-`Copy` `s.value` or any mutable use of `s.value` outside composable contexts (use `read`, `edit` or `update`). A non-composable closure inside a composable, a `remember` calculation and a default value are not composable contexts, so the runtime checks what they do. Rejected: one code for all three, reusing `E4008`, and a warning for unremembered state.
- **Cells in an arena, values in blocks that never move** (D8). A cell holds its slot type record, its value in a block of the record's size, its flag, readers and release mark; handles are `compose-runtime`'s generational handles with generations from 1, so the word 0 is never a handle. Every entry validates the handle and compares the record it is given with the cell's, so a handle never reaches a cell of another type, even across a reload. Rejected: values inline in the arena's vector (growing it would move borrowed values), values in the owning slot, a global locked arena, and skipping the record check.
- **Reads register the innermost restart scope** (D9). A read during a pass, in a composable, a content lambda or a plain function it calls, registers the pass's current scope once per cell (a vector with a membership test, a hash set beyond eight, pruned of scopes that left their composition before it grows); a read outside a pass registers nothing. A write clears the readers, so a scope that stopped reading recomposes at most once more; clearing a scope's old reads when it recomposes is a follow-up. Rejected: registering groups rather than scopes, and no deduplication.
- **Equal writes invalidate nothing** (D10, refining `compose-architecture` D16). A `value =` write or a compound assignment compares through the record's equality entry, the type's equality glue (`NaN` is never equal); an equal value is dropped and the cell kept, Jetpack's `structuralEqualityPolicy`. `update` and `edit` always invalidate. The runtime compares, not the write site, so code does not grow with writes. Rejected: comparing in generated code, invalidating on every write, a configurable policy, and recomposing until nothing is invalid.
- **State is read-only during composition, at run time too** (D11, `compose-architecture` D14). Writes, `take` and `begin edit` during a pass are `state written during composition`; a creation during a pass needs a creation owner, else `state created during composition outside remember`. A `remember` miss brackets its calculation with `begin remember` and `end remember`, which attaches the cells created since to the slot written; a top-level initializer runs as program code between `prism_compose_begin_program` and `end_program`. From the review: until a composition starts the program scope is a no-op (a process-wide flag, one load), so programs without compose never touch the runtime's state. Rejected: deferring writes to the end of the pass, inspecting remembered values for handles, and a static rule for plain functions (a whole-program analysis).
- **Each cell has one owner** (D12, `compose-architecture` D15). A cell created in a `remember` window belongs to its slot and is freed when the slot is released (a changed key, a removed group, a disposed composition), after the slot's value, at the end of the pass; a slot holding cells is never overwritten in place. One created by a handler or side effect belongs to its composition, freed when it is disposed; any other belongs to the program, freed only under `--check-allocations`, before live compositions are counted. A cell released while a closure member holds it becomes stale at once and is freed when that call ends. Rejected: reference counting, freeing program cells at every exit, and freeing a borrowed cell at once.
- **`SideEffect`, brought forward from `compose-architecture` D25** (D13). Jetpack's simplest effect: queued each time a pass runs the call, run after the pass and its releases in queue order as handlers of the composition, then dropped. Without it nothing composed could act outside a pass before `ui-toolkit`, so composition-owned cells and stale handles could not be tested from Prism. `LaunchedEffect` and `DisposableEffect` stay deferred.
- **Errors at the access** (D14). An entry that fails records the error's kind and returns a failure result; the generated code tests it and, in a cold block, calls `prism_compose_state_error` with the location, as list bounds checks do. Rejected: no location, location arguments on every call, and a static record per access.
- **Stale handles** (D15). A handle is stale when its index names no cell, its generation differs, its cell was released while borrowed, or its record is not the cell's; any use is `stale state handle`. Indexes at the maximum generation are retired. `==` of handles compares words and never fails.
- **One program thread** (D16). Cells live in the per-thread state; a handle used on another thread, which only Rust tests can do, fails as stale. Snapshots wait for threads.
- **Cells in dumps** (D20). A slot's cells follow it one level deeper as `cell <type name>`, in creation order; program and composition cells are not dumped.
- **Costs** (D21). Per access, a member lookup, a marker lookup and a flag test in the type checker and one to three calls in MIR; nothing per instance or across files. At apply, `state-accesses` at 256, 1,024 and 4,096 grew x4.0 to x5.1 per x4 in every stage, and a read cost 13 to 15 ns with 1,000 to 100,000 readers of one state, each registered once; steady-state reads allocate nothing.

Decisions from `compose-hot-reload` (planned autonomously on 2026-10-18, reconciled with the archived compose changes and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`). It implements `compose-architecture` D23 ("Hot Reload of Composables Keeps State") of `docs/architecture.md` §5; the frame loop's side is in the `ui` Design Rationale, restart contracts in `hot-reload`'s.

- **A hook, and reloads during a pass wait for its end** (D3). `prism_runtime::reload::set_applied_hook`, installed by `Program::start`, calls `prism_compose::reload::reload_applied` on the program thread after the slot writes and before the message. With no pass active it walks at once; during a pass, which only the next-call policy of a plain composition allows, it queues the walk until the pass's releases are done, so the walk never sees a tree being updated, and the scopes recompose in the next pass. Rejected: the frame loop calling compose itself (plain compositions would never learn of reloads); walking mid-pass.
- **The scopes above affected bodies** (D4, refining `compose-architecture` D23). A reload names the composable bodies, functions and closures, of the units it defines (an instance's generic's for an instance unit) and of every file importing a reloaded file, directly or not; one pre-order walk over each live composition invalidates the nearest current restart scope above each of their groups, the root counting as one. That covers non-skippable composables, content lambdas (whose author reruns) and callers of an edited helper. It misses code reached only through a function value from a file that does not import the reloaded one, which updates at its next recomposition. Rejected: the restart scopes of reloaded files only (the three cases above); recomposing everything; a call graph per reload; per-body hashes now (a follow-up). Cost: O(files + imports) in the session, O(live groups) for the walk (16, 13 and 8 ns per group for 1,000 to 100,000 groups, release build).
- **Body keys in summaries** (D5, D15). The transform records each composable body's key (`H("body", path)`, or the closure key) on the composed MIR body, `Body::body_key`, in the codec, and `FileSummary::composable_bodies` and `GenericSummary::composable_bodies` collect them, so a session that loaded a file from the cache has them without MIR. A 64-bit collision only recomposes one scope more. Rejected: a unit word on every group start; `invalidate_where` over restart descriptors (restart scopes only); a side table returned by `compose_file`.
- **No forced pass** (D6, revising `compose-runtime` D15). An invalidated scope reruns through its restart function with the force bit, an invalid group never skips, and every other skippable composable with unchanged arguments skips, so the reload's pass is an ordinary pass.
- **The pass after a reload compares every kept argument** (D17, review). A call site's `same` and `static` change bits describe the code that computed them, which a reload may have changed (`Label(5)` edited to `Label(6)`, `Inner(a, b)` to `Inner(b, a)`), so applying a reload marks every live composition and its next pass compares those arguments as uncertain ones; afterwards every kept argument was written by current code. Rejected: clearing static bits at compile time (a comparison per argument on every pass); comparing only at affected call sites (needs a word per call); invalidating every callee (reruns unchanged subtrees). Cost: one flag test per kept `same` or `static` argument, and in the reload's pass one equality call each.
- **State survives when its key and type match** (D7). Matched groups keep their slots and every read checks the slot's fingerprint, so a remembered value or cell of an unchanged type is kept, one of another type is recomputed and the old one released at the end of the pass with its cells and its old glue, and an edited `remember` calculation of the same type keeps its value, as in Compose and Flutter. Handles held elsewhere reach a freed cell only as `stale state handle`. Slots stay positional within their group, as in Compose. Rejected: resetting on an edited calculation (a hash per `remember` site, and it loses state on tuning edits; a possible opt-in); migrating values (`hot-reload` D7).
- **Retired scopes recompose from their parent and reset their own slots** (D8, correcting `compose-transform` D22). A restart function's contract covers its composable's, so changing a skippable composable's parameter types splits both, and `AppliedReload::retired_slots` names their old slots. The walk marks a restart group whose restart function names one `reset_slots` and moves its invalidation to the nearest current scope; when the new code starts the group, it forgets all its slots onto the pass's release list and keeps its children. A reload applied during a pass records its retired slots for the rest of that pass, so a retired group new code starts there is reset too (review). A new version that is not skippable starts a replaceable group, which does not match, so the old one is removed. Rejected: leaving restart contracts alone (new restart code reads old slots by its new layout); recreating the whole group (loses the children's state); a version word per restart group.
