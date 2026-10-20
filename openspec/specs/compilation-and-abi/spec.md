# Prism Compilation Pipeline & ABI Extraction

## Purpose
The compilation and ABI specification defines the two-phase pipeline as implemented: Phase 1 (`crates/prism-abi`) extracts each file's exported declaration surface and computes one deterministic ABI hash that ignores private symbols and every function body, Phase 2 checks each file against its direct imports' ABIs and keeps a generic record for each generic, whose canonical encoding's hash is its Monomorphization Hash, instances are planned per program from file summaries and keyed by their record and type-argument definitions, and `prism check` and `prism run` skip Phase 2 (and `prism run` code generation) for files and instances whose inputs the build cache (the `build-cache` spec) already holds. Cached `.abi` entries and Tier 2 are planned and described in [`docs/architecture.md`](../../../docs/architecture.md).

## Requirements

### Requirement: Phase 1 Exported Surface Extraction
The compiler MUST extract each source file's exported ABI surface (`public` and `package` structs, enums and function signatures) from its AST without typechecking function bodies.

#### Scenario: Phase 1 surface extraction
- **WHEN** the compiler extracts the ABI of a source file
- **THEN** it records the file's package and the signatures of its `public` and `package` structs, enums and functions, without typechecking function bodies

#### Scenario: Private symbols excluded from the ABI
- **WHEN** a source file declares functions, structs or enums without a visibility modifier
- **THEN** those declarations are absent from the extracted ABI's exported symbols, and a private struct or enum is recorded only as a reachable type definition, when an export reaches it

#### Scenario: Exported function without explicit return type rejected
- **WHEN** a `public` or `package` function omits its return type (`public fun add(a: Int, b: Int) = a + b`)
- **THEN** ABI extraction fails with a missing explicit return type error naming the function and its visibility

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

### Requirement: Structured ABI Types
Phase 1 MUST record every type in an exported signature (parameter, return, struct field, enum payload, exported variable) as a structured type tree, never as source text that must be re-parsed. The ABI hash MUST cover that structure. An importer MUST obtain exactly the type the exporter declared, including nested function, nullable and reference types.

#### Scenario: Higher-order parameter type round-trips
- **WHEN** a file exports `public fun hof(f: ((Int) -> Int) -> Int): Int`
- **THEN** its ABI records a function parameter whose own parameter is the function type `(Int) -> Int` and whose return type is `Int`

#### Scenario: Nullable return of a function type distinguished
- **WHEN** one file exports `public fun pick(f: (Int) -> Int?): Int` and another exports `public fun pick(f: ((Int) -> Int)?): Int`
- **THEN** the first records a function type returning `Int?`, the second records a nullable function type, and their ABI hashes differ

### Requirement: Module-Qualified Nominal Types in the ABI
The ABI MUST record the file's module path (its root-relative path without extension). Each nominal type name in an exported signature MUST be bound to its defining module, using only the file's own declarations and the type names exported by its direct imports, never a function body. A name that is ambiguous or not found MUST be recorded unbound.

#### Scenario: Locally declared type bound to the file's module
- **WHEN** `a/lib.prism` declares `public struct User` and `public struct Order { val user: User }`
- **THEN** the ABI of `a/lib` records the field type of `Order.user` as `User` from module `a/lib`

#### Scenario: Imported type bound to the exporting module
- **WHEN** `a/svc.prism` imports `/model/types`, which exports `public struct Item`, and declares `public fun first(): Item`
- **THEN** the ABI of `a/svc` records the return type as `Item` from module `model/types`

#### Scenario: Binding does not depend on function bodies
- **WHEN** only a function body of a direct import changes
- **THEN** the importer's ABI and ABI hash are unchanged

#### Scenario: Unknown type name recorded unbound
- **WHEN** an exported signature names a type that is neither declared in the file nor exported by a direct import
- **THEN** ABI extraction succeeds and records the name unbound

### Requirement: Copy and Clone Markers in the ABI
The ABI MUST record each exported struct's and enum's marker (`Copy`, `Clone` or none), and the ABI hash MUST cover it. A marker is part of the type's exported contract, so adding, removing or changing it MUST change the ABI hash, as changing a field type does.

#### Scenario: Adding or changing a marker changes the ABI hash
- **WHEN** `public struct Point { val x: Int }` gains `: Clone`, then `: Clone` becomes `: Copy`, or `public enum Color { Red }` gains `: Copy`
- **THEN** the file's exported ABI hash changes at each step, and the ABI records the new marker

#### Scenario: Marker on a private type
- **WHEN** a private struct that no exported signature mentions gains or loses a marker
- **THEN** the exported ABI hash is unaffected

### Requirement: Markers of Referenced Nominal Types
Each nominal type in an exported signature MUST record the referenced struct's or enum's marker, taken from the defining file's declarations alone (its own, or a direct import's exported ones), never from a body, and covered by the ABI hash. An importer MUST learn whether a type is `Copy` or `Clone` from the signature that mentions it, without importing the type's module.

#### Scenario: Referenced type records its marker
- **WHEN** `shop/models` exports `public struct Money : Copy { val cents: Int }` and `public struct Note : Clone { val text: String }`, and `shop/api` imports it and exports `public struct Order { val price: Money, val note: Note }`
- **THEN** the ABI of `shop/api` records `Order.price` as `Money` from module `shop/models` marked `Copy`, and `Order.note` as `Note` marked `Clone`

#### Scenario: Changing a referenced type's marker
- **WHEN** `Money` in `shop/models` loses its `: Copy` marker and nothing else changes
- **THEN** the ABI hashes of both `shop/models` and `shop/api` change

#### Scenario: Built-in Range in an exported signature
- **WHEN** a file that neither declares nor imports a type named `Range` exports `public struct Span : Copy { val r: Range }`, and an importer reads `s.r` by value for `s: Span`
- **THEN** the ABI records `r` as the built-in `Range` type, and the importer's read typechecks as a copy

### Requirement: Definitions of Reachable Types in the ABI
Each file's ABI MUST record the definition (kind, type parameters, marker, and fields or variants with their bound types) of every struct and enum that a value reached through its exports can hold, transitively through fields and payloads, other than its own exported structs and enums. Private types and types of modules the file does not import MUST be included when reached; each definition MUST come from its defining file's declarations alone.

#### Scenario: A private field type is recorded
- **WHEN** `lib/a.prism` declares `struct Secret : Copy { val k: Int }`, `public struct Outer : Copy { val s: Secret, val n: Int }` and the private `struct Helper { val h: Int }`, and `prism abi lib/a.prism` runs
- **THEN** it lists `Outer` among the exported symbols, and among the reachable types the definition of `Secret` from module `lib/a` with its field `k: Int`, and does not list `Helper`

