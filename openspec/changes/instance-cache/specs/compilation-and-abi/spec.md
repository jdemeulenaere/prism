# Compilation and ABI Specification Delta

## RENAMED Requirements

- FROM: `### Requirement: Compilation Avoidance via Interface Hashes`
- TO: `### Requirement: Compilation Avoidance via ABI Hashes`

## MODIFIED Requirements

### Requirement: Deterministic ABI Hash
The compiler MUST compute a deterministic 256-bit BLAKE3 hash of each file's exported ABI that covers exported signatures, enum variant shapes, default parameter values and the definitions of reachable types, and excludes every other private declaration, every function body, generic ones included, and source positions.

#### Scenario: Modifying non-generic function body
- **WHEN** a developer modifies the internal implementation of a public function without changing its signature
- **THEN** the file's exported ABI hash remains unchanged

#### Scenario: Modifying a generic function body keeps the ABI hash
- **WHEN** a developer modifies the body of the exported generic function `public fun <T> keep(x: T): T` without changing its signature
- **THEN** the file's exported ABI hash remains unchanged

#### Scenario: Modifying public function signature
- **WHEN** a developer changes the parameter types or return type of a public function
- **THEN** the file's exported ABI hash changes

#### Scenario: Modifying private helper function
- **WHEN** a private helper function is added, removed, or modified in a source file
- **THEN** the exported ABI hash is unaffected

#### Scenario: Changing a default parameter value
- **WHEN** the default value of a parameter of an exported function changes
- **THEN** the exported ABI hash changes

#### Scenario: Distinct enum variant shapes
- **WHEN** two exported enums with the same name have different variant payload shapes
- **THEN** their ABI hashes differ

#### Scenario: Reordering exported declarations
- **WHEN** exported declarations are reordered within a file without other changes
- **THEN** the exported ABI hash remains unchanged

#### Scenario: Hashes printed in full
- **WHEN** a developer runs `prism abi` on a source file, or `prism check` on a workspace
- **THEN** each ABI hash is printed as `0x` followed by 64 lowercase hexadecimal digits, and `prism abi` prints no other hash

### Requirement: Compilation Avoidance via ABI Hashes
`prism check` and `prism run` MUST skip Phase 2 (type checking, MIR lowering and borrow checking) for a file whose content, module path and direct imports' ABI hashes are those of an earlier successful check by the same compiler, recorded in the build cache (the `build-cache` spec), unless `prism run` needs the file's MIR to build an instance whose record entry is missing. They MUST run Phase 2 for every other file. The report and the exit status MUST be the same either way.

#### Scenario: Modifying non-generic function body
- **WHEN** `lib/text.prism` exports `public fun shout(s: &String): String`, `app/main.prism` imports `/lib/text`, both have been checked, and the body of `shout` changes
- **THEN** the next `prism check` re-checks `lib/text` only, and reuses `app/main`: zero downstream files that import it are recompiled

#### Scenario: Modifying a generic function body re-checks no importer
- **WHEN** `lib/box.prism` exports `public fun <T> keep(x: T): T`, `app/main.prism` imports `/lib/box` and calls `keep(1)`, both have been checked, and the body of `keep` changes
- **THEN** the next `prism check` re-checks `lib/box` only, and reuses `app/main`, whose direct import's ABI hash is unchanged

#### Scenario: Modifying public function signature
- **WHEN** in the workspace of `lib/text` `shout` gains a parameter with a default value, `n: Int = 1`, and `app/main`, whose exported signatures do not mention `lib/text`, is itself imported by `app/cli`
- **THEN** the next `prism check` re-checks `lib/text` and its direct importer `app/main`, and reuses `app/cli`, whose direct import's ABI hash is unchanged

#### Scenario: An incompatible signature change is reported in the importer
- **WHEN** in that workspace the parameter of `shout` becomes `s: Int`, and nothing else changes
- **THEN** the next `prism check` re-checks `app/main` and reports its `E4004`, although `app/main` itself was not edited

