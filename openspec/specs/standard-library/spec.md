# Standard Library Specification

## Purpose
The standard library is the Prism source the compiler ships under the reserved root package `prism`: which modules exist, which of their names every file sees without an import, the `external` and `intrinsic` declaration forms that only the library may use, and how the library is checked.

## Requirements

### Requirement: The Compiler Ships the Library
The `prism` executable MUST contain the source of every library module and MUST NOT read library files from disk. The modules MUST be `/prism/core`, `/prism/collections` and `/prism/compose/runtime`, the prelude modules, and `/prism/system` and `/prism/composer` (compose, "The Composer Protocol Module"), which are imported explicitly. Each MUST be a file of the package `prism`, or of a package below it, whose module path is `prism/<name>`.

#### Scenario: A library module is imported
- **WHEN** `app/main.prism` declares `import /prism/system` and `fun main() { println(1); exitProcess(3) }`, in a workspace with no `prism` directory
- **THEN** `prism run app/main.prism` prints `1` and exits with status 3

#### Scenario: The composer module is not in the prelude
- **WHEN** `prism check --explain-cache main.prism` runs on a file without imports, and again after the file declares `import /prism/composer`
- **THEN** the first explanation names no `prism/composer.prism`, and the second names it

### Requirement: Library Modules Compile Like Workspace Files
A library module MUST go through every stage a workspace file does: Phase 1 extracts its ABI, Phase 2 checks its bodies, its generics are instantiated per program, and its code is a unit of its own, cached like a workspace file's. A program MUST parse only the library modules it reaches: the prelude modules and those imported, directly or transitively.

#### Scenario: Unreached modules are not parsed
- **WHEN** `prism check --explain-cache main.prism` runs on a file without imports
- **THEN** its file lines name `prism/core.prism`, `prism/collections.prism`, `prism/compose/runtime.prism` and `main.prism`, and no other library module

### Requirement: Prelude Names Are Found Last
The public declarations of the prelude modules MUST be in scope in every workspace file without an import. A name MUST resolve to a prelude declaration only when no local, parameter, top-level declaration or explicit import of that name is in scope, and such a declaration or import shadows it without `E4010` or `E4012`. An explicit import of a prelude module MUST be accepted, and makes its names explicit imports.

#### Scenario: A declaration shadows a prelude function
- **WHEN** `fun print(n: &Int) {}` is declared and `fun f() { print(1); println(2) }` is checked
- **THEN** the file typechecks, `print(1)` calls `main::print` and `println(2)` calls the intrinsic `println` of `/prism/core`

#### Scenario: A file's own declaration of a prelude name
- **WHEN** a file declares `fun listOf(n: Int): Int = n` and `fun f(): Int = listOf(3)`
- **THEN** the file typechecks without `E4010`, and the call is a call of `main::listOf`

#### Scenario: Prelude intrinsics are only called
- **WHEN** `fun f() { val p = println }` is checked
- **THEN** the type checker emits `E4013` for `println`, a generic function used as a value

### Requirement: Prelude Extensions Are Found After Imported Ones
An extension of a prelude module that is not a member ("Intrinsic Extensions Are Members") MUST be considered only after the file's own extensions and those of its explicit imports, so either shadows it without `E4012`.

#### Scenario: An import shadows a prelude extension
- **WHEN** `lib/util.prism` exports `public fun <T> List<T>.forEach(action: (&T) -> Unit): Unit {}`, and a file that imports `/lib/util` calls `listOf(1).forEach { n -> println(n) }`
- **THEN** the file typechecks, and the call resolves to `lib/util`'s extension, with no `E4012`

### Requirement: Library Files Import What They Use
A library file MUST NOT see the prelude implicitly: the names it uses from another library module come from an explicit import of that module.

#### Scenario: A library module imports the core
- **WHEN** a library test module that uses `println` declares no import
- **THEN** the type checker emits `E4001` for `println` in that module, and accepts it once the module declares `import core`

### Requirement: External Functions Are Runtime Entry Points
An `external fun` MUST declare, without a body, a function the runtime provides under the C symbol equal to its name. A call MUST go directly to that symbol with the platform's C calling convention, through no function-table slot. Naming an external as a value MUST give an adapter closure that calls it, as for any function.

