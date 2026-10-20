## RENAMED Requirements

- FROM: `### Requirement: The UI Modules Are Bundled Outside the Prelude`
- TO: `### Requirement: The UI Modules Are Bundled`

## ADDED Requirements

### Requirement: Library Names Are Imported
A file MUST see a public function, extension, struct, enum or value of a library module only through an import of that module, as for a workspace module: no library module is imported implicitly. Without the import, a use MUST be reported as for any unknown name (`E4001`) or member (`E4006`), with help naming the import (diagnostics, "Unresolved Library Names Suggest Their Import").

#### Scenario: Library functions are imported
- **WHEN** a file without imports declares `fun main() { println(1) }`, and the same file then declares `import /prism/core`
- **THEN** the first is rejected with `E4001` for `println`, with help naming `import /prism/core`, and the second runs and prints `1`

#### Scenario: Library helpers are imported
- **WHEN** a file without imports declares `fun f(xs: &List<Int>): Int = xs.sumOf { x -> x }`, and the same file then declares `import /prism/collections`
- **THEN** the first is rejected with `E4006` for `sumOf`, with help naming `import /prism/collections`, and the second typechecks

#### Scenario: Members of a library type are imported
- **WHEN** a file without imports declares `fun f(scope: RecomposeScope) { scope.invalidate() }`, and the same file then declares `import /prism/compose/runtime`
- **THEN** the first is rejected with `E4006` for `invalidate`, with help naming `import /prism/compose/runtime`, and the second typechecks

### Requirement: Library Names Follow the Rules of Imports
A library name a file imports MUST behave as a name of any imported module: a top-level declaration of the same name in the file MUST shadow it (type-system, "Top-level declarations shadow imports"), a name or extension exported by two of its imports MUST be `E4012` at use, and the file's own extension MUST shadow an imported one. A file that does not import a library module MUST be free to declare any of its names as its own.

#### Scenario: A file's own declaration of a library name
- **WHEN** a file without imports declares `fun listOf(n: Int): Int = n` and `fun f(): Int = listOf(3)`, and another file declares the same with `import /prism/collections`
- **THEN** both typecheck, the call resolving to `main::listOf` in each

#### Scenario: An imported extension beside the library's
- **WHEN** `lib/util.prism` exports `public fun <T> List<T>.forEach(action: (&T) -> Unit): Unit {}`, a file importing only `/lib/util` declares `fun visit(n: &Int) {}` and `fun f(xs: &List<Int>) { xs.forEach(visit) }`, and a file importing `/lib/util` and `/prism/collections` declares the same
- **THEN** the first typechecks, the call resolving to `lib/util`'s extension, and the second is rejected with `E4012` naming `lib/util` and `prism/collections`

### Requirement: Built-In Types and Their Members Need No Import
The built-in types `Int`, `Double`, `Bool`, `Char`, `String`, `Unit`, `Nothing`, `List`, `Heap` and `Range`, and their members, MUST be in scope in every workspace file without an import. A built-in type's members MUST be the compiler's built-in members (`clone()`, `unbox()`) and the intrinsic extensions declared on it by its member module: `/prism/core` for the scalars and `String`, `/prism/collections` for `List`.

#### Scenario: Members need no import
- **WHEN** a file without imports declares `fun f(xs: &mut List<Int>, s: &String): Int { xs.add(1); return xs.size + s.length + 42.toString().length + xs.indexOf(1) }`
- **THEN** the file typechecks

#### Scenario: An extension of a built-in type from another module is imported
- **WHEN** a file without imports declares `fun f(): Int { val d = 16.dp; return 0 }`
- **THEN** the type checker emits `E4006` for `dp`, with help naming `import /prism/compose/ui`

### Requirement: Intrinsic Functions Are Only Called
A library intrinsic function MUST only be called: naming it without a call MUST be rejected with `E4013`, since it has no code to name.

#### Scenario: Library intrinsics are only called
- **WHEN** a file declaring `import /prism/core` checks `fun f() { val p = println }`
- **THEN** the type checker emits `E4013` for `println`, a generic function used as a value