#### Scenario: A type of a module the importer does not import
- **WHEN** `geo/core.prism` exports `public struct Vec : Copy { val x: Int }`, and `geo/shapes.prism` imports `/geo/core` and exports `public struct Circle : Copy { val center: Vec, val r: Int }`
- **THEN** the ABI of `geo/shapes` records the definition of `Vec` from module `geo/core`, so a file importing only `/geo/shapes` can lay out a `Circle`

#### Scenario: Changing a reachable type changes the hashes
- **WHEN** in that workspace `Vec` gains a field `val y: Int`, or the private `Secret` of `lib/a` gains a field
- **THEN** the interface and ABI hashes of `geo/core` and `geo/shapes` change, or those of `lib/a`, while changing the private `Helper` changes no hash

### Requirement: Reachable Definitions Serve Layout Alone
A definition an importer knows only through the ABI of a file that reaches it MUST serve layout alone: its fields and variants MUST stay unknown to the importer's type checker, as if the definition were absent.

#### Scenario: A reachable private type keeps its members hidden
- **WHEN** `lib/a.prism` declares `struct Secret : Copy { val k: Int }`, `enum Mode : Copy { On, Off }`, `public struct Outer : Copy { val s: Secret, val m: Mode }` and `public fun make(): Outer = Outer(Secret(1), Mode.On)`, and `app/main.prism` imports `/lib/a` and prints `make().s.k`, or prints `when (make().m) { On -> 1; else -> 2 }`
- **THEN** `prism check app/main.prism` reports `E4006` for the field `k`, or `E4001` for the variant name `On`, as it does for any type whose definition it does not know

### Requirement: Extension Functions in the ABI
Phase 1 MUST record an exported extension as a function signature together with its receiver mode and its receiver type, bound like any signature type, and the ABI hash MUST cover both. Struct and enum records MUST NOT depend on extensions. The order of exported symbols MUST take the receiver into account, so that reordering declarations never changes the hash.

#### Scenario: Exported extension recorded with its receiver
- **WHEN** `models/user.prism` declares `public struct User { var name: String }` and `public mut fun User.rename(n: String): Unit`
- **THEN** its ABI records a function `rename` with the parameter `n: String`, the return type `Unit`, the receiver type `User` from module `models/user` and the mutable receiver mode

#### Scenario: Adding an extension in another file
- **WHEN** `ui/ui.prism`, which imports `/models/user`, gains `public fun User.shout(): String`
- **THEN** the ABI hash of `models/user` is unchanged, and the ABI hash of `ui/ui` changes

#### Scenario: Adding an extension in the receiver type's file
- **WHEN** `models/user.prism` gains `public fun User.shout(): String`
- **THEN** its ABI hash changes, while its record of `User` is unchanged

#### Scenario: Changing a receiver mode or type changes the ABI hash
- **WHEN** `public fun User.reset(): Unit` becomes `public mut fun User.reset(): Unit`, or `public fun User.size(): Int` becomes `public fun Team.size(): Int`
- **THEN** the exported ABI hash changes

#### Scenario: Extensions sharing a name
- **WHEN** a file exports `public fun describe(): String`, `public fun User.describe(): String` and `public fun Team.describe(): String`, and the three declarations are reordered
- **THEN** the ABI records three distinct functions, and the exported ABI hash is the same in every order

#### Scenario: Extension body or private extension edits
- **WHEN** the body of an exported non-generic extension changes, or a private extension is added, removed or modified
- **THEN** the exported ABI hash is unchanged

### Requirement: Phase 2 Produces a Typed Tree per File
Phase 2 MUST check each file into a typed tree (the `typed-tree` spec) using only the file's own AST and the ABIs of its direct imports, never another file's bodies or typed tree, so that a file's tree is unaffected by edits to the bodies of the files it imports.

#### Scenario: Typed tree from the type checker
- **WHEN** the type checker checks a file
- **THEN** it produces a typed tree in which every node carries its type and span, names are resolved to bindings or declaration identities, calls are normalized to positional order with explicit default slots, and every implicit copy, move, borrow, copy-out and reborrow is an explicit node; the tree is produced even when the file has type errors

#### Scenario: Typed tree independent of dependency bodies
- **WHEN** `lib/text.prism` declares `public fun shout(s: &String): String = nope`, which has a type error in its body, and `app/main.prism` imports `/lib/text` and declares `fun f(s: String): String = shout(s)`
- **THEN** `prism dump-typed-tree app/main.prism` prints the tree of `app/main`, whose call names `lib/text::shout`, while `prism check app/main.prism` reports the `E4001` in `lib/text`

### Requirement: Phase 2 Lowers Each Typed File to MIR
Phase 2 MUST lower the typed tree of every file without type errors to MIR (the `mir` spec) from that tree alone, so that a file's MIR, like its typed tree, depends only on the file and the ABIs of its direct imports. A file with type errors MUST NOT be lowered.

#### Scenario: Lowering to MIR
- **WHEN** a file has no type errors
- **THEN** each function, top-level `val`, default value and closure body is lowered to a place-based control-flow graph of basic blocks: statements assign rvalues to places (a local plus projections) and mark storage, terminators branch, call, return and drop, and `&&`, `||`, `?.`, `?:`, `when` and `for` become explicit branches; `prism dump-mir <file.prism>` prints it

#### Scenario: MIR independent of dependency bodies
- **WHEN** `lib/text.prism` declares `public fun shout(s: &String): String = nope`, which has a type error in its body, and `app/main.prism` imports `/lib/text` and declares `fun f(s: String): String = shout(s)`
- **THEN** `prism dump-mir app/main.prism` prints the MIR of `app/main`, whose call names `lib/text::shout`, while `prism check app/main.prism` reports the `E4001` in `lib/text`

### Requirement: Phase 2 Borrow-Checks Each File's MIR
Phase 2 MUST borrow-check the MIR of every file that typechecks from that MIR alone, never another file's bodies, MIR or ABI, reporting `E5xxx` diagnostics, and MUST elaborate the drops of each file that passes. `prism check` MUST fail on a file with borrow-checker errors as it does on one with type errors.

#### Scenario: Borrow checking on MIR
- **WHEN** a file's MIR has been built
- **THEN** the borrow checker checks every body of it for uses of moved and uninitialized values, reporting `E5xxx` diagnostics; when it reports none, the file's drops are elaborated, and the elaborated MIR is what later stages consume

#### Scenario: Borrow checking independent of dependency bodies
- **WHEN** `lib/text.prism` declares `public fun shout(s: String): String { val t = s; return s }`, which uses `s` after moving it, and `app/main.prism` imports `/lib/text` and declares `fun f(s: String): String = shout(s)`
- **THEN** `prism dump-mir --elaborated app/main.prism` prints the elaborated MIR of `app/main`, while `prism check app/main.prism` reports the `E5001` in `lib/text`