#### Scenario: An external called from library code
- **WHEN** `/prism/system` declares `private external fun prism_rt_exit(status: Int): Nothing` and `public fun exitProcess(status: Int): Nothing = prism_rt_exit(status)`
- **THEN** the Cranelift IR of the unit of `prism/system` calls the imported function `prism_rt_exit` directly in the body of `exitProcess`, with no load from a slot, and declares it with the signature of the runtime's entry

### Requirement: Values Crossing an External Call
An external's machine signature MUST be the one a Prism function of the same signature has. `Int`, `Double`, `Bool` and `Char` parameters MUST be passed by value. Any other value, held in memory (strings, structs, enums, nullables, lists, boxes, function values), and any reference MUST be passed by address, with the ownership a Prism callee would take. A result held in memory MUST be written through an address passed first, and a `Nothing` result means the function never returns.

#### Scenario: A string result through an address
- **WHEN** a library test module declares `external fun prism_rt_int_to_string(value: Int): String` and `public fun text(n: Int): String = prism_rt_int_to_string(n)`, and its unit is compiled
- **THEN** the external is declared with an address parameter followed by a 64-bit integer and no result, which matches the runtime's entry, and `text` passes the address of its own result first

### Requirement: External Declarations Are Restricted
An external MUST NOT be `public`, generic, an extension or `compose`, MUST NOT have default values, and MUST NOT return a type holding a reference. Each such declaration MUST be rejected with `E4029` (`InvalidLibraryDeclaration`), naming the rule it breaks.

#### Scenario: Externals that cannot be declared
- **WHEN** a library test module declares `public external fun prism_rt_a(): Int`, `external fun <T> prism_rt_b(x: T)`, `external fun Int.prism_rt_c()`, `external fun prism_rt_d(n: Int = 0)` and `external fun prism_rt_e(s: &String): &Int`
- **THEN** the type checker emits `E4029` for each of the five, naming the rule each breaks

### Requirement: Intrinsic Declarations
An `intrinsic fun` or `intrinsic val` MUST declare, without a body, a function or extension property whose calls the compiler lowers itself. The compiler MUST hold a catalog of its intrinsics. A library declaration naming no intrinsic of the catalog, or whose signature differs from the catalog's (visibility, receiver and mode, type parameters and bounds, parameter names and types, result), MUST be rejected with `E4029`.

#### Scenario: An unknown intrinsic is rejected
- **WHEN** a library test module declares `public intrinsic fun Int.squared(): Int`
- **THEN** the type checker emits `E4029`, saying that the compiler provides no intrinsic `squared` on `Int`

#### Scenario: An intrinsic with another signature is rejected
- **WHEN** a library test module declares `public intrinsic fun Int.toString(radix: Int): String`
- **THEN** the type checker emits `E4029`, showing the signature the compiler provides, `fun Int.toString(): String`

### Requirement: Intrinsic Calls Are Typed by Their Signatures
A call of an intrinsic MUST be checked as a call of its declared signature, except for those the catalog types by a rule:
- `print` and `println` (`fun <T> print(value: &T)`) take an argument with a text form (type-system, "Output Built-In Arguments"), and `println` also none;
- `listOf` (`fun <T> listOf(element: T): List<T>`) takes any number of elements;
- `contains` and `indexOf` of `List<T>` also need `==` on `T`;
- `remember` also takes keys ("Remember Takes Up to Three Keys").

#### Scenario: A signature-typed intrinsic
- **WHEN** `fun f(xs: &List<Int>): Bool = xs.isEmpty(1)` is checked
- **THEN** the type checker emits `E4004`, since `isEmpty` is declared with no parameter

#### Scenario: A rule-typed intrinsic
- **WHEN** `fun f() { println(); println(1); val xs = listOf(1, 2, 3) }` is checked
- **THEN** the file typechecks, although `println` and `listOf` are declared with one parameter

### Requirement: Remember Takes Up to Three Keys
A call of `remember` (`fun <T : Copy> remember(calculation: () -> T): T`) MUST also accept up to three keys before the calculation, each typed by a type parameter of its own bounded by `Copy` and borrow-free (compose, "Remembering With Keys").