### Requirement: Scenarios Omit Library Imports
A scenario of these specifications whose program uses a library function, extension or type without showing an import MUST be read as that program with an import of each library module declaring a name it uses, and its fixture MUST declare those imports. A scenario about imports MUST show them.

#### Scenario: A scenario's program and its fixture
- **WHEN** a scenario runs `fun main() { println(listOf(1, 2).size) }` and shows no import
- **THEN** its fixture declares `import /prism/collections` and `import /prism/core`, and prints `2`

### Requirement: The Collections Module Declares Lists and Their Helpers
`/prism/collections` MUST declare, as public intrinsics, `listOf` and the members of `List<T>` (collections, "List Members Are Typed as Extensions", "Mutating Members", "Size and Indices Are Read Without Parentheses"), and, as public Prism functions, the higher-order helpers (collections, "Higher-Order Helpers").

#### Scenario: Helpers are imported
- **WHEN** `import /prism/collections`, `import /prism/core` and `fun main() { val xs = listOf(1, 2, 3); println(xs.map { x -> x * 2 }.sumOf { x -> x }) }` run
- **THEN** it prints `12`

### Requirement: The State Declarations of the Compose Runtime Module
`/prism/compose/runtime` MUST declare, as intrinsics, the structs `State<T>`, `MutableState<T>` and `WeakReference<T> : Copy`, the function `mutableStateOf`, the property `value` of `State<T>` and the writable property `value` of `MutableState<T>`, the member `update` of `MutableState<T>`, the capture functions `weak` of `State<T>` and of `MutableState<T>`, the member `get` of `WeakReference<T>`, and the composable `SideEffect`.

#### Scenario: Signatures of the state declarations
- **WHEN** `prism abi` prints the ABI of `/prism/compose/runtime`
- **THEN** it holds `public intrinsic struct State<T>`, `public intrinsic struct MutableState<T>`, `public intrinsic struct WeakReference<T> : Copy`, `public intrinsic fun <T> mutableStateOf(value: T): MutableState<T>`, `public intrinsic val <T> State<T>.value: T`, `public intrinsic var <T> MutableState<T>.value: T`, `public intrinsic fun <T> MutableState<T>.update(transform: (T) -> T)`, `public intrinsic capture fun <T> State<T>.weak(): WeakReference<State<T>>`, `public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>`, `public intrinsic fun <T> WeakReference<T>.get(): (&T)?` and `public intrinsic compose fun SideEffect(effect: attached () -> Unit)`

#### Scenario: State declarations are imported
- **WHEN** a file declaring `import /prism/compose/runtime` declares `compose fun Counter() { val count = remember { mutableStateOf(0) }; SideEffect { } }` and `public fun counter(): MutableState<Int> = mutableStateOf(0)`
- **THEN** it typechecks, and `prism abi` writes the signature of `counter` as `public fun counter(): MutableState<Int>`

## MODIFIED Requirements

### Requirement: The Compiler Ships the Library
The `prism` executable MUST contain the source of every library module and MUST NOT read library files from disk. The sources MUST be the `.prism` files under the top-level `library/` directory of the compiler's repository, the module `prism/<name>` being the file `library/prism/<name>.prism`. The modules MUST be `/prism/core`, `/prism/collections`, `/prism/system`, `/prism/composer`, `/prism/compose/runtime` and the UI modules, each a file of the package `prism` or of a package below it.

#### Scenario: A library module is imported
- **WHEN** `app/main.prism` declares `import /prism/core`, `import /prism/system` and `fun main() { println(1); exitProcess(3) }`, in a workspace with no `prism` directory
- **THEN** `prism run app/main.prism` prints `1` and exits with status 3

#### Scenario: The composer module is not in the prelude
- **WHEN** `prism check --explain-cache main.prism` runs on a file without imports, and again after the file declares `import /prism/composer`
- **THEN** the first explanation names no `prism/composer.prism`, and the second names it

### Requirement: Library Modules Compile Like Workspace Files
A library module MUST go through every stage a workspace file does: Phase 1 extracts its ABI, Phase 2 checks its bodies, its generics are instantiated per program, and its code is a unit of its own, cached like a workspace file's. A program MUST parse only the library modules it reaches: the member modules (`/prism/core` and `/prism/collections`) and those imported, directly or transitively.

