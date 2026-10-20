## ADDED Requirements

### Requirement: Drop and Clone Hooks in the ABI
The ABI MUST record, for each exported struct and enum and in each reachable definition, whether the type has a drop hook and whether it has a clone hook, taken from the declaring file's declarations alone. The ABI hash and the hash of definitions that keys instances MUST cover both flags. A hook's body MUST NOT be part of the ABI.

#### Scenario: Adding a drop hook changes the hashes
- **WHEN** `lib/net.prism` declares `public struct Session(val id: Int)` and `app/main.prism` imports `/lib/net`, and `lib/net.prism` then gains `operator mut fun Session.drop() {}`
- **THEN** the ABI hash of `lib/net` changes, `prism abi lib/net.prism` shows `Session` with a drop hook, and `prism check --explain-cache app/main.prism` checks `app/main.prism` again

#### Scenario: A reachable type's hook is recorded
- **WHEN** `lib/a.prism` declares `struct Secret { val k: Int }`, `operator mut fun Secret.drop() {}` and `public struct Outer { val s: Secret }`
- **THEN** `prism abi lib/a.prism` lists among the reachable types the definition of `Secret` with a drop hook, so an importer's drop glue of `Outer` calls it

#### Scenario: Editing a hook's body recompiles nothing downstream
- **WHEN** `lib/net.prism` declares `public struct Session(val id: Int)` and `operator mut fun Session.drop() { println("closed") }`, `app/main.prism` imports `/lib/net` and declares `fun main() { val s = Session(1) }`, the program has been run, and the hook's body changes to `println("bye")`
- **THEN** the ABI hash of `lib/net` is unchanged, the next `prism run --explain-cache app/main.prism` prints `  checked lib/net.prism` and `  reused app/main.prism`, and the program prints `bye`

### Requirement: Hook Instances Are Planned From Summaries
Each file's summary MUST request the drop hook instance of every generic type with a drop hook that the types its bodies drop or clone, and the type arguments of its generic calls, reach through fields, payloads, nullables, boxes, lists and function environments, and the clone hook instance of each such type that is `Clone` at its type arguments. A generic's summary MUST request them with its own type parameters left as they are, which planning substitutes as it does for calls.

#### Scenario: Hook instances of a generic type
- **WHEN** `lib/own.prism` declares `public struct Owned<T>(val v: T)` and `operator mut fun <T> Owned<T>.drop() { println("drop owned") }`, and `app/main.prism` imports `/lib/own` and declares `fun <T> hold(x: T) {}` and `fun main() { hold(Owned(1)); val xs = listOf(Owned("a")) }`
- **THEN** `prism check --explain-cache app/main.prism` lists `  instantiate lib/own::Owned.drop<Int>` and `  instantiate lib/own::Owned.drop<String>`, and `prism run --check-allocations app/main.prism` prints `drop owned` twice

#### Scenario: A hook instance requested inside a generic
- **WHEN** `app/main.prism` declares `struct Owned<T>(val v: T)`, `operator mut fun <T> Owned<T>.drop() { println("drop owned") }`, `fun <T> wrap(x: T) { val o = Owned(x) }` and `fun main() { wrap(true) }`
- **THEN** `prism check --explain-cache app/main.prism` lists `  instantiate app/main::wrap<Bool>` and `  instantiate app/main::Owned.drop<Bool>`, and running it prints `drop owned`

#### Scenario: Hook requests join the polymorphic recursion check
- **WHEN** `app/main.prism` declares `struct Owned<T>(val v: T)`, `operator mut fun <T> Owned<T>.drop() { val deeper = Owned(listOf<T>()) }` and `fun main() { val o = Owned(1) }`, and `prism check app/main.prism` runs
- **THEN** it reports `E6004` naming `app/main::Owned.drop`, whose instances would request `Owned.drop<List<T>>` without end, as it reports polymorphic recursion through calls across files