#### Scenario: Keys of remember
- **WHEN** `compose fun f(a: Int, b: Bool) { val x = remember(a, b) { 1 }; val y = remember(a, b, 'c', 1.5) { 2 } }` is checked
- **THEN** the type checker accepts the first call and reports an argument count error (`E4004`) for the second, which has four keys

### Requirement: Intrinsic Extensions Are Members
An intrinsic extension function or property MUST be a member of its receiver type wherever its module is visible. A call or read of that name on that type, through any `&`, `&mut` and `Heap<T>` layers, MUST use it before any extension, after fields and the built-in `clone()` and `unbox()`. A workspace extension of that name on that receiver type constructor could never be called, so it MUST be rejected with `E4010`.

#### Scenario: A workspace extension named like a member
- **WHEN** a file declares `fun Int.toString(): String = "x"`, `fun String.length(): Int = 0` and `fun <T> List<T>.add(element: T) {}`
- **THEN** the type checker emits `E4010` for each, saying that the member of `/prism/core` or `/prism/collections` always takes precedence

#### Scenario: A workspace extension named like a helper
- **WHEN** a file declares `fun <T> List<T>.map(f: (&T) -> Int): Int = 0` and `fun f(xs: &List<Int>): Int = xs.map { x -> x }`
- **THEN** the file typechecks, and the call resolves to the file's own extension

### Requirement: Library-Only Declarations Are Rejected in the Workspace
A workspace file declaring anything `external` or `intrinsic` MUST be rejected in Phase 1 with `E3002` (`LibraryOnlyDeclaration`), at the modifier, with help saying that only the bundled library may declare them.

#### Scenario: External declarations outside the bundled library rejected
- **WHEN** a workspace file declares `external fun f(): Int` or `public intrinsic fun g(): Int`
- **THEN** `prism check` reports `E3002` at `external`, and at `intrinsic`

### Requirement: The Core Module
`/prism/core` MUST declare, as public intrinsics, `print` and `println`, `toString()` of `Int`, `Double`, `Bool` and `Char` (`fun Int.toString(): String`, ...), and the property `val String.length: Int`. Their behavior is that of the type-system and execution requirements for them.

#### Scenario: Core names need no import
- **WHEN** `fun main() { val s = 42.toString(); println(s.length) }` runs in a file without imports
- **THEN** it prints `2`

### Requirement: The Collections Module
`/prism/collections` MUST declare, as public intrinsics, `listOf` and the members of `List<T>` (collections, "List Members Are Typed as Extensions", "Mutating Members", "Size and Indices Are Read Without Parentheses"), and, as public Prism functions, the higher-order helpers (collections, "Higher-Order Helpers").

#### Scenario: Helpers need no import
- **WHEN** `fun main() { val xs = listOf(1, 2, 3); println(xs.map { x -> x * 2 }.sumOf { x -> x }) }` runs in a file without imports
- **THEN** it prints `12`

### Requirement: Ending the Process
`/prism/system` MUST declare `public fun exitProcess(status: Int): Nothing`. A call MUST flush the program's output and end the process with exit status `status & 255`. No further code of the program MUST run, no drop MUST run, and the check of `--check-allocations` MUST NOT run, as for a runtime error. Under `prism run --watch`, it MUST end the program's run as `main` returning with that status does.

#### Scenario: Exiting with a status
- **WHEN** `import /prism/system` and `fun main() { val s = "a" + "b"; print(s); exitProcess(7); println("never") }` run with `--check-allocations`
- **THEN** stdout holds `ab`, stderr is empty, and the exit status is 7

#### Scenario: Only the low bits of the status
- **WHEN** a program calls `exitProcess(258)`
- **THEN** its exit status is 2

#### Scenario: Exiting under a watch session
- **WHEN** `prism run --watch --edits edits main.prism` runs `import /prism/system` and `fun main() { println(1); exitProcess(3); println("never") }`, with an edit script whose only step is `after exit` and changes the program to print `2` and exit with status 4
- **THEN** stdout holds `1` and `2`, each on its own line, stderr holds `watch: main returned with status 3; waiting for changes`, `watch: running main again` and `watch: main returned with status 4; waiting for changes`, and the session's exit status is 4