#### Scenario: Unreached modules are not parsed
- **WHEN** `prism check --explain-cache main.prism` runs on a file without imports
- **THEN** its file lines name `prism/core.prism`, `prism/collections.prism` and `main.prism`, and no other library module

#### Scenario: An imported module is parsed
- **WHEN** `prism check --explain-cache main.prism` runs on a file that declares `import /prism/compose/runtime`
- **THEN** its file lines name `prism/core.prism`, `prism/collections.prism`, `prism/compose/runtime.prism` and `main.prism`, and no other library module

### Requirement: Library Files Import What They Use
A library file MUST NOT see any library module implicitly, the member modules included: the names and members it uses from another library module come from an explicit import of that module.

#### Scenario: A library module imports the core
- **WHEN** a library test module that uses `println` declares no import
- **THEN** the type checker emits `E4001` for `println` in that module, and accepts it once the module declares `import core`

#### Scenario: A library module imports the members it reads
- **WHEN** a library test module declares `public fun width(s: &String): Int = s.length` and no import
- **THEN** the type checker emits `E4006` for `length` in that module, and accepts it once the module declares `import core`

### Requirement: Intrinsic Extensions Are Members
An intrinsic extension function or property MUST be a member of its receiver type wherever it is in scope: in every workspace file for the intrinsic extensions of the member modules, and where its module is imported for any other. A call or read of that name on that type, through any `&`, `&mut` and `Heap<T>` layers, MUST use it before any extension, after fields and the built-in `clone()` and `unbox()`. A workspace extension of that name on that receiver type constructor, where the member is in scope, MUST be rejected with `E4010`.

#### Scenario: A workspace extension named like a member
- **WHEN** a file declares `fun Int.toString(): String = "x"`, `fun String.length(): Int = 0` and `fun <T> List<T>.add(element: T) {}`
- **THEN** the type checker emits `E4010` for each, saying that the member of `/prism/core` or `/prism/collections` always takes precedence

#### Scenario: A workspace extension named like a helper
- **WHEN** a file declares `fun <T> List<T>.map(f: (&T) -> Int): Int = 0` and `fun f(xs: &List<Int>): Int = xs.map { x -> x }`
- **THEN** the file typechecks, and the call resolves to the file's own extension

### Requirement: The Library Passes Its Own Checks
Every library module MUST pass `prism check` with no diagnostic and `prism fmt --check`, and every external MUST match the runtime's function table. The modules the executable contains MUST be exactly the `.prism` files of the top-level `library/` directory.

#### Scenario: The library is checked by the compiler's tests
- **WHEN** `cargo test -p prism-cli --test library_tests` runs
- **THEN** it checks and formats every embedded module, compares every external with the runtime table, and compares the embedded module list with the files of `library/prism/`, failing on any difference

### Requirement: The Core Module
`/prism/core` MUST declare, as public intrinsics, `print` and `println`, `toString()` of `Int`, `Double`, `Bool` and `Char` (`fun Int.toString(): String`, ...), and the property `val String.length: Int`. Their behavior is that of the type-system and execution requirements for them.

#### Scenario: Core names need no import
- **WHEN** a file without imports declares `fun f(): Int { val s = 42.toString(); return s.length }`
- **THEN** it typechecks: `toString()` and `length` are members of built-in types

#### Scenario: Core functions are imported
- **WHEN** `import /prism/core` and `fun main() { val s = 42.toString(); println(s.length) }` run, and the same `main` is checked without the import
- **THEN** the first prints `2`, and the second is rejected with `E4001` for `println` alone

### Requirement: The Compose Module
`/prism/compose/runtime` MUST declare, all as intrinsics, the structs `Composition` and `RecomposeScope : Copy`, the functions `composition`, `remember`, `key` and `currentRecomposeScope`, the members `setContent`, `recompose` and `dump` of `Composition`, and the member `invalidate` of `RecomposeScope`. `remember`, `key` and `currentRecomposeScope` MUST be composables.

