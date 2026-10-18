# Prism Compilation Pipeline & ABI Extraction

## Purpose
The compilation and ABI specification defines the two-phase pipeline as implemented: Phase 1 (`crates/prism-abi`) extracts each file's exported declaration surface and computes deterministic ABI and interface hashes that ignore private symbols and non-generic function bodies, Phase 2 checks each file against its direct imports' ABIs, and `prism check` skips Phase 2 for files whose inputs the build cache (the `build-cache` spec) already checked. Code generation, instantiation, cached `.abi` entries and the execution tiers are planned and described in [`docs/architecture.md`](../../../docs/architecture.md).

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
The compiler MUST compute a deterministic 256-bit BLAKE3 hash of each file's exported ABI that covers exported signatures, enum variant shapes, default parameter values and the definitions of reachable types, and excludes every other private declaration, non-generic function bodies and source positions.

#### Scenario: Modifying non-generic function body
- **WHEN** a developer modifies the internal implementation of a public function without changing its signature
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
- **THEN** each ABI hash is printed as `0x` followed by 64 lowercase hexadecimal digits

### Requirement: Monomorphization Hash for Generics
Exported generic functions, including generic extension functions, MUST record a Monomorphization Hash of their body in their ABI signature, computed independently of source positions, comments and whitespace.

#### Scenario: Changing generic function body
- **WHEN** the body of an exported generic function changes
- **THEN** its Monomorphization Hash and the file's ABI hash change

#### Scenario: Generic body hash ignores formatting
- **WHEN** comments or blank lines are added to an exported generic function without changing its code
- **THEN** its Monomorphization Hash and the file's ABI hash remain unchanged

#### Scenario: Changing a generic extension body
- **WHEN** the body of the exported extension `public fun <T> Box<T>.get(): &T` changes
- **THEN** its Monomorphization Hash and the file's ABI hash change

#### Scenario: Changing a non-generic extension of a generic type
- **WHEN** the body of the exported extension `public fun Box<Int>.sum(): Int` changes
- **THEN** it records no Monomorphization Hash and the file's ABI hash is unchanged

### Requirement: Interface Hash
The compiler MUST compute, for each file, an interface hash: a deterministic 256-bit BLAKE3 hash of its exported ABI with the Monomorphization Hashes of its generic bodies left out. It MUST cover everything else the ABI hash covers, and the ABI hash MUST be a function of the interface hash and the Monomorphization Hashes.

#### Scenario: Changing a generic body keeps the interface hash
- **WHEN** the body of the exported generic function `public fun <T> keep(x: T): T` changes
- **THEN** the file's ABI hash changes and its interface hash is unchanged

#### Scenario: Changing a signature changes the interface hash
- **WHEN** a parameter type of an exported function changes, or an exported struct gains a field
- **THEN** the file's interface hash and its ABI hash both change

#### Scenario: Interface hash printed by `prism abi`
- **WHEN** a developer runs `prism abi` on a source file
- **THEN** it prints the file's interface hash after its ABI hash, in the same format

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

### Requirement: Compilation Avoidance via Interface Hashes
`prism check` MUST skip Phase 2 (type checking, MIR lowering and borrow checking) for a file whose content, module path and direct imports' interface hashes are those of an earlier successful check by the same compiler, recorded in the build cache (the `build-cache` spec). It MUST run Phase 2 for every other file. The report and the exit status MUST be the same either way.

#### Scenario: Modifying non-generic function body
- **WHEN** `lib/text.prism` exports `public fun shout(s: &String): String`, `app/main.prism` imports `/lib/text`, both have been checked, and the body of `shout` changes
- **THEN** the next `prism check` re-checks `lib/text` only, and reuses `app/main`: zero downstream files that import it are recompiled

#### Scenario: Modifying public function signature
- **WHEN** in that workspace `shout` gains a parameter with a default value, `n: Int = 1`, and `app/main`, whose exported signatures do not mention `lib/text`, is itself imported by `app/cli`
- **THEN** the next `prism check` re-checks `lib/text` and its direct importer `app/main`, and reuses `app/cli`, whose direct import's interface hash is unchanged

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
A change to a struct or enum MUST re-check every file whose direct import's ABI records that type, as a symbol or as a reachable definition, through the existing interface-hash keys of the build cache, and no other file because of it.