### Requirement: Generic Signatures in the ABI
Phase 1 MUST record each exported function's, extension's, struct's and enum's type parameters, in order, with their bounds. A type parameter in a signature, field or payload type MUST be recorded as a reference to its position, never bound to a struct or enum of the same name. The ABI hash MUST cover the number of type parameters, their bounds and these references, and MUST NOT depend on type parameter names.

#### Scenario: Type parameters recorded by position
- **WHEN** `lib/box.prism` declares `public struct T { val n: Int }`, `public struct Box<T>(val v: T)` and `public fun <A, B : Copy> pair(a: A, b: B): Box<A>`
- **THEN** the ABI records `Box` with one type parameter whose field `v` is type parameter 0, not the struct `T`, and `pair` with type parameters `A` (unbounded) and `B` (bounded by `Copy`), parameters of type parameter 0 and 1, and the return type `Box` from module `lib/box` applied to type parameter 0

#### Scenario: Changing a bound changes the ABI hash
- **WHEN** `public fun <T> keep(x: T): Int` becomes `public fun <T : Copy> keep(x: T): Int`, or `public struct Box<T>(val v: T)` gains a second type parameter
- **THEN** the exported ABI hash changes

#### Scenario: Renaming a type parameter of a struct
- **WHEN** `public struct Box<T>(val v: T)` becomes `public struct Box<U>(val v: U)`
- **THEN** the exported ABI hash is unchanged

### Requirement: Marker Parameters in the ABI
Each exported generic struct or enum, and each reference to a nominal type in an exported signature, MUST record which of the type's parameters condition its marker (those occurring in its field or payload types), read from the defining file's declarations alone and covered by the ABI hash, so an importer knows whether an instantiation is `Copy` or `Clone` without importing the type's module.

#### Scenario: Marker parameters recorded
- **WHEN** `lib/state.prism` exports `public struct Handle<T> : Copy { val id: Int }` and `public struct Box<T> : Copy { val v: T }`, `app/api.prism` imports it and exports `public fun handle(): Handle<String>` and `public fun box(): Box<String>`, and `app/main.prism` imports only `/app/api` and declares `fun f(): Int { val h = handle(); val g = h; val k = h; val b = box(); val c = b; val d = b; return 1 }`
- **THEN** the ABI of `lib/state` records `Handle` with no marker parameter and `Box` with type parameter 0, the ABI of `app/api` records both return types with the same marker parameters, and `prism check app/main.prism` emits `E5001` at the second use of `b` only, because `Handle<String>` is `Copy` and `Box<String>` is not

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

### Requirement: Phase 3 Generates Each File's Code From Its Elaborated MIR
Code generation MUST run on each file only after the file passes Phase 2, and MUST generate the file's code from its elaborated MIR alone, never from another file's bodies or MIR, so that a file's code depends only on the file, the ABIs of its direct imports, the compiler and the target. A file's code MUST NOT contain any instance of a generic, its own generics' included: it calls instances by symbol.

#### Scenario: Code generation independent of dependency bodies
- **WHEN** `lib/math.prism` declares `public fun twice(n: Int): Int = n * 2`, `app/main.prism` imports `/lib/math` and declares `fun main() { println(twice(2)) }`, and the body of `twice` is then changed to `n + n`
- **THEN** `prism dump-clif app/main.prism` prints the same text before and after the change, and `prism run app/main.prism` prints `4` both times

#### Scenario: A dependency with errors stops the run
- **WHEN** `lib/text.prism` declares `public fun shout(n: Int): Int = nope`, which has a type error in its body, and `app/main.prism` imports `/lib/text` and declares `fun main() { println(shout(1)) }`
- **THEN** `prism dump-clif app/main.prism` prints the IR of `app/main`, while `prism run app/main.prism` reports the `E4001` in `lib/text`, runs nothing and exits with status 1

#### Scenario: A generic body change leaves its callers' code unchanged
- **WHEN** `lib/box.prism` declares `public fun <T> pick(a: T, b: T): T = a`, `app/main.prism` imports `/lib/box` and declares `fun main() { println(pick(1, 2)) }`, and the body of `pick` is then changed to `b`
- **THEN** `prism dump-clif app/main.prism` prints the same text before and after the change, while `prism run app/main.prism` prints `1` before it and `2` after it

### Requirement: Generic Records Carry What Instantiation Needs
For each generic function and extension of a file that passes Phase 2, private ones included, the compiler MUST keep a generic record: the elaborated polymorphic MIR of its body, its default value generators and its closures, and the definitions of the structs and enums those bodies mention. A record MUST NOT hold the body of any other declaration: the non-generic functions a generic body calls are called by symbol, and the generics it calls are instances of their own records.

#### Scenario: A generic body calls private helpers
- **WHEN** `lib/box.prism` declares `fun helper(n: Int): Int = n + 1`, `fun <T> inner(x: T): T = x` and `public fun <T> outer(x: T, n: Int): Int { val y = inner(x); return helper(n) }`, and `app/main.prism` imports `/lib/box` and declares `fun main() { println(outer("a", 1)) }`
- **THEN** `prism run --check-allocations app/main.prism` prints `2`, the instance `lib/box::outer<String>` calling `lib/box::helper` through its slot and the instance `lib/box::inner<String>` through its own

### Requirement: Instances Depend Only on Their Record and Type Arguments
Phase 3 MUST build each instance from its generic's record and the definitions of the structs and enums its type arguments mention, never from another body of the generic's file or of the calling file, so that an instance is the same whichever file requests it first, and a file's own code never depends on a generic body.

#### Scenario: An instance of a generic from a module the caller does not import
- **WHEN** `lib/util.prism` declares `public struct Pair<A, B>(val a: A, val b: B)` and `public fun <T : Copy> twin(x: T): Pair<T, T> = Pair(x, x)`, `lib/box.prism` imports `/lib/util` and declares `public fun <T : Copy> first(x: T): T = twin(x).a`, and `app/main.prism` imports only `/lib/box` and declares `fun main() { println(first(7)) }`
- **THEN** `prism run --check-allocations app/main.prism` prints `7`, compiling the instances `lib/box::first<Int>` and `lib/util::twin<Int>`

### Requirement: Escaping Parameters in the ABI
Phase 1 MUST record whether each parameter of an exported function, and each parameter of every function type in an exported signature, is `escaping`, and the ABI hash MUST cover it, so an importer obtains the flags the exporter declared and is re-checked when one changes.

#### Scenario: Escaping parameters round-trip
- **WHEN** `lib/ui.prism` exports `public fun button(onClick: escaping () -> Unit, register: (escaping () -> Unit) -> Unit): Int` and `app/main.prism` imports `/lib/ui` and declares `fun greet(s: &String) {}` and `fun f(s: String): Int = button({ greet(s) }) { h -> h() }`
- **THEN** `prism abi lib/ui.prism` shows `onClick: escaping () -> Unit` and `register: (escaping () -> Unit) -> Unit`, and `prism check app/main.prism` passes, the first closure owning `s`