### Requirement: The Library Passes Its Own Checks
Every library module MUST pass `prism check` with no diagnostic and `prism fmt --check`, and every external MUST match the runtime's function table. The prelude modules MUST export pairwise distinct names. The modules the executable contains MUST be exactly the `.prism` files of the library's source directory.

#### Scenario: The library is checked by the compiler's tests
- **WHEN** `cargo test -p prism-cli --test library_tests` runs
- **THEN** it checks and formats every embedded module, compares every external with the runtime table, compares the prelude modules' exported names, and compares the embedded module list with the files of `crates/prism-library/prism/`, failing on any difference

### Requirement: Intrinsic Structs
An `intrinsic struct` MUST declare a type whose representation, glue and operations the compiler provides from its catalog, with the markers the catalog gives it. A library declaration naming no intrinsic struct of the catalog, or with other type parameters or markers, MUST be rejected with `E4029`. No program MUST construct a value of an intrinsic struct, read a field of one, or take one apart: only its intrinsic functions make and use its values.

#### Scenario: An unknown intrinsic struct is rejected
- **WHEN** a library test module declares `public intrinsic struct Window`
- **THEN** the type checker emits `E4029`, saying that the compiler provides no intrinsic struct `Window`

#### Scenario: An intrinsic struct cannot be constructed
- **WHEN** a workspace file declares `fun f() { val c = Composition() }` or `fun g(s: RecomposeScope) { val (h) = s }`
- **THEN** the type checker rejects each, saying that `Composition` and `RecomposeScope` have no constructor and no fields

### Requirement: The Compose Module
`/prism/compose/runtime` MUST be a prelude module declaring, all as intrinsics, the structs `Composition` and `RecomposeScope : Copy`, the functions `composition`, `remember`, `key` and `currentRecomposeScope`, the members `setContent`, `recompose` and `dump` of `Composition`, and the member `invalidate` of `RecomposeScope`. `remember`, `key` and `currentRecomposeScope` MUST be composables.

#### Scenario: Signatures of the compose module
- **WHEN** `prism abi` prints the ABI of `/prism/compose/runtime`
- **THEN** it holds `public intrinsic fun composition(content: escaping compose () -> Unit): Composition`, `public intrinsic fun Composition.setContent(content: escaping compose () -> Unit)`, `public intrinsic fun Composition.recompose(): Bool`, `public intrinsic fun Composition.dump(): String`, `public intrinsic compose fun <T : Copy> remember(calculation: () -> T): T`, `public intrinsic compose fun key(value: Int, content: compose () -> Unit)`, `public intrinsic compose fun currentRecomposeScope(): RecomposeScope` and `public intrinsic fun RecomposeScope.invalidate()`

#### Scenario: Members of a composition
- **WHEN** a file declares `fun Composition.recompose(): Bool = false`
- **THEN** the type checker emits `E4010`, saying that the member of `/prism/compose/runtime` always takes precedence

### Requirement: The State Declarations of the Compose Module
`/prism/compose/runtime` MUST declare, as intrinsics, the structs `State<T> : Copy` and `MutableState<T> : Copy`, the function `mutableStateOf`, the property `value` of `State<T>` and the writable property `value` of `MutableState<T>`, the members `read` of `State<T>` and `update` and `edit` of `MutableState<T>`, and the composable `SideEffect`.

#### Scenario: Signatures of the state declarations
- **WHEN** `prism abi` prints the ABI of `/prism/compose/runtime`
- **THEN** it holds `public intrinsic struct State<T> : Copy`, `public intrinsic struct MutableState<T> : Copy`, `public intrinsic fun <T> mutableStateOf(value: T): MutableState<T>`, `public intrinsic val <T> State<T>.value: T`, `public intrinsic var <T> MutableState<T>.value: T`, `public intrinsic fun <T, R> State<T>.read(reader: (&T) -> R): R`, `public intrinsic fun <T> MutableState<T>.update(transform: (T) -> T)`, `public intrinsic fun <T> MutableState<T>.edit(editor: (&mut T) -> Unit)` and `public intrinsic compose fun SideEffect(effect: escaping () -> Unit)`