#### Scenario: Modifying private helper function
- **WHEN** a private helper function of `lib/text.prism` is added, removed or modified
- **THEN** the next `prism check` re-checks `lib/text` only, and downstream files remain cached

#### Scenario: A failing file is checked again
- **WHEN** a file has type or borrow errors, and `prism check` runs again with no edit
- **THEN** the file is checked again and reports the same diagnostics, because a file with errors is never recorded

#### Scenario: Cached and uncached checks agree
- **WHEN** a workspace is checked once with the cache disabled and once from a warm cache
- **THEN** both runs print the same report and exit with the same status

### Requirement: Reachable Type Changes Re-Check Importers
A change to a struct or enum MUST re-check every file whose direct import's ABI records that type, as a symbol or as a reachable definition, through the ABI-hash keys of the build cache, and no other file because of it.

#### Scenario: A field added to a transitively reached type
- **WHEN** `geo/core.prism` also exports `public fun origin(): Vec = Vec(0)`, `geo/shapes.prism` exports `public fun unit(): Circle = Circle(origin(), 1)`, `app/main.prism` imports only `/geo/shapes` and prints `unit().r`, all three have been checked, and then only `geo/core.prism` changes: `Vec` gains a field `val y: Int` and `origin` returns `Vec(0, 0)`
- **THEN** the next `prism check --explain-cache app/main.prism` prints `  checked geo/core.prism`, `  checked geo/shapes.prism` and `  checked app/main.prism`, and `prism run app/main.prism` prints `1` before and after the change

#### Scenario: A body change in the defining file
- **WHEN** in that workspace only the body of a function of `geo/core` changes
- **THEN** the next `prism check --explain-cache app/main.prism` checks `geo/core` again and reuses `geo/shapes` and `app/main`

### Requirement: Escaping Parameters in the ABI
Phase 1 MUST record whether each parameter of an exported function, and each parameter of every function type in an exported signature, is `escaping`, and the ABI hash MUST cover it, so an importer obtains the flags the exporter declared and is re-checked when one changes.

#### Scenario: Escaping parameters round-trip
- **WHEN** `lib/ui.prism` exports `public fun button(onClick: escaping () -> Unit, register: (escaping () -> Unit) -> Unit): Int` and `app/main.prism` imports `/lib/ui` and declares `fun greet(s: &String) {}` and `fun f(s: String): Int = button({ greet(s) }) { h -> h() }`
- **THEN** `prism abi lib/ui.prism` shows `onClick: escaping () -> Unit` and `register: (escaping () -> Unit) -> Unit`, and `prism check app/main.prism` passes, the first closure owning `s`

#### Scenario: Marking a parameter escaping changes the hashes
- **WHEN** an exported `public fun store(f: () -> Unit): Int` becomes `public fun store(f: escaping () -> Unit): Int`
- **THEN** the file's ABI hash changes, and an importing file is checked again

## REMOVED Requirements

### Requirement: Monomorphization Hash for Generics
**Reason**: The Monomorphization Hash is no longer a Phase 1 hash of an exported generic body's AST recorded in the ABI. It is a Phase 2 hash of every generic's record ("Monomorphization Hash of Generic Records"), which also covers the private types the body holds, its callees' signatures and its source positions, since instances report runtime errors at them. So "Generic body hash ignores formatting" no longer holds: an edit before a generic in its file regenerates its instances, though it re-checks no importer. "Changing generic function body" and "Changing a generic extension body" now say that the ABI hash does not change.
**Migration**: None for programs. Reformatting a file regenerates the instances of the generics whose positions moved.

### Requirement: Interface Hash
**Reason**: The ABI no longer records any body hash, so the ABI hash covers exactly what the interface hash covered and the two are one hash. Check keys use the direct imports' ABI hashes, as the deterministic hermetic builds invariant states. Its scenarios "Changing a generic body keeps the interface hash" and "Changing a signature changes the interface hash" now hold for the ABI hash, under "Deterministic ABI Hash".
**Migration**: `prism abi` no longer prints `Interface Hash:`. The `ABI Hash:` line now has the value the interface hash had.