#### Scenario: Marking a parameter escaping changes the hashes
- **WHEN** an exported `public fun store(f: () -> Unit): Int` becomes `public fun store(f: escaping () -> Unit): Int`
- **THEN** the file's ABI hash changes, and an importing file is checked again

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
- **WHEN** `lib/box.prism` exports `public fun <T> drop(x: T): Int` and, after it, `public fun <T> keep(x: T): T`, `app/a.prism` calls `keep(1)`, `app/b.prism` calls `drop("s")`, `app/main.prism` imports both, the program has been run, and the body of `keep` changes
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

### Requirement: Library Declarations in the ABI
Phase 1 MUST extract and hash a library module's ABI as a workspace file's, from its syntax tree alone. An exported function's entry MUST record whether it is ordinary, intrinsic or external. An exported extension property of any module MUST record its type parameters, receiver type, name, type, writability and whether it is intrinsic. Both MUST be covered by the ABI hash and shown by `prism abi` with their modifiers (`public val Int.dp: Dp`). A getter's body MUST NOT be part of the ABI.

#### Scenario: The ABI of a library module
- **WHEN** the ABI of `/prism/core` is extracted
- **THEN** it holds `public intrinsic mut fun <T> List<T>.add(element: T): Unit`, `public intrinsic val <T> List<T>.size: Int`, the ordinary `public fun <T> List<T>.isEmpty(): Bool` and the property `public val <T> List<T>.indices: Range`, and changing an entry from intrinsic to ordinary changes the module's ABI hash

#### Scenario: Editing a getter recompiles nothing downstream
- **WHEN** `lib/geo.prism` exports `public struct Box : Copy { val w: Int }` and `public val Box.twice: Int get() = w * 2`, `app/main.prism` imports `/lib/geo` and prints `Box(3).twice`, the program has been run, and the getter's body becomes `w + w + 1`
- **THEN** the ABI hash of `lib/geo` is unchanged, and the next `prism run --explain-cache app/main.prism` prints `7`, `  checked lib/geo.prism` and `  reused app/main.prism`

#### Scenario: A property's type is part of the ABI
- **WHEN** the exported `public val Box.twice: Int get() = w * 2` of `lib/geo.prism` becomes `public val Box.twice: Double get() = 2.0`
- **THEN** the ABI hash of `lib/geo` changes, and a file importing it is checked again

### Requirement: Receivers and Contexts in the ABI
Phase 1 MUST record whether a function type in an exported signature has a receiver, and whether that receiver is mutable, as part of its structured type, and MUST record an exported function's context type, bound like any signature type. The ABI hash MUST cover both, and the order of exported symbols MUST take the context into account, so that reordering declarations never changes the hash.

#### Scenario: A receiver function type round-trips
- **WHEN** `layout/column.prism` exports `public struct ColumnScope : Copy {}` and `public fun column(content: ColumnScope.() -> Unit): Unit { content(ColumnScope()) }`, and a file importing `/layout/column` runs `column { }`
- **THEN** the ABI of `layout/column` records the parameter type of `content` as a function type with the shared receiver `ColumnScope` from module `layout/column`, and the importer's closure is checked with that receiver

#### Scenario: Receivers and contexts change the ABI hash
- **WHEN** `public fun column(content: ColumnScope.() -> Unit): Unit` becomes `public fun column(content: (&ColumnScope) -> Unit): Unit` or `public fun column(content: mut ColumnScope.() -> Unit): Unit`, or `context(scope: RowScope) public fun Modifier.weight(w: Double): Modifier` becomes `context(scope: ColumnScope) public fun Modifier.weight(w: Double): Modifier`
- **THEN** the exported ABI hash changes

#### Scenario: Extensions differing by context in the ABI
- **WHEN** a file exports `context(scope: RowScope) public fun Modifier.weight(w: Double): Modifier` and `context(scope: ColumnScope) public fun Modifier.weight(w: Double): Modifier`, and the two declarations are reordered
- **THEN** the ABI records two distinct functions, and the exported ABI hash is the same in both orders

### Requirement: Intrinsic Structs in the ABI
An exported intrinsic struct MUST be an ABI entry with its name, type parameters and markers, marked intrinsic, covered by the ABI hash and shown by `prism abi` with its modifier (`public intrinsic struct RecomposeScope : Copy`). Its layout MUST NOT be part of the ABI: it comes from the compiler, whose version every artifact key holds.

#### Scenario: The ABI of the compose module
- **WHEN** the ABI of `/prism/compose/runtime` is extracted
- **THEN** it holds `public intrinsic struct Composition`, `public intrinsic struct RecomposeScope : Copy` and `public intrinsic compose fun <T> remember(calculation: () -> T): T`, and removing the `Copy` marker of `RecomposeScope` changes the module's ABI hash

### Requirement: Companion Values in the ABI
A public struct and its public companion value MUST both be exported, as two symbols with one name: the struct with its definition and the value with its declared type. Adding, removing or retyping the companion value MUST change the file's ABI hash.

#### Scenario: A struct and its companion value are exported
- **WHEN** `prism abi` runs on a file declaring `public struct Gap : Copy { val size: Int }`, `public struct Gaps : Copy { val None: Gap }` and `public val Gap: Gaps = Gaps(Gap(0))`
- **THEN** it lists both `public struct Gap : Copy` and `public val Gap: Gaps`

#### Scenario: Adding a companion value changes the ABI hash
- **WHEN** `public val Gap: Gaps = Gaps(Gap(0))` is added to a file that declares `public struct Gap` and `public struct Gaps`
- **THEN** the file's ABI hash changes

### Requirement: Attached Parameters and Capture Functions in the ABI
Phase 1 MUST record whether each parameter of an exported function, and each parameter of every function type in an exported signature, is `attached`, and whether an exported intrinsic function is a capture function, and the ABI hash MUST cover both, so an importer obtains what the exporter declared and is checked again when one changes.

#### Scenario: Attached parameters round-trip
- **WHEN** `lib/ui.prism` exports `public fun button(onClick: attached () -> Unit): Int` and `app/main.prism` imports `/lib/ui` and declares `fun f(count: &MutableState<Int>): Int = button { count.value += 1 }`
- **THEN** `prism abi lib/ui.prism` shows `onClick: attached () -> Unit`, and `prism check app/main.prism` passes, the closure capturing `count` through a weak handle

#### Scenario: Marking a parameter attached changes the hashes
- **WHEN** an exported `public fun store(f: escaping () -> Unit): Int` becomes `public fun store(f: attached () -> Unit): Int`
- **THEN** the file's ABI hash changes, and an importing file is checked again