#### Scenario: A field added to a transitively reached type
- **WHEN** `geo/core.prism` also exports `public fun origin(): Vec = Vec(0)`, `geo/shapes.prism` exports `public fun unit(): Circle = Circle(origin(), 1)`, `app/main.prism` imports only `/geo/shapes` and prints `unit().r`, all three have been checked, and then only `geo/core.prism` changes: `Vec` gains a field `val y: Int` and `origin` returns `Vec(0, 0)`
- **THEN** the next `prism check --explain-cache app/main.prism` prints `  checked geo/core.prism`, `  checked geo/shapes.prism` and `  checked app/main.prism`, and `prism run app/main.prism` prints `1` before and after the change

#### Scenario: A body change in the defining file
- **WHEN** in that workspace only the body of a function of `geo/core` changes
- **THEN** the next `prism check --explain-cache app/main.prism` checks `geo/core` again and reuses `geo/shapes` and `app/main`

### Requirement: Instantiation Records for Generic Body Changes
When a file that passes Phase 2 calls an exported generic function or extension of a direct import with type arguments that mention no type parameter of the caller, `prism check` MUST record an instance for that generic, its signature, its Monomorphization Hash and those type arguments. An instance already recorded MUST be reported as reused, any other as one to instantiate.

#### Scenario: Changing generic function body
- **WHEN** `lib/box.prism` exports `public fun <T> keep(x: T): T` and `public fun <T> drop(x: T): Int`, `app/a.prism` calls `keep(1)`, `app/b.prism` calls `drop("s")`, the workspace has been checked, and the body of `keep` changes
- **THEN** the next `prism check` re-checks `lib/box` only, reports that `app/a` must instantiate `lib/box::keep<Int>` again, and reports the instance `lib/box::drop<String>` of `app/b` as reused: re-monomorphization is triggered only in downstream files that instantiate that generic

#### Scenario: Changing a generic signature instantiates again
- **WHEN** `app/a.prism` calls `drop(1)` of that `lib/box`, the workspace has been checked, then the parameter of `drop` becomes `x: &T` with its body unchanged and the call becomes `drop(&1)`
- **THEN** the next `prism check` re-checks `lib/box` and `app/a`, and reports that `app/a` must instantiate `lib/box::drop<Int>` again, since an instance depends on the generic's signature as well as on its body

#### Scenario: Polymorphic calls are not recorded
- **WHEN** `app/a.prism` declares `fun <T> wrap(x: T): T = keep(x)`, calling the imported generic `keep` with its own type parameter
- **THEN** no instance is recorded for that call, since its type arguments are known only when `wrap` is instantiated

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
Phase 1 MUST record whether each parameter of an exported function, and each parameter of every function type in an exported signature, is `escaping`, and the ABI and interface hashes MUST cover it, so an importer obtains the flags the exporter declared and is re-checked when one changes.

#### Scenario: Escaping parameters round-trip
- **WHEN** `lib/ui.prism` exports `public fun button(onClick: escaping () -> Unit, register: (escaping () -> Unit) -> Unit): Int` and `app/main.prism` imports `/lib/ui` and declares `fun greet(s: &String) {}` and `fun f(s: String): Int = button({ greet(s) }) { h -> h() }`
- **THEN** `prism abi lib/ui.prism` shows `onClick: escaping () -> Unit` and `register: (escaping () -> Unit) -> Unit`, and `prism check app/main.prism` passes, the first closure owning `s`

#### Scenario: Marking a parameter escaping changes the hashes
- **WHEN** an exported `public fun store(f: () -> Unit): Int` becomes `public fun store(f: escaping () -> Unit): Int`
- **THEN** the file's ABI hash and interface hash both change, and an importing file is checked again

## Design Rationale
- **The ABI comes from the AST and the direct imports' exported type names, never from bodies.** Explicit types on exported declarations let Phase 1 run before any body is checked (`type-checker-core`). Phase 1a reads each file's exported struct and enum names from its AST alone. Phase 1b then binds each nominal name in an exported signature to the file's own declarations or to the one direct import exporting it, reading only those 1a outputs, so import cycles still work and no body is read (`typechecker-soundness`, D4).
- **ABI types are structured trees, not strings** (`typechecker-soundness`, D4). Importers once re-parsed `Display` strings, which split higher-order types wrongly and resolved names in the importer's scope. A tree recorded once is exact, hashing it structurally (a tag byte per node) covers structure rather than formatting, and each bound nominal type records whether it is a struct or an enum, so an importer can type a value of a type whose module it does not import. A balanced-parenthesis string parser was rejected: two parsers can drift, and names would still bind in the wrong scope. The module path is part of the hash, because it is part of every exported type's identity.
- **The hash ignores spans and formatting.** Source positions, comments and whitespace are excluded, including from a generic function's body hash, so reformatting never invalidates downstream files.
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