### Requirement: Instantiation Records for Generic Body Changes
**Reason**: Instances are now planned per program from the files' summaries and keyed by their generic record and type arguments ("Instances Are Planned per Program", "Instances Are Keyed by Their Record and Type Arguments"), and their code is cached. Its scenario "Changing generic function body" moves to the latter and "Changing a generic signature instantiates again" stays there. "Polymorphic calls are not recorded" is replaced by "Polymorphic calls are instantiated with their caller".
**Migration**: `--explain-cache` lists each instance once per program, without the `for <path>` suffix (the `build-cache` spec).

## ADDED Requirements

### Requirement: Monomorphization Hash of Generic Records
Each generic function and extension of a file that passes Phase 2, private ones included, MUST have a Monomorphization Hash: the BLAKE3 hash of the canonical encoding of its generic record, which covers its elaborated MIR with source positions and the definitions of the types that MIR mentions. It MUST NOT be part of the ABI, and it MUST NOT depend on the body of any other declaration.

#### Scenario: Changing a generic function body changes its hash
- **WHEN** the body of the generic function `public fun <T> keep(x: T): T` of `lib/box` changes from `= x` to the block `{ val y = x; return y }`
- **THEN** its Monomorphization Hash changes and the ABI hash of `lib/box` does not

#### Scenario: A private type held by the generic gains a field
- **WHEN** `lib/box.prism` declares the private `struct Tag(val n: Int)`, `fun makeTag(): Tag = Tag(1)` and `public fun <T> tagged(x: T): Int = makeTag().n`, and `Tag` gains a field `val m: Int`, with `makeTag` returning `Tag(1, 2)`
- **THEN** the Monomorphization Hash of `tagged` changes, while the ABI hash of `lib/box`, whose exports do not reach `Tag`, does not

#### Scenario: A callee's signature changes under an unchanged body
- **WHEN** `lib/box.prism` declares `public fun <T> outer(x: T, n: Int): Int = helper(n)` and, after it, `fun helper(n: Int): Int = n`, and only the helper changes, to `fun helper(n: &Int): Int = n`
- **THEN** the Monomorphization Hash of `outer` changes, although its body's text does not, because its call now lends `n`

#### Scenario: A non-generic helper's body change keeps the hash
- **WHEN** in the file with the first `helper` only its body changes, to `n + 0`
- **THEN** the Monomorphization Hash of `outer` is unchanged, since its instances call `helper` by symbol and `outer`'s source positions do not move

#### Scenario: An edit after the generic keeps the hash
- **WHEN** a function declared after `outer` in its file is added, removed or changed
- **THEN** the Monomorphization Hash of `outer` is unchanged, while any edit before `outer` that moves its source, such as an added line, changes it, since its instances report runtime errors at its source positions

#### Scenario: Changing a generic extension body
- **WHEN** the body of the exported extension `public fun <T> Box<T>.get(): &T` changes
- **THEN** its Monomorphization Hash changes and the file's ABI hash does not

#### Scenario: Changing a non-generic extension of a generic type
- **WHEN** the body of the exported extension `public fun Box<Int>.sum(): Int` changes
- **THEN** it has no Monomorphization Hash and the file's ABI hash is unchanged

### Requirement: Instances Are Planned per Program
`prism check` and `prism run` MUST find every instance a program needs: the calls with concrete type arguments in its non-generic bodies and, transitively, the generic calls inside each needed instance, across files and through modules a caller does not import. They MUST find them from each file's summary alone, without reading or rebuilding any file's MIR.

#### Scenario: Transitive instances through a module the caller does not import
- **WHEN** `lib/util.prism` declares `public struct Pair<A, B>(val a: A, val b: B)` and `public fun <T : Copy> twin(x: T): Pair<T, T> = Pair(x, x)`, `lib/box.prism` imports `/lib/util` and declares `public fun <T : Copy> first(x: T): T = twin(x).a`, `app/main.prism` imports only `/lib/box` and declares `fun main() { println(first(7)) }`, and `prism check --explain-cache app/main.prism` runs twice
- **THEN** both runs list `  instantiate lib/box::first<Int>` and `  instantiate lib/util::twin<Int>`, the second reusing every file