#### Scenario: State handles need no import
- **WHEN** a file without imports declares `compose fun Counter() { val count = remember { mutableStateOf(0) }; SideEffect { } }` and `public fun counter(): MutableState<Int> = mutableStateOf(0)`
- **THEN** it typechecks, and `prism abi` writes the signature of `counter` as `public fun counter(): MutableState<Int>`

### Requirement: Intrinsic Struct Markers Hold for Every Type Argument
The markers the catalog gives a generic intrinsic struct MUST hold for every type argument, unlike the conditional markers of declared generic structs: `State<String>` and `MutableState<List<Int>>` are `Copy`.

#### Scenario: A state of a String is Copy
- **WHEN** `fun f(s: MutableState<String>): Int { val a = s; val b = s; return 1 }` is checked
- **THEN** the file typechecks

### Requirement: Writable Intrinsic Properties
An `intrinsic var` property MUST be an intrinsic extension property that can also be assigned, as the catalog lowers it. It MUST be checked against the catalog like an `intrinsic val`, and declaring as `var` a property the catalog lists as read-only, or the reverse, MUST be `E4029`.

#### Scenario: A writable property the catalog lists as read-only
- **WHEN** a library test module declares `public intrinsic var String.length: Int`
- **THEN** the type checker emits `E4029`, saying that the compiler provides `length` as a read-only property

### Requirement: The Value of a State Is Typed by Its Own Rule
`value` of `State<T>` and `MutableState<T>` MUST be typed by the catalog's rule rather than as a computed property: a copy for a `Copy` `T`, a place for any other `T`, and an assignment target on `MutableState<T>` (compose, "Reading a Copy Value", "Borrowing a Non-Copy Value in Composable Contexts", "Writing a Value").

#### Scenario: Assigning the value of a read-only handle
- **WHEN** `fun f(s: State<Int>) { s.value = 1 }` is checked
- **THEN** the type checker emits `E4003`, saying that a `State<Int>` is read-only, with help suggesting a `MutableState<Int>` parameter

### Requirement: The UI Modules Are Bundled Outside the Prelude
The compiler MUST ship `/prism/compose/ui`, `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3` as bundled modules that are not in the prelude, all files of the package `prism.compose` beside `/prism/compose/runtime`. Their files MUST be checked, formatted and compiled by the library tests like every bundled module, and their externals MUST match `prism-ui`'s entry points.

#### Scenario: The UI modules' externals match the entry points
- **WHEN** `library_tests.rs` checks and compiles every bundled module
- **THEN** `prism/compose/ui.prism`, `prism/compose/foundation.prism`, `prism/compose/layout.prism` and `prism/compose/material3.prism` pass `prism check`, `prism fmt --check` and the backend's check of each external, with no `E6005`

#### Scenario: The UI modules are not in the prelude
- **WHEN** `prism check --explain-cache` runs on a file without imports, and then on the same file with `import /prism/compose/material3`
- **THEN** the first lists no file of `prism/compose/` but `prism/compose/runtime.prism`, and the second also lists `prism/compose/ui.prism`, `prism/compose/foundation.prism`, `prism/compose/layout.prism` and `prism/compose/material3.prism`

### Requirement: The UI Intrinsics
The intrinsic catalog MUST hold, as intrinsics of `/prism/compose/ui`: the one-word structs `NodeId : Copy`, with `package` visibility, and `HandlerId : Copy`, public, whose equality is the runtime entry `prism_ui_handlers_equal`; the `package` composables `Node(kind: Int, update: (NodeId) -> Unit, content: compose () -> Unit)` and `LeafNode(kind: Int, update: (NodeId) -> Unit)`; and the function `runApp`. A declaration of one of them with another signature or in another module MUST be `E4029`.

#### Scenario: The public signature of runApp
- **WHEN** `prism abi` prints the ABI of `/prism/compose/ui`
- **THEN** it holds the public intrinsic function `runApp` with the parameters `title: &String`, `width: Int` and `height: Int`, each with a default, and `content: escaping compose () -> Unit`, the public intrinsic struct `HandlerId : Copy`, and the public intrinsic properties `Int.dp` and `Int.sp`, and none of `NodeId`, `Node` and `LeafNode`, which are not public

