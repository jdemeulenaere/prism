# Compose Specification Delta

## Purpose
Compose is Prism's native support for declarative UI. This change adds hot reload to the capability that `compose-runtime` creates and `compose-transform` and `state-handles` extend: which scopes a reload recomposes, which slot-table state survives it, and what happens to restart scopes whose composable's signature changed.

## ADDED Requirements

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