### Requirement: Member Modules Are Inputs of Every Workspace File
Phase 2 of a workspace file MUST see the ABIs of its direct imports and, after them, the members of the built-in types that the member module `/prism/core` declares (`standard-library`, "Built-In Types and Their Members Need No Import"), and nothing else of the library. Instances of the generic members it calls MUST be planned for the program as those of an import's generics, whether or not the file imports `/prism/core`. Phase 2 of a library file MUST see only the ABIs of its direct imports.

#### Scenario: Instances of imported library generics
- **WHEN** `import /prism/collections`, `import /prism/core` and `fun main() { println(listOf(1, 2).map { x -> x + 1 }.size) }` are run
- **THEN** the program's instances include `prism/collections::List.map<Int, Int>`, compiled into its own unit, and the program prints `2`

#### Scenario: Member modules give members only
- **WHEN** a file without imports declares `fun f(xs: &List<Int>): Int = xs.size` and `fun g(): Int = listOf(1).size`
- **THEN** `f` typechecks, and `g` is rejected with `E4001` for `listOf`, which `/prism/collections` declares

#### Scenario: Instances of generic members
- **WHEN** a file importing `/prism/collections` and `/prism/system`, and not `/prism/core`, declares `fun main() { val xs = listOf("a"); exitProcess(if (xs.isNotEmpty()) 3 else 4) }` and runs with `prism run --explain-cache`
- **THEN** the program's instances include `prism/core::List.isNotEmpty<String>`, and the exit status is 3

### Requirement: Operator Functions and Equality Operators in the ABI
Phase 1 MUST record whether an exported function is `operator`, and, in the record of an exported struct or enum and in its definition among the reachable types, whether it has an `equals` operator; the ABI hash and the structural hash of definitions MUST cover both. Editing an operator function's body MUST change neither.

#### Scenario: Marking a function operator changes the hash
- **WHEN** `public fun Vector2.plus(other: Vector2): Vector2` in `geometry/vector.prism` becomes `public operator fun Vector2.plus(other: Vector2): Vector2`
- **THEN** the exported ABI hash of `geometry/vector` changes, and importers are re-checked