- **BLAKE3, 256 bits, everywhere** (D1). The ABI, interface and Monomorphization Hashes and every cache key are full 256-bit BLAKE3 hashes, printed as 64 hex digits. `DeterministicHasher` kept its stream of tags, lengths and `0xFF`-terminated strings, so every equality the specs state survived the switch. Once the ABI hash keys a cache, a collision silently reuses a verdict computed against another interface, and FNV-1a has no collision resistance at all. BLAKE3 is cryptographic, faster than byte-wise FNV-1a on all but tiny inputs, and what the original design named. Rejected: FNV-1a; SHA-256 (slower, nothing gained); BLAKE3 truncated to 128 bits.
- **An interface hash keys an importer's Phase 2** (D2). It is the ABI hash's walk with the Monomorphization Hashes left out, and the ABI hash is `H(interface hash, each symbol's body hash or its absence)`, so it changes exactly when it used to. Phase 2 reads signatures only, never a generic body, so a verdict holds for every body with the same interface and a generic body edit re-checks no importer; the deterministic hermetic builds invariant holds because the verdict depends on less than the import ABI hashes, never more. Rejected: keying by ABI hashes (re-checks every importer on a generic body edit). Deferred: per-symbol fingerprints, re-checking an importer only when a symbol it uses changes, as Kotlin's and Zig's incremental compilers do.
- **The check key** (D5). It covers the cache format version, the compiler's identity (the `build-cache` spec), the file's module path, its exact source text as the resolver parsed it, and its direct imports' interface hashes in import order. The package follows from the module path, and nothing in Phase 2 depends on the target, so the target triple joins keys with the first object-code entry. Phase 1 still runs for every file, since the resolver parses every file to build the import graph.
- **Instance records stand in for re-monomorphization** (D9, D12). After a file passes, its MIR is scanned for calls to generics of its direct imports whose type arguments mention no type parameter; each becomes an instance request stored in the check entry, so a reused file keeps its requests. An instance is keyed by the generic's identity, its signature hash (its own part of the interface hash stream), its Monomorphization Hash and a hash of its module-qualified type arguments. After a generic body changes, importers are reused and exactly the requests naming that generic miss, which is "re-monomorphization only where a generic body changed" at the granularity code generation needs. The signature joined the key in review: the body hash alone reused `drop<Int>` after `drop(x: T)` became `drop(x: &T)`. Polymorphic calls are instantiated with their caller, calls to the file's own generics are re-checked with the file, and default value generators are already part of the interface hash, so none is recorded. `monomorphization` compiles instances from generic records without these records, which keep these keys and their gaps (the Monomorphization Hash ignores the private helpers a body calls, and the type-arguments hash ignores the layout and markers of the types it names) until `instance-cache` replaces them (below). Rejected: recording nothing until `monomorphization` (the scenario would stay untested); storing the body hash seen at check time in the check entry (its value would depend on history, not on its key).