#### Scenario: A UI intrinsic in another library module
- **WHEN** a library test module declares `private intrinsic compose fun LeafNode(kind: Int, update: (Int) -> Unit)`
- **THEN** the type checker emits `E4029`, saying that the compiler provides `LeafNode` only in `/prism/compose/ui`

#### Scenario: Calling runApp with its defaults
- **WHEN** `fun main() { runApp { } }` runs with `import /prism/compose/ui` under `prism run --headless` with the script `dump`
- **THEN** stdout is `window 800x600`: `width` and `height` took their defaults

### Requirement: The Unit Properties
The catalog MUST hold the public intrinsic properties `val Int.dp: Dp` and `val Int.sp: TextUnit` of `/prism/compose/ui`, lowered as the constructions `Dp(n)` and `TextUnit(n)`, and members of `Int` wherever that module is visible.

#### Scenario: Unit properties are members of Int
- **WHEN** a file declares `import /prism/compose/ui` and `fun Int.dp(): Int = 0`
- **THEN** the type checker emits `E4010`, saying that the member of `/prism/compose/ui` always takes precedence

## Design Rationale

Decisions from `standard-library` (planned autonomously on 2026-10-18, implemented and reviewed the same day, and logged in `docs/decisions-for-review.md`). Why `prism` is a reserved root package is in the Design Rationale of `package-resolution`.