#### Scenario: Polymorphic calls are instantiated with their caller
- **WHEN** `app/a.prism` imports `/lib/box` and declares `public fun <T> wrap(x: T): T = keep(x)`, and `app/main.prism` imports `/app/a` and calls `wrap(1)`
- **THEN** `prism check --explain-cache app/main.prism` lists the instances `app/a::wrap<Int>` and `lib/box::keep<Int>`

### Requirement: Instances Are Keyed by Their Record and Type Arguments
Each instance MUST be keyed by its symbol, its generic's Monomorphization Hash and a hash of the definitions, markers included, of every struct and enum its type arguments reach. Its code MUST be generated again when one of these changes, and only then.

#### Scenario: Changing generic function body
- **WHEN** `lib/box.prism` exports `public fun <T> keep(x: T): T` and `public fun <T> drop(x: T): Int`, `app/a.prism` calls `keep(1)`, `app/b.prism` calls `drop("s")`, `app/main.prism` imports both, the program has been run, and the body of `keep` changes
- **THEN** the next `prism run --explain-cache app/main.prism` prints `  checked lib/box.prism`, `  reused app/a.prism`, `  reused app/b.prism`, `  reused app/main.prism`, `  reused instance lib/box::drop<String>` and `  instantiate lib/box::keep<Int>`: re-monomorphization happens only for the instances of that generic

#### Scenario: Changing a generic signature instantiates again
- **WHEN** `app/a.prism` calls `drop(1)` of that `lib/box`, the program has been run, then the parameter of `drop` becomes `x: &T` with its body unchanged and the call becomes `drop(&1)`
- **THEN** the next `prism run --explain-cache` checks `lib/box` and `app/a` again and lists `  instantiate lib/box::drop<Int>`

#### Scenario: A type argument gains a field
- **WHEN** `app/main.prism` imports `/lib/box`, declares the private `struct Thing(val a: Int)` and `fun main() { println(keep(Thing(1)).a); println(keep(2)) }`, the program has been run, and `Thing` becomes `struct Thing(val b: Int, val a: Int)` with the call `keep(Thing(0, 1))`
- **THEN** the next `prism run --explain-cache app/main.prism` prints `1` and `2`, lists `  instantiate lib/box::keep<app/main::Thing>` and `  reused instance lib/box::keep<Int>`, and reuses `lib/box.prism`

#### Scenario: A private helper's signature change regenerates the instances
- **WHEN** `lib/box.prism` declares `public fun <T> outer(x: T, n: Int): Int = helper(n)` and, after it, `fun helper(n: Int): Int = n + 1`, `app/main.prism` prints `outer("a", 1)`, the program has been run, and the helper becomes `fun helper(n: &Int): Int = n`
- **THEN** the next `prism run --explain-cache app/main.prism` prints `1`, checks `lib/box` again, reuses `app/main` and lists `  instantiate lib/box::outer<String>`

#### Scenario: A private helper's body change keeps the instances
- **WHEN** in the program with the first `helper` only its body changes, to `n + 2`
- **THEN** the next `prism run --explain-cache app/main.prism` prints `3`, checks `lib/box` again and lists `  reused instance lib/box::outer<String>`, whose code calls the new `helper` through its slot

### Requirement: Code Generation Avoidance
`prism run` MUST load from the build cache the code of every file and every instance whose code entry exists, without checking, lowering or compiling it again, and MUST generate code for the others alone.

#### Scenario: A body edit regenerates one file's code
- **WHEN** `lib/text.prism` exports `public fun shout(s: &String): String`, `app/main.prism` imports `/lib/text` and prints `shout("a")`, the program has been run, and the body of `shout` changes
- **THEN** the next `prism run --explain-cache app/main.prism` prints `  checked lib/text.prism` and `  reused app/main.prism`, and its output follows the new body