Decisions from `codegen-core` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **Phase 3 is per file** (D3 and D20). Each file's code is generated from its own elaborated MIR, which names other files' functions only by symbol, so its code depends on the same inputs as its check (the file, its direct imports' ABIs, the compiler, and now the target) and can be cached per file later, keyed like a check entry plus the target triple. `prism run` does not use the build cache yet: a cached verdict carries no MIR.

Decisions from `codegen-aggregates` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **The ABI carries the definitions of reachable types** (D2, settling `codegen-core`'s D24). A file's ABI records, besides its exported symbols, the definition of every struct and enum a value reached through its exports can hold, transitively, private types and types of modules the importer does not import included (never the built-in `Range`), sorted by module and name and covered by the interface hash. A layout change therefore changes the interface hash of every file whose exports reach the type, and the build cache's existing keys re-check exactly the importers of those files, while a file's code still depends only on its direct imports' ABIs (the deterministic hermetic builds invariant). Rejected: per-program layout at link time (code would depend on files it does not import), resilient layouts queried at run time (not zero-cost), only a size and alignment per type (an importer reading a field of an imported struct needs the whole shape, and the ABI would depend on the target), layout functions called through the function table.
- **Phase 1a reads every declared type; Phase 1b walks the closure** (D3). Each file's definitions, private ones included, are read from its AST and bound against its own declarations and its direct imports' headers, so all files' definitions are read in any order, import cycles included; `FileAbi::extract` then walks the closure over every file's definitions. Records keep field and variant names, so an importer's layout dump stays readable, at the cost of rebuilding importers when a reached private field is renamed. Rejected: anonymous records, computing the closure from the direct imports' ABIs in dependency order (a fixpoint for cycles).
- **Reachable definitions serve layout only** (D4, tightened in review). The type checker inserts them into its type table so `MirFile::types` holds every type a body can hold, but defines no name, so a private type stays unnameable; members of a type from a module the file does not import are `E4014` as before, and the fields and variants of any other type that is neither the file's own, built in, nor exported by an import stay unknown (`E4006`, `E4001`), as they were before the ABI carried them. This keeps the type system's rule that using a type's members needs its module imported, and adds no visibility rule of its own.

Decisions from `monomorphization` (planned autonomously on 2026-10-17, refined while implementing and reviewing the change on 2026-10-18, and logged in `docs/decisions-for-review.md`):

- **What an exporter ships: a generic record per generic** (D2). For every generic function and extension of a file that passes Phase 2, private ones included, `prism_mir::GenericRecord` holds the elaborated polymorphic MIR of its body, its default generators and its closures, which is self-describing since calls record their callees' paths, declared parameter types and type arguments, and the definitions of every struct and enum those bodies mention, closed over fields and payloads (`TypeTable::restricted_to`, a subset of the file's table, private types and those of modules an importer does not import included). It holds no other body: a non-generic helper, private or not, is called by symbol through its slot, so an instance depends on its signature, never its body, and another generic is another instance, built from its own record. A private generic therefore has a record, an "instantiable" symbol no importer can name; visibility stays a type-checking rule, and Tier 2 must export these symbols from object files instead of giving them local linkage. Why MIR: the typed tree or the source would make the importer check or lower the body again, against the generic's own imports' ABIs, which it may not have. Why per generic: the record is what `instance-cache` hashes, so an edit to one generic, or to a type only another generic uses, must not touch it. Records are built in memory from the `MirFile`s `prism run` holds. Rejected: Swift's `@inlinable` rule (an exported body may use only exported declarations, which widens APIs for an implementation detail); shipping the source or typed tree; shipping the whole file's MIR (an instance would depend on unrelated bodies).
- **An instance depends on its record and its arguments' definitions alone** (D3, D5). So it is the same whichever file requests it first, and a file's own code holds no instance and depends on no generic body: a generic body edit changes only that generic's instances (`code-generation`). Two tables never disagree about a type, since a `NominalId` is unique in a program and every definition comes from the same ABIs.
- **Direction for `instance-cache`** (D13, recorded). The Monomorphization Hash becomes a hash of the generic record, computed after Phase 2: it then covers the private types a body uses and every callee's signature, which the AST hash misses, but not non-generic helpers' bodies or other generics' records, and it covers source positions, since an instance reports runtime errors at them (positions relative to the declaration can come later). The Phase 1 body hash then leaves the ABI, so a file's ABI hash and interface hash become one. An instance key adds a structural hash of the definitions, with markers, its type arguments reach, so `keep<Thing>` is regenerated when `Thing` gains a field. Records go in the defining file's check entry, so `prism check` can collect instances over the program and find `E6004` per import cycle. Rejected: keeping the Phase 1 AST hash as the instance key (wrong in both cases above); hashing the whole defining file (every edit regenerates all its instances).

Decisions from `escaping-closures` (planned autonomously on 2026-10-18, revised for compile speed on 2026-10-19, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **`escaping` flags are part of the ABI and both hashes** (D3). Every exported function's parameter (now an `AbiParameter` record) and every function type's parameter hashes one flag byte, so marking a parameter `escaping` re-checks importers, whose calls are checked differently, while flags come from declarations only and a body edit cannot change one. All hashes changed once when the flags were introduced, which the compiler-version key of the build cache covers.