- **Prism declarations for every name a program uses without declaring it.** `print`, `toString()`, `length`, `listOf` and the list members were type-checker tables, each with a typing, a lowering and a hand-written precedence rule. They are now declarations of the library, so the compose engine adds modules rather than tables, and the library is checked, formatted, cached, instantiated and located in errors like user code.
- **Embedded in the executable** (D1). `crates/prism-library` holds the sources under `prism/` and a static module list of `include_str!`s, so the compiler is one self-contained file that cannot be half-updated or point at the wrong library, reading the library touches no disk, and the compiler identity, which hashes the executable, covers it. A test checks the list against the directory, so no build script is needed. Rejected: a `lib/` directory next to the binary (an install layout that can drift), materializing the sources into the cache, and a `PRISM_LIBRARY_DIR` override (a second input to every build, a follow-up if library authors need it).
- **Ordinary units, no precompiled ABI** (D3). Library files go through Phase 1, Phase 2, instance planning and code generation like any file, keyed by the same inputs (module path, source text, imports' ABI hashes, compiler identity), so their entries are shared by every workspace using one compiler and cache. Only the reached modules are parsed: about 300 lines, a few hundred microseconds per process. An embedded serialized ABI was rejected: a second codec and a second source of truth for well under a millisecond.
- **A prelude found last, as one shared scope** (D4). `/prism/core` and `/prism/collections` are implicit imports of every workspace file, after its explicit imports, as Kotlin's default imports are. Their ABIs become one `PreludeScope` per graph, shared by every checker through an `Arc`, so a file pays a lookup on a miss rather than an ingestion of the prelude (measured at about 13 us per file whatever the workspace's size, against 28 us when each checker ingested it). A local, parameter, declaration or import shadows a prelude name without `E4010` or `E4012`, so adding a library function never breaks a program that declares that name. Library files import what they use, so the library's own dependency order is explicit. Prelude modules count as direct imports for check keys and instance planning. Rejected: per-file ingestion, a re-exporting `prism/prelude` module (Prism has no re-exports), and unshadowable prelude names.
- **`external fun` is a C symbol with Prism's own calling convention** (D5). An external is named by its symbol and called directly, with no slot and no reload, as runtime functions already were. Its machine signature is the one a Prism function of that signature has (scalars by value, everything in memory and every reference by address, a result held in memory through an address passed first), so there is one convention, and the backend compares it with the runtime's table (`E6005`); the library test compiles every external, so a mismatch fails `cargo test`. Externals are not public, generic, extensions, `compose`, defaulted or returning references (`E4029`): a Prism wrapper does each, and keeps runtime symbols out of the library's API. Rejected: symbols derived from module paths (a rename silently breaks the link), annotations (none exist yet), and a separate C-only convention.
- **`intrinsic` declarations checked against a catalog** (D6). An intrinsic declares the signature of a call the compiler lowers itself (`Callee::Builtin`), and the type checker's catalog holds each intrinsic's canonical signature text, so the declaration, its typing and its lowering cannot drift apart (`E4029`). Most intrinsics are typed by their declared signatures through the ordinary paths. `print`/`println` (a text-form argument, or none), `listOf` (any number of elements) and `contains`/`indexOf` (`==` on `T`) are typed by rules until overloading, `vararg` and traits exist; their declarations have the shape their future ones will. `intrinsic val` is the one new declaration form, for `length`, `size` and `indices`; ordinary extension properties stay a separate decision. Defaults on intrinsics are allowed and not compared, since a default is a generator of the declaring file (R8).
- **Intrinsic extensions are members; other library extensions are ordinary** (D7, R1). An intrinsic extension is found before every extension, after fields and the built-in `clone()` and `unbox()`, and a workspace extension of its name on its receiver is `E4010`: members belong to the type's representation, as in Kotlin where a member beats an extension. The helpers (`map`, `forEach`, ...) are ordinary prelude extensions that a file's own or imported extension shadows, so adding a helper never breaks a program. `List` itself stays a reserved built-in type; only its members are declarations.
- **What moved, and what stays** (D8). `print`, `println`, the scalars' `toString()`, `String.length`, `listOf` and the list members moved as declarations. The built-in types, `Range`, `clone()` (defined by the `Clone` marker, not per type), `Heap(...)` and `unbox()` (the parser rejects `Heap` receivers), indexing, operators, `==` and templates stay in the compiler until a later change gives them declarations. Math (`abs`, `min`, `max`) waits for a decision on serving `Int` and `Double` without overloading.
- **`print` prints anything a template part can** (D9). `print(x)` and `println(x)` accept exactly what `"$x"` accepts and print what it would: a `String` borrowed, a scalar copied out, any other type through the `toString()` that `x.toString()` resolves, called on a shared borrow into a statement temporary, so MIR and the backend see a print of a `&String`. Nullables, `Unit`, function values and type parameters are `E4026`, as in templates, so the two never disagree; printing nullables is a follow-up for both.
- **`exitProcess` in `/prism/system`** (D10). Named after Kotlin's `kotlin.system.exitProcess` and, as there, not in the prelude. It flushes the output and exits with `status & 255`, running no drop and no allocation check, as a runtime error does, so statuses are the same on every platform; under `--watch` it ends the run as `main` returning does. It gives the external form a real user end to end. Rejected: `main(): Int` (a change of the entry point), and a runtime error for statuses outside `0..255` (an external has no caller location).
- **The helpers** (D11, R4). Kotlin's names, written over `for` loops and the members, each function parameter last so a trailing lambda binds to it and `it` works. Their function parameters borrow each element and are not `escaping`, so a closure borrows its captures. `filter` consumes its list and moves the kept elements without cloning. `joinToString` takes its transform, since a type parameter has no text form before traits. A closure passed to a helper is not a composable context, so composables are emitted from `for` loops.
- **Library-only forms** (D12). `external` and `intrinsic` are contextual modifiers, after the visibility and before `compose`, so programs using them as names keep working. A workspace file using either is `E3002` in Phase 1, which knows the file's origin, so no ABI ever carries a workspace intrinsic. A body-less `fun` without either is now `E2001`, where the parser accepted it silently.
- **How the library is tested** (D13). `library_tests.rs` checks and compiles every module, formats it, compares the prelude modules' names and the module list. Fixtures add test modules with a `// library:` header (`Library::with_modules`), never reachable from the command line, so library diagnostics have real-source cases.
- **Compose modules come with their changes** (D14). `/prism/compose` (with its prelude entry) and `/prism/ui` are created by `compose-transform`, `state-handles` and `ui-toolkit`, which fill them; empty stubs would specify nothing.
- **Reports and watch sessions** (D15, R3). `prism check`'s report lists workspace files only, and `--explain-cache` lists library files first. Diagnostics take sources from the graph's nodes, never from disk, which library modules and watch overlays need. Library nodes come from the library, never from a watch session's store, so they are never watched, re-parsed or reloaded.
- **`/prism/composer` is a protocol module outside the prelude** (`compose-runtime`, D10). It declares the compose runtime's entry points as private externals with public wrappers over `Int` words, the machine form generated composables pass, so fixtures can write composables in lowered form. Its names never reach a program that does not import it, and it is parsed only when imported. It is not `/prism/compose/runtime`, `compose-transform`'s prelude module for applications.