#### Scenario: Signatures of the compose module
- **WHEN** `prism abi` prints the ABI of `/prism/compose/runtime`
- **THEN** it holds `public intrinsic fun composition(content: attached compose () -> Unit): Composition`, `public intrinsic fun Composition.setContent(content: attached compose () -> Unit)`, `public intrinsic fun Composition.recompose(): Bool`, `public intrinsic fun Composition.dump(): String`, `public intrinsic compose fun <T> remember(calculation: () -> T): T`, `public intrinsic compose fun key(value: Int, content: compose () -> Unit)`, `public intrinsic compose fun currentRecomposeScope(): RecomposeScope` and `public intrinsic fun RecomposeScope.invalidate()`

#### Scenario: Members of a composition
- **WHEN** a file declares `import /prism/compose/runtime` and `fun Composition.recompose(): Bool = false`
- **THEN** the type checker emits `E4010`, saying that the member of `/prism/compose/runtime` always takes precedence

### Requirement: The UI Modules Are Bundled
The compiler MUST ship `/prism/compose/ui`, `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3` as bundled modules, all files of the package `prism.compose` beside `/prism/compose/runtime`. Their files MUST be checked, formatted and compiled by the library tests like every bundled module, and their externals MUST match `prism-ui`'s entry points.

#### Scenario: The UI modules' externals match the entry points
- **WHEN** `library_tests.rs` checks and compiles every bundled module
- **THEN** `prism/compose/ui.prism`, `prism/compose/foundation.prism`, `prism/compose/layout.prism` and `prism/compose/material3.prism` pass `prism check`, `prism fmt --check` and the backend's check of each external, with no `E6005`

#### Scenario: The UI modules are not in the prelude
- **WHEN** `prism check --explain-cache` runs on a file without imports, and then on the same file with `import /prism/compose/material3`
- **THEN** the first lists no file of `prism/compose/`, and the second lists `prism/compose/ui.prism`, `prism/compose/foundation.prism`, `prism/compose/layout.prism` and `prism/compose/material3.prism`, and not `prism/compose/runtime.prism`

### Requirement: The Unit Properties
The catalog MUST hold the public intrinsic properties `val Int.dp: Dp` and `val Int.sp: TextUnit` of `/prism/compose/ui`, lowered as the constructions `Dp(n)` and `TextUnit(n)`, and members of `Int` wherever that module is imported.

#### Scenario: Unit properties are members of Int
- **WHEN** a file declares `import /prism/compose/ui` and `fun Int.dp(): Int = 0`
- **THEN** the type checker emits `E4010`, saying that the member of `/prism/compose/ui` always takes precedence

## REMOVED Requirements

### Requirement: Prelude Names Are Found Last
**Reason**: There is no prelude (user direction 2026-10-20): every library name comes from an explicit import, so no name is found after the imports.
**Migration**: Add the import of the library module that declares the name (`import /prism/core` for `print` and `println`, `import /prism/collections` for `listOf`, `import /prism/compose/runtime` for `remember`); see "Library Names Are Imported" and "Library Names Follow the Rules of Imports". The scenario "Prelude intrinsics are only called" moves to "Intrinsic Functions Are Only Called" as "Library intrinsics are only called".

### Requirement: Prelude Extensions Are Found After Imported Ones
**Reason**: Library extensions are imported like any extension, so they no longer form a layer below the imports.
**Migration**: Import `/prism/collections` to call a helper; a file importing another module that exports an extension of the same receiver and name gets `E4012`, as for any two imports ("Library Names Follow the Rules of Imports", "An imported extension beside the library's").

### Requirement: The Collections Module
**Reason**: Its scenario "Helpers need no import" no longer holds: the helpers are imported.
**Migration**: See "The Collections Module Declares Lists and Their Helpers", which keeps the declarations and replaces the scenario with "Helpers are imported".

### Requirement: The State Declarations of the Compose Module
**Reason**: Its scenario "State handles need no import" no longer holds: `/prism/compose/runtime` is imported.
**Migration**: See "The State Declarations of the Compose Runtime Module", which keeps the declarations and replaces the scenario with "State declarations are imported".
