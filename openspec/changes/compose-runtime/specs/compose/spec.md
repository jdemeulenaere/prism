# Compose Specification Delta

## Purpose
Compose is Prism's native support for declarative UI. This capability covers the composition runtime that composable code runs on: compositions and their passes, the slot table of keyed groups and typed slots, deferred removals, restart scopes and invalidation, node groups and the applier that owns nodes, the validation of handles and the errors for misuse, the composer protocol of `/prism/composer`, text dumps, and the runtime's costs. The language side (the `compose fun` transform, `remember`, state handles) is added to this capability by later changes.

## ADDED Requirements

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
- **WHEN** `fun Greeting(k: Int, name: String) { startRestartGroup(k, 3); println("hello " + name); endRestartGroup(k) { c -> Greeting(c, name) } }` is declared, and `fun main() { val c = startComposition { k -> Greeting(k, "a" + "b") }; disposeComposition(c) }` runs with `--check-allocations` and `import /prism/composer`
- **THEN** stdout is `hello ab`, and the program exits with status 0: the restart function and the string it owns are freed

### Requirement: Handles Are Validated
Every entry point MUST validate the handles it is given in constant time. A composer MUST be valid only during the pass that passed it. Compositions and scopes MUST be 64-bit generational handles of a 32-bit index and a 32-bit generation, and an index whose generation reaches its maximum MUST be retired, never reused.

#### Scenario: A composer used after its pass
- **WHEN** `fun Label(k: Int) { startRestartGroup(k, 1); if (changedInt(k, 0, 1)) { invalidate(currentScope(k)) }; endRestartGroup(k) { c -> Label(k) } }` is declared, its restart function calling `Label` with the composer it captured instead of `c`, and `fun main() { val c = startComposition { k -> Label(k) }; recomposeComposition(c) }` runs
- **THEN** the program ends with `panic: composition misuse: composer used outside its pass` and status 101

#### Scenario: Disposing a composition twice
- **WHEN** `fun main() { val c = startComposition { k -> }; disposeComposition(c); disposeComposition(c) }` runs
- **THEN** the program ends with `panic: composition misuse: stale composition handle` and status 101

### Requirement: Composition Misuse Is a Runtime Error
Misusing the composition protocol MUST end the program with the runtime error `composition misuse: <detail>`, which writes no location (execution, "Runtime Errors End the Program"). The details for handles are `composer used outside its pass` and `stale composition handle` ("Handles Are Validated"), for passes `composition operation during a pass` ("One Pass at a Time"), and for slots `no slot to read`, when a read that must find a slot finds none.

#### Scenario: Reading a slot that is not there
- **WHEN** a Rust test calls the entry point that reads a word slot where the cursor is at no slot
- **THEN** the runtime error `composition misuse: no slot to read` ends the test's child process with status 101

### Requirement: Groups Are Balanced
Ending a group when none is open in the current scope MUST be the misuse `end of a group that was not started`. `endGroup` closing a restart group, or `endRestartGroup` closing a group of another kind, MUST be the misuse `group ended with the wrong kind`. A content or restart function returning while a group it started is open MUST be the misuse `group left open`.

#### Scenario: Ending a group that was not started
- **WHEN** `fun main() { val c = startComposition { k -> endGroup(k) } }` runs
- **THEN** stderr is exactly `panic: composition misuse: end of a group that was not started` and a newline, and the exit status is 101

#### Scenario: A group left open
- **WHEN** `fun main() { val c = startComposition { k -> startGroup(k, 1) } }` runs
- **THEN** the program ends with `panic: composition misuse: group left open` and status 101

### Requirement: One Pass at a Time
A thread MUST run at most one pass at a time. Starting, recomposing, setting the content of or disposing any composition while a pass runs on the thread MUST end the program with `composition misuse: composition operation during a pass`.

#### Scenario: A composition operation during a pass
- **WHEN** `fun main() { val c = startComposition { k -> }; setCompositionContent(c) { k -> println(recomposeComposition(c)) } }` runs
- **THEN** the program ends with `panic: composition misuse: composition operation during a pass` and status 101

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