Decisions from `compose-transform` (planned autonomously on 2026-10-18, reconciled with `compose-runtime`, `receiver-lambdas` and `compose-hot-reload` and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **`/prism/compose/runtime` is a prelude module** (D15, `ui-toolkit` D21). It is the library's first module in a subdirectory (`prism/compose/runtime.prism`, package `prism.compose`), so `MODULES` and its test walk the library recursively; the other compose modules (`ui`, `foundation`, `layout`, `material3`) are `ui-toolkit`'s, outside the prelude. Its declarations are all intrinsics, and a file's own declaration or import of one of its names shadows it.
- **Intrinsic structs** (D16). `intrinsic struct Name<Parameters> : Markers` declares a type without fields whose representation, glue and operations the compiler provides, from a catalog of its own (`Composition`, neither `Copy` nor `Clone`; `RecomposeScope : Copy`); another name, parameter list or marker set in the library is `E4029`, and the form in the workspace `E3002`. `state-handles` declares its handle types with it. Rejected: user-defined destructors, built-in types without declarations, forgeable library structs.
- **`remember` keys are rule-typed** (D12): up to three keys, each a fresh type parameter bounded by `Copy`, so the existing `E4021` and `E4022` checks apply, and a key written as a reference is copied out like any argument of a type parameter.

Decisions from `state-handles` (planned autonomously on 2026-10-18, reconciled with the archived compose changes and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **The state declarations** (D2). `State<T>` and `MutableState<T>` are `public intrinsic struct ... : Copy`, `mutableStateOf`, `read`, `update`, `edit` and the composable `SideEffect` are intrinsic functions of `/prism/compose/runtime`, after Jetpack's names; `read`, `update` and `edit` take a shared receiver, so a handler's captured `val` handle calls them. The catalog types `value` by its own rule (a copy, a place or an assignment target, compose D5) rather than as a computed property.
- **`intrinsic var`** (D4). `MutableState<T>.value` is assigned, so the property form gains `intrinsic var`, checked against the catalog's read-only or writable flag (`E4029` for a mismatch) and recorded in the ABI, as Kotlin declares `val value` on `State` and `var value` on `MutableState`. The writability of an assignment comes from the declaration found. Rejected: one `intrinsic val` whose writability depends on the receiver (the ABI would not say which handle is writable), and ordinary extension properties (a follow-up).

Decisions from `ui-toolkit` (planned on 2026-10-18 and revised on 2026-10-19 by user direction to follow Jetpack Compose's APIs and modules, implemented and reviewed on 2026-10-19, and logged in `docs/decisions-for-review.md`):

- **The toolkit modules** (D21, user direction). `/prism/compose/ui`, `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3` are files of the package directory `prism/compose/` beside `/prism/compose/runtime`, outside the prelude, importing each other with sibling imports (`import ui`) in one direction, as Compose's artifacts depend: `ui` on nothing of the toolkit, `foundation` and `layout` on `ui`, `material3` on all three. A program that imports none pays nothing; one importing `material3` loads four cached ABIs. Rejected: one `/prism/ui`, toolkit modules in the prelude.
- **The UI intrinsics** (D3, D7, D15, D23). `NodeId` (`package`) and `HandlerId` (public, whose catalog equality entry is `prism_ui_handlers_equal`) are one-word intrinsic structs; `Node` and `LeafNode` are `package` intrinsic composables; `runApp` is an intrinsic so that `E4030` names it and its call builds a root content adapter, its defaults being ordinary generators in `/prism/compose/ui` that `BuiltinFunction::declaration_with_defaults` names; `Int.dp` and `Int.sp` are intrinsic properties lowered as constructions. Setters are `package` externals, so only the toolkit's files reach them. `ModifierElement` and `HandlerId` stay public because programs read chains, until fields have visibility.
