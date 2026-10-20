## ADDED Requirements

### Requirement: Hooks Are Reloaded Like Functions
A reload that changes a hook's body and leaves its type's fingerprint unchanged MUST repoint the hook's slot, so that every later drop or clone of a value of that type runs the new body, whichever code made the value and whichever glue drops it.

#### Scenario: A value made before a reload runs the new hook
- **WHEN** `struct Noisy { val name: String }`, `operator mut fun Noisy.drop() { println("drop " + name) }` and `fun main() { val n = Noisy("a"); println("made"); println("end") }` run with `--check-allocations` under `after line 1: edit main.prism main.prism.v2`, where the hook prints `"bye " + name`
- **THEN** stdout holds `made`, `end` and `bye a`, stderr is exactly `reload: applied main.prism` and `watch: main returned with status 0; waiting for changes`, and the session exits with status 0

### Requirement: Values Keep the Hooks of the Code That Made Them
A reload that adds or removes a type's hook changes its fingerprint, so a value made before it MUST be dropped and cloned as the code that made it would, with or without the hook, and never by glue of the other kind.

#### Scenario: A hook added by a reload does not run for older values
- **WHEN** `struct Ticket { val id: Int }` and `fun main() { val t = Ticket(1); println("made"); println(t.id) }` run with `--check-allocations` under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` adds `operator mut fun Ticket.drop() { println("hook") }`
- **THEN** stdout holds `made` and `1` and no `hook`, and the session exits with status 0

## MODIFIED Requirements

### Requirement: Slots Record the Contract of Their Code
Each slot of the function table MUST record the contract of the code it holds: the type fingerprints of its parameter and result types and, for a closure's code, its environment's captures with their modes, type fingerprints and paths (`t.id`). A type fingerprint MUST be a structural hash of the type. It covers the names, markers, drop and clone hooks' presence, type arguments, fields, variants and payloads of every struct and enum the type reaches, with type arguments substituted.

#### Scenario: A changed field type keeps old values with old code
- **WHEN** `struct P { val x: Int }`, `fun show(p: &P) { println(p.x) }` and `fun main() { val p = P(1); show(p); show(p) }` run under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` declares `struct P { val x: Double }`, `fun show(p: &P) { println(p.x + 0.5) }` and `fun main() { val p = P(1.0); show(p); show(p) }`
- **THEN** stdout holds `1` and `1`, and stderr notes that the signature of `main::show` changed

#### Scenario: A changed state type keeps old handles with old code
- **WHEN** `val Count: MutableState<Int> = mutableStateOf(1)` and `fun main() { var i = 0; while (i < 3) { Count.value += 1; println(Count.value); i += 1 } }` run with `--check-allocations` under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` declares `val Count: MutableState<String> = mutableStateOf("a")` and a `main` that writes it
- **THEN** stdout holds `2`, `3` and `4`, stderr notes that the signatures of `main::Count#init` and `main::Count#get` changed, and the session exits with status 0: the running `main` keeps the handle of its `MutableState<Int>` cell

#### Scenario: Adding a drop hook changes the contracts that mention its type
- **WHEN** `struct T { val id: Int }`, `fun show(t: &T) { println(t.id) }` and `fun main() { val t = T(1); show(t); show(t) }` run under `after line 1: edit main.prism main.prism.v2`, where `main.prism.v2` adds `operator mut fun T.drop() {}` and `show` prints `t.id * 10`
- **THEN** stdout holds `1` and `1`, and stderr notes that the signature of `main::show` changed