#### Scenario: Adding an equality operator changes the type's definition
- **WHEN** `models/user.prism`, which exports `public struct User { val id: Int, val name: String }`, gains `public operator fun User.equals(other: &User): Bool = id == other.id`
- **THEN** its ABI records that `User` has an `equals` operator, its ABI hash changes, and the instance key of any instance whose type arguments reach `User` (`List<User>`'s glue) changes

#### Scenario: Editing an operator body recompiles nothing downstream
- **WHEN** the body of the exported `equals` or `plus` of `models/user` changes
- **THEN** the exported ABI hash is unchanged, so no importer is re-checked

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

### Requirement: Vararg Parameters in the ABI
Phase 1 MUST record whether each parameter of an exported function is a vararg, with its type recorded as the `List<E>` its body sees, and the ABI hash MUST cover the flag, so an importer packs the elements of its calls and is checked again when the flag changes. `prism abi` MUST print such a parameter as `vararg name: E`.

#### Scenario: Vararg parameters round-trip
- **WHEN** `lib/math.prism` exports `public fun sum(vararg xs: Int): Int = xs.size` and `app/main.prism` imports `/lib/math` and declares `fun f(): Int = sum(1, 2, 3)`
- **THEN** `prism abi lib/math.prism` shows `vararg xs: Int`, and `prism check app/main.prism` passes

#### Scenario: Making a parameter vararg changes the hash
- **WHEN** the exported `public fun sum(xs: List<Int>): Int` of `lib/math` becomes `public fun sum(vararg xs: Int): Int` while an importer calls `sum(listOf(1))`
- **THEN** the file's ABI hash changes, and the importer is checked again and rejected with `E4004`, since `listOf(1)` is not an `Int`

## Design Rationale
- **The ABI comes from the AST and the direct imports' exported type names, never from bodies.** Explicit types on exported declarations let Phase 1 run before any body is checked (`type-checker-core`). Phase 1a reads each file's exported struct and enum names from its AST alone. Phase 1b then binds each nominal name in an exported signature to the file's own declarations or to the one direct import exporting it, reading only those 1a outputs, so import cycles still work and no body is read (`typechecker-soundness`, D4).
- **ABI types are structured trees, not strings** (`typechecker-soundness`, D4). Importers once re-parsed `Display` strings, which split higher-order types wrongly and resolved names in the importer's scope. A tree recorded once is exact, hashing it structurally (a tag byte per node) covers structure rather than formatting, and each bound nominal type records whether it is a struct or an enum, so an importer can type a value of a type whose module it does not import. A balanced-parenthesis string parser was rejected: two parsers can drift, and names would still bind in the wrong scope. The module path is part of the hash, because it is part of every exported type's identity.
- **The hash ignores spans and formatting.** Source positions, comments and whitespace are excluded, and since `instance-cache` the ABI hash covers no body at all, generic ones included, so reformatting never invalidates downstream files. Only the Monomorphization Hash of a generic record covers positions, which regenerates instances, never importers (below).
- **Block-bodied exports default to `Unit` here.** ABI extraction only rejects expression-bodied exports without a return type. The type checker rejects both forms (`E4011`). Whether to tighten the ABI rule or relax the specs is an open item in `ROADMAP.md` (`resolve-nominal-type-annotations`).
- **The hash algorithm is BLAKE3 at 256 bits** (`abi-cache`, D1). See the `abi-cache` decisions below.
- **Markers on declarations and on every reference** (`value-semantics`, decided by the user). Each exported struct and enum records its marker (`Copy`, `Clone` or none), and so does each nominal reference in an exported signature, so an importer knows how values of a type may be duplicated without importing its module. Phase 1a reads markers from the AST alone, so no body is read and import cycles still work. Changing a type's marker therefore changes the ABI hash of every file whose exported signatures mention it, which is correct, because what importers may do with those values changes. Requiring the import (`E4014`) whenever Copy-ness is needed was rejected because an import would change whether unrelated code compiles, and treating an unknown marker as "none" because it gives false errors.
- **The built-in `Range` has its own ABI node** (`value-semantics`). It was recorded as an unbound name and came back to importers as an opaque type, which would have made an imported `Range` field move-only. A user-declared or imported `Range` still binds first.
- **An extension is a function record with a receiver** (`extension-methods`). `ExportedSymbol::Function` carries an optional receiver mode and type, the type bound like any signature type, and the hash covers a presence byte, the mode and the type. The sort key adds the receiver's module-qualified type text after the name, so `describe`, `User.describe` and `Team.describe` hash the same in any order. Struct and enum records never depend on extensions, so an extension in another file never changes the receiver type's file hash. Rejected: a separate `Extension` symbol kind, which would duplicate the parameter and generic-body handling, and attaching extensions to their receiver's record, which would make every extension change the receiver's ABI. Elision rule 2 needs nothing more, since it reads the receiver mode.
- **Phase 2 turns each file into a typed tree from its AST and its direct imports' ABIs** (`typed-tree`). The typed tree is the first stage of the semantic pipeline (then MIR, then borrow checking), and it reads no other file's bodies or tree, so editing a dependency's body never changes a file's tree. `prism check` still checks files in dependency order and stops at the first file with errors, while `prism dump-typed-tree` checks only its entry, which is what the scenario with an erroneous dependency shows.
- **Phase 2 lowers each file that typechecks to MIR from its typed tree alone** (`mir-lowering`). The tree carries the file's type table, and calls name imported callees and default generators by declaration path, so lowering reads no other file and a dependency's bodies cannot change a file's MIR. `prism check` lowers every file that typechecks right after checking it and discards the MIR, so lowering is exercised by every checked program and the borrow checker only has to be added to the pipeline; `prism dump-mir`, like `prism dump-typed-tree`, checks only its entry. What the MIR holds is its own capability (`mir`), which the borrow checker, drop elaboration, closure conversion and the backend will extend; the pipeline step is recorded here, next to the typed tree's.
- **Phase 2 borrow-checks each file's MIR from the MIR alone** (`borrowck-moves`, user for the crate). The type table travels in the MIR and every operand already says whether it moves, so the checker (`prism-borrowck`) reads no other file's bodies, MIR or ABI, which keeps the single-file compilation units and deterministic hermetic builds invariants; returned-borrow checks will read callee signatures from each call's parameter types. `prism check` checks and elaborates every file it lowers and stops at the first file with borrow errors, as for type errors; plain `prism dump-mir` prints the MIR as lowered whatever its borrow errors, which is what debugging the checker needs, and `--elaborated` prints the result. One entry point returns the elaborated MIR or the errors, so the move data and dataflow are computed once; the checker lives outside `prism-mir`, which would otherwise grow every borrow analysis.

Decisions from `generics` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Type parameters are positional in the ABI.** Each exported generic symbol records its type parameters with their bounds, and a type parameter in a signature is `AbiType::Parameter { index }`, lowered before binding names to the file's or imports' types, so `T` never binds to a `struct T`. The hash covers the count, the bounds, the positions and the marker parameters, but not the names: type arguments are positional, so a rename changes nothing an importer can do (a generic body's Monomorphization Hash still changes, since its tokens do). Rejected: hashing names, and binding parameters by name in the importer.
- **Marker parameters travel with every nominal reference.** `NominalHeader` and `AbiType::Nominal` record which parameters condition the type's marker, computed by one function over the AST that the type checker also uses, so an importer decides whether `Box<String>` is `Copy` without importing `Box`'s module and the two cannot drift.

Decisions from `abi-cache` (planned autonomously on 2026-10-17, reviewed on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **BLAKE3, 256 bits, everywhere** (D1). The ABI and Monomorphization Hashes and every cache key are full 256-bit BLAKE3 hashes, printed as 64 hex digits. `DeterministicHasher` kept its stream of tags, lengths and `0xFF`-terminated strings, so every equality the specs state survived the switch. Once the ABI hash keys a cache, a collision silently reuses a verdict computed against another interface, and FNV-1a has no collision resistance at all. BLAKE3 is cryptographic, faster than byte-wise FNV-1a on all but tiny inputs, and what the original design named. Rejected: FNV-1a; SHA-256 (slower, nothing gained); BLAKE3 truncated to 128 bits.
- **An interface hash keys an importer's Phase 2** (D2; superseded by `instance-cache` D3, which made it the only ABI hash). It is the ABI hash's walk with the Monomorphization Hashes left out, and the ABI hash is `H(interface hash, each symbol's body hash or its absence)`, so it changes exactly when it used to. Phase 2 reads signatures only, never a generic body, so a verdict holds for every body with the same interface and a generic body edit re-checks no importer; the deterministic hermetic builds invariant holds because the verdict depends on less than the import ABI hashes, never more. Rejected: keying by ABI hashes (re-checks every importer on a generic body edit). Deferred: per-symbol fingerprints, re-checking an importer only when a symbol it uses changes, as Kotlin's and Zig's incremental compilers do.
- **The check key** (D5). It covers the cache format version, the compiler's identity (the `build-cache` spec), the file's module path, its exact source text as the resolver parsed it, and its direct imports' interface hashes in import order (their ABI hashes since `instance-cache`). The package follows from the module path, and nothing in Phase 2 depends on the target, so the target joined keys with the first object-code entry (`instance-cache` D8). Phase 1 still runs for every file, since the resolver parses every file to build the import graph.
- **Instance records stand in for re-monomorphization** (D9, D12; replaced by `instance-cache`, whose planning, instance keys and cached code are below). After a file passes, its MIR is scanned for calls to generics of its direct imports whose type arguments mention no type parameter; each becomes an instance request stored in the check entry, so a reused file keeps its requests. An instance is keyed by the generic's identity, its signature hash (its own part of the interface hash stream), its Monomorphization Hash and a hash of its module-qualified type arguments. After a generic body changes, importers are reused and exactly the requests naming that generic miss, which is "re-monomorphization only where a generic body changed" at the granularity code generation needs. The signature joined the key in review: the body hash alone reused `drop<Int>` after `drop(x: T)` became `drop(x: &T)`. Polymorphic calls are instantiated with their caller, calls to the file's own generics are re-checked with the file, and default value generators are already part of the interface hash, so none is recorded. `monomorphization` compiles instances from generic records without these records, which keep these keys and their gaps (the Monomorphization Hash ignores the private helpers a body calls, and the type-arguments hash ignores the layout and markers of the types it names) until `instance-cache` replaces them (below). Rejected: recording nothing until `monomorphization` (the scenario would stay untested); storing the body hash seen at check time in the check entry (its value would depend on history, not on its key).

Decisions from `codegen-core` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Phase 3 is per file** (D3 and D20). Each file's code is generated from its own elaborated MIR, which names other files' functions only by symbol, so its code depends on the same inputs as its check (the file, its direct imports' ABIs, the compiler, and now the target) and can be cached per file, keyed like a check entry plus the target, which `instance-cache` does (D8 there).

Decisions from `codegen-aggregates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **The ABI carries the definitions of reachable types** (D2, settling `codegen-core`'s D24). A file's ABI records, besides its exported symbols, the definition of every struct and enum a value reached through its exports can hold, transitively, private types and types of modules the importer does not import included (never the built-in `Range`), sorted by module and name and covered by the ABI hash. A layout change therefore changes the ABI hash of every file whose exports reach the type, and the build cache's existing keys re-check exactly the importers of those files, while a file's code still depends only on its direct imports' ABIs (the deterministic hermetic builds invariant). Rejected: per-program layout at link time (code would depend on files it does not import), resilient layouts queried at run time (not zero-cost), only a size and alignment per type (an importer reading a field of an imported struct needs the whole shape, and the ABI would depend on the target), layout functions called through the function table.
- **Phase 1a reads every declared type; Phase 1b walks the closure** (D3). Each file's definitions, private ones included, are read from its AST and bound against its own declarations and its direct imports' headers, so all files' definitions are read in any order, import cycles included; `FileAbi::extract` then walks the closure over every file's definitions. Records keep field and variant names, so an importer's layout dump stays readable, at the cost of rebuilding importers when a reached private field is renamed. Rejected: anonymous records, computing the closure from the direct imports' ABIs in dependency order (a fixpoint for cycles).
- **Reachable definitions serve layout only** (D4, tightened in review). The type checker inserts them into its type table so `MirFile::types` holds every type a body can hold, but defines no name, so a private type stays unnameable; members of a type from a module the file does not import are `E4014` as before, and the fields and variants of any other type that is neither the file's own, built in, nor exported by an import stay unknown (`E4006`, `E4001`), as they were before the ABI carried them. This keeps the type system's rule that using a type's members needs its module imported, and adds no visibility rule of its own.

Decisions from `monomorphization` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **What an exporter ships: a generic record per generic** (D2). For every generic function and extension of a file that passes Phase 2, private ones included, `prism_mir::GenericRecord` holds the elaborated polymorphic MIR of its body, its default generators and its closures, which is self-describing since calls record their callees' paths, declared parameter types and type arguments, and the definitions of every struct and enum those bodies mention, closed over fields and payloads (`TypeTable::restricted_to`, a subset of the file's table, private types and those of modules an importer does not import included). It holds no other body: a non-generic helper, private or not, is called by symbol through its slot, so an instance depends on its signature, never its body, and another generic is another instance, built from its own record. A private generic therefore has a record, an "instantiable" symbol no importer can name; visibility stays a type-checking rule, and Tier 2 must export these symbols from object files instead of giving them local linkage. Why MIR: the typed tree or the source would make the importer check or lower the body again, against the generic's own imports' ABIs, which it may not have. Why per generic: the record is what `instance-cache` hashes, so an edit to one generic, or to a type only another generic uses, must not touch it. Records are built in memory from the `MirFile`s `prism run` holds. Rejected: Swift's `@inlinable` rule (an exported body may use only exported declarations, which widens APIs for an implementation detail); shipping the source or typed tree; shipping the whole file's MIR (an instance would depend on unrelated bodies).
- **An instance depends on its record and its arguments' definitions alone** (D3, D5). So it is the same whichever file requests it first, and a file's own code holds no instance and depends on no generic body: a generic body edit changes only that generic's instances (`code-generation`). Two tables never disagree about a type, since a `NominalId` is unique in a program and every definition comes from the same ABIs.
- **Direction for `instance-cache`** (D13; done in `instance-cache`, below). The Monomorphization Hash becomes a hash of the generic record, computed after Phase 2: it then covers the private types a body uses and every callee's signature, which the AST hash misses, but not non-generic helpers' bodies or other generics' records, and it covers source positions, since an instance reports runtime errors at them (positions relative to the declaration can come later). The Phase 1 body hash then leaves the ABI, so a file's ABI hash and interface hash become one. An instance key adds a structural hash of the definitions, with markers, its type arguments reach, so `keep<Thing>` is regenerated when `Thing` gains a field. Records go in the defining file's check entry, so `prism check` can collect instances over the program and find `E6004` per import cycle. Rejected: keeping the Phase 1 AST hash as the instance key (wrong in both cases above); hashing the whole defining file (every edit regenerates all its instances).

Decisions from `escaping-closures` (planned autonomously on 2026-10-18, revised for compile speed on 2026-10-19, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`escaping` flags are part of the ABI and its hash** (D3). Every exported function's parameter (now an `AbiParameter` record) and every function type's parameter hashes one flag byte, so marking a parameter `escaping` re-checks importers, whose calls are checked differently, while flags come from declarations only and a body edit cannot change one. All hashes changed once when the flags were introduced, which the compiler-version key of the build cache covers.

Decisions from `instance-cache` (planned autonomously on 2026-10-17, implemented and reviewed on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **The Monomorphization Hash is the hash of the record's canonical encoding** (D2, from `monomorphization` D13). `BLAKE3(encode(record))` for every generic of a file that passes Phase 2, private ones included, so the hash covers exactly what is stored and decoded and no second walk can drift from the codec. It covers the private types the body holds (the record's restricted table), every callee's signature (each call's parameter, destination and type-argument types, so a helper's parameter becoming `&Int` regenerates the instances), the generic's own signature (its locals) and every span with its byte offsets, since an instance reports runtime errors, and `E6001` underlines source, at them. So any edit before a generic that moves its source regenerates its instances, and the instances of every generic declared after an edited one in the same file, though it re-checks no importer and compiles no other file's unit; positions relative to the declaration remain the follow-up. No other body is covered: a non-generic helper is called through its slot, so its body edit changes only its file's unit. Type-parameter names are encoded, because a decoded record must equal the original. Rejected: the Phase 1 AST hash (misses private types and callee signatures), hashing the defining file (every edit regenerates every instance), excluding positions (stale lines in runtime errors).
- **One ABI hash** (D3). With no body hash left in the ABI, the interface hash and the ABI hash are one value, named "ABI hash" as the deterministic hermetic builds invariant names it; check keys use the direct imports' ABI hashes, and `prism abi` prints only `ABI Hash:`.
- **What an instance key covers** (D6, answering `abi-cache` D9). The format, the compiler and target identities, the instance's symbol (the generic and its module-qualified type arguments), its generic's Monomorphization Hash and a hash of the definitions, markers and marker parameters included, of every struct and enum the type arguments reach through fields and payloads over every module, sorted by module and name. The definitions come from Phase 1a, which every run computes for every file, and each nominal's hash is memoized per run. It needs neither the generic's signature (the record holds it), nor private helpers' bodies (called by symbol), nor other generics' records (other instances), nor the requester (one instance serves every caller).
- **Instances are planned per program from summaries, in `prism check` too** (D7). `plan_instances` runs Tarjan's components over every summary's generic calls (`E6004`) and then the worklist over summaries, substituting each distinct instance's calls once; `build_instance_unit` instantiates a record only for an instance whose code misses. A reused file's summary comes from its check entry, so `prism check` lists every instance of the program, transitively and through modules a caller does not import, and finds `E6004` across an import cycle from reused verdicts. A test compares planning with instantiating bodies over the fixture corpus. Cost: linear in the summaries' generic calls plus the distinct instances, with no MIR read.
- **A program-wide type table for instance units** (D13). A missed instance's requester may be cached, with no MIR in memory, so instance units take their type arguments' definitions from the program's Phase 1a definitions, converted once per run as an importer converts ABI definitions (`type_definition_from_abi`) and built only when an instance misses. Every `NominalId` has one definition per program, so this agrees with every file's table, which a library test checks.

Decisions from `receiver-lambdas` (planned autonomously on 2026-10-19 for `ui-toolkit`'s Compose-like layout scopes, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Receivers and contexts are part of the ABI** (D9). A structured function type records whether it has a receiver, and an extension's entry its context type, bound like any signature type; the hash covers both, so adding a context parameter or a receiver changes an importer's view, while the context parameter's name, which the ABI does not record, does not. Exported symbols are ordered by their context after receiver and name, so reordering declarations never changes the hash.

Decisions from `compose-transform` (planned autonomously on 2026-10-18, reconciled with `compose-runtime`, `receiver-lambdas` and `compose-hot-reload` and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **Intrinsic structs in the ABI** (D16): an exported intrinsic struct is an ABI entry with its name, type parameters and markers, so changing a marker changes the hash, but not its layout, which comes from the compiler whose version every artifact key holds.

Decisions from `ui-toolkit` (planned on 2026-10-18 and revised on 2026-10-19 by user direction to follow Jetpack Compose's APIs and modules, implemented and reviewed on 2026-10-19, and logged in `docs/decisions-for-review.md`):

- **A struct and its companion value are two symbols of one name** (D4). `FileAbi::symbols` already held a list, so both are exported, sorted by kind before name, and the order of the two declarations does not change the hash; an import brings both.
- **Default inputs ride on the default's hash** (D26, review). Each parameter with a default also records which of the context, the receiver and the earlier parameters its default may read, computed from the default's syntax in Phase 1, so a caller in another file borrows exactly what the generator takes (`mir`). It follows from the syntax the ABI hash already covers, so it is not hashed again, and editing a default still recompiles exactly the files that a changed default recompiled before.

Decisions from `explicit-library` (review item A4, agreed with the user on 2026-10-20; the mechanism decided while proposing and implementing, and logged in `docs/decisions-for-review.md`):

- **Member modules replace the prelude modules as implicit inputs** (D2, D14). With no prelude, a workspace file's Phase 2 sees its direct imports and, after them, only the members of the built-in types that `/prism/core` and `/prism/collections` declare (since `core-library-in-prism` D3, below, `/prism/core` alone), through one `MemberScope` per graph shared by every checker (one `Arc` copy per file and one lookup per member the file's own and imported members miss). The two member modules are still parsed for every program and counted by check keys, after the direct imports; `/prism/compose/runtime` is an input only of the files that import it. Instances of library generics are planned from imports, as any import's. Library files see neither. Rejected: members typed from the catalog with no module loaded, and a per-file member scope from imports only (`s.length` would need an import).

Decisions from `operator-conventions` (planned autonomously on 2026-10-20 for part L1 of the review's agreed item A4, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`; the conventions themselves are in `operators`):

- **Operator flags and equality operators are in the ABI** (D15). `ExportedSymbol::Function::is_operator` and an `equality` flag on struct and enum symbols and reachable definitions are covered by the ABI hash and the structural hash of definitions, so marking a function `operator` or adding `equals` re-checks importers and changes instance keys reaching the type, while editing an operator's body changes nothing downstream. An importer reads the flag from both the exported symbols and the reachable definitions (review fix).

Decisions from `drop-and-clone-hooks` (part L2 of the review's agreed item A4, planned autonomously on 2026-10-20, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`; the user decided in A4 that there is no `unsafe` layer for now, so hooks serve types wrapping runtime handles, not raw memory):

- **Hook flags are part of a type's definition** (D10): `drop_hook` and `clone_hook` on exported struct and enum records and on reachable definitions, set in Phase 1 by a syntactic match of the file's own hook declarations (`prism_abi::hook_types`, also used by the type checker), covered by the ABI hash and by the structural hash of definitions that keys instances. Adding or removing a hook re-checks importers; editing a hook's body changes no hash. An importer's `TypeTable` takes the flags from both the exported symbols and the reachable definitions.
- **Hook instances of generic types are planned from summaries** (D11): Phase 2 walks the local types of every compiled body and the type arguments of its calls through fields, payloads, nullables, boxes and lists (`HookWalk` in `prism-mir`'s `summary.rs`, each distinct type once per file, nothing when the file's table has no generic type with a hook), and requests `<Type>.drop<Args>` for each generic type with a drop hook it reaches and `<Type>.clone<Args>` for each one `Clone` at its arguments: as roots for non-generic bodies, and with type parameters left as they are for generics (once for the function and its closures, once per default), which planning substitutes. A type an instance drops is built from types its record names or from its type arguments, so every hook instance is found; a type argument an instance only borrows still requests its hook (one instance compiled, never a missed one). Requests are ordinary `InstanceRequest`s, so summaries and the cache need no new field, and they join the cross-file polymorphic recursion check (`E6004`, also within one file, with a note when the growing request is a hook's). Compile-time cost: linear in the distinct types walked and in requests (shape `generic-hook-instances`). Rejected: walking types in the planner (it has no definitions), requesting instances while generating glue (instance keys and `--explain-cache` would be decided after planning), deferring generic hooks (`library-in-prism`'s state needs them).

Decisions from `vararg` (part L4 of the review's agreed item A4, user direction 2026-10-20; planned autonomously on 2026-10-20, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`; the rules themselves are in `vararg-parameters`):

- **Vararg flags in the ABI** (D14). An exported parameter records `is_vararg` with its type as the `List<E>` the body sees, so importers build the function type unchanged and know to pack. The ABI hash covers the flag (the parameter-kind byte's value 3, so existing hashes keep their bytes), since it changes what callers may write; the machine signature does not change, so slot contracts and hot reload are unaffected.

Decisions from `core-library-in-prism` (the core half of the review's agreed item A4, user direction 2026-10-20 for writing the library in Prism over the smallest intrinsic core with no `unsafe`; planned autonomously on 2026-10-20, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Extension properties in the ABI** (D12). Every `public` and `package` extension property is exported with its kind (plain or intrinsic) and type, covered by the ABI hash; a getter's body is not in the ABI, so editing it recompiles no importer, and changing its type re-checks them. `prism abi` prints `public val Int.dp: Dp`. Ingestion makes a plain property an imported extension, or a member when `/prism/core` declares it on a built-in type. Cost: one flag per exported property in Phase 1.
- **One member module, and instances of generic members** (D3, D4). A workspace file's key appends `/prism/core`'s ABI hash alone, so a change of `/prism/collections` reaches only its importers. Generic members written in Prism (`isEmpty`, `isNotEmpty`, `indices`) are called by files that never import `/prism/core`; their instances are planned from file summaries and cached like an import's, one per element type and program, shared by every workspace with one compiler and cache, and invalidated with the rest of the cache by the compiler identity, which hashes the executable embedding the library.
