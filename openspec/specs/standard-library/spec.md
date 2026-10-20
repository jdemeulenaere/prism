# Standard Library Specification

## Purpose
The standard library is the Prism source the compiler ships under the reserved root package `prism`, kept in the repository's top-level `library/` directory and embedded in the executable: which modules exist, how files import their names (no module is implicit; only the built-in types and their members are everywhere), the `external` and `intrinsic` declaration forms that only the library may use, and how the library is checked.

## Requirements

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
- `contains` and `indexOf` of `List<T>` also need `==` on `T`;
- `remember` also takes keys ("Remember Takes Up to Three Keys").

#### Scenario: A signature-typed intrinsic
- **WHEN** `fun f(xs: &List<Int>): Bool = xs.isEmpty(1)` is checked
- **THEN** the type checker emits `E4004`, since `isEmpty` is declared with no parameter

#### Scenario: A rule-typed intrinsic
- **WHEN** `fun f() { println(); println(1) }` is checked
- **THEN** the file typechecks, although `println` is declared with one parameter

### Requirement: Remember Takes Up to Three Keys
A call of `remember` (`fun <T> remember(calculation: () -> T): T`, whose result the catalog types as a copy of a `Copy` `T` and as a shared borrow of any other `T`) MUST also accept up to three keys before the calculation, each typed by a type parameter of its own bounded by `Copy` and borrow-free (compose, "Remembering With Keys").

#### Scenario: Keys of remember
- **WHEN** `compose fun f(a: Int, b: Bool) { val x = remember(a, b) { 1 }; val y = remember(a, b, 'c', 1.5) { 2 } }` is checked
- **THEN** the type checker accepts the first call and reports an argument count error (`E4004`) for the second, which has four keys

### Requirement: Intrinsic Extensions Are Members
An intrinsic extension function or property MUST be a member of its receiver type wherever it is in scope: in every workspace file for those of the member modules, where its module is imported for any other. A call or read of that name on that type, through `&`, `&mut` and `Heap<T>`, MUST use it before any extension, after fields and the built-in `clone()` and `unbox()`. A workspace extension of that name on that receiver type constructor where the member is in scope MUST be `E4010`.

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
- **WHEN** a file without imports declares `fun f(): Int { val s = 42.toString(); return s.length }`
- **THEN** it typechecks: `toString()` and `length` are members of built-in types

#### Scenario: Core functions are imported
- **WHEN** `import /prism/core` and `fun main() { val s = 42.toString(); println(s.length) }` run, and the same `main` is checked without the import
- **THEN** the first prints `2`, and the second is rejected with `E4001` for `println` alone

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
Every library module MUST pass `prism check` with no diagnostic and `prism fmt --check`, and every external MUST match the runtime's function table. The modules the executable contains MUST be exactly the `.prism` files of the top-level `library/` directory.

#### Scenario: The library is checked by the compiler's tests
- **WHEN** `cargo test -p prism-cli --test library_tests` runs
- **THEN** it checks and formats every embedded module, compares every external with the runtime table, and compares the embedded module list with the files of `library/prism/`, failing on any difference

### Requirement: Intrinsic Structs
An `intrinsic struct` MUST declare a type whose representation, glue and operations the compiler provides from its catalog, with the markers the catalog gives it. A library declaration naming no intrinsic struct of the catalog, or with other type parameters or markers, MUST be rejected with `E4029`. No program MUST construct a value of an intrinsic struct, read a field of one, or take one apart: only its intrinsic functions make and use its values.

#### Scenario: An unknown intrinsic struct is rejected
- **WHEN** a library test module declares `public intrinsic struct Window`
- **THEN** the type checker emits `E4029`, saying that the compiler provides no intrinsic struct `Window`

#### Scenario: An intrinsic struct cannot be constructed
- **WHEN** a workspace file declares `fun f() { val c = Composition() }` or `fun g(s: RecomposeScope) { val (h) = s }`
- **THEN** the type checker rejects each, saying that `Composition` and `RecomposeScope` have no constructor and no fields

### Requirement: The Compose Module
`/prism/compose/runtime` MUST declare, all as intrinsics, the structs `Composition` and `RecomposeScope : Copy`, the functions `composition`, `remember`, `key` and `currentRecomposeScope`, the members `setContent`, `recompose` and `dump` of `Composition`, and the member `invalidate` of `RecomposeScope`. `remember`, `key` and `currentRecomposeScope` MUST be composables.

#### Scenario: Signatures of the compose module
- **WHEN** `prism abi` prints the ABI of `/prism/compose/runtime`
- **THEN** it holds `public intrinsic fun composition(content: attached compose () -> Unit): Composition`, `public intrinsic fun Composition.setContent(content: attached compose () -> Unit)`, `public intrinsic fun Composition.recompose(): Bool`, `public intrinsic fun Composition.dump(): String`, `public intrinsic compose fun <T> remember(calculation: () -> T): T`, `public intrinsic compose fun key(value: Int, content: compose () -> Unit)`, `public intrinsic compose fun currentRecomposeScope(): RecomposeScope` and `public intrinsic fun RecomposeScope.invalidate()`

#### Scenario: Members of a composition
- **WHEN** a file declares `import /prism/compose/runtime` and `fun Composition.recompose(): Bool = false`
- **THEN** the type checker emits `E4010`, saying that the member of `/prism/compose/runtime` always takes precedence

### Requirement: Writable Intrinsic Properties
An `intrinsic var` property MUST be an intrinsic extension property that can also be assigned, as the catalog lowers it. It MUST be checked against the catalog like an `intrinsic val`, and declaring as `var` a property the catalog lists as read-only, or the reverse, MUST be `E4029`.

#### Scenario: A writable property the catalog lists as read-only
- **WHEN** a library test module declares `public intrinsic var String.length: Int`
- **THEN** the type checker emits `E4029`, saying that the compiler provides `length` as a read-only property

### Requirement: The Value of a State Is Typed by Its Own Rule
`value` of `State<T>` and `MutableState<T>` MUST be typed by the catalog's rule rather than as a computed property: a copy for a `Copy` `T`, a place for any other `T`, and an assignment target on a `MutableState<T>` reached by value or through any reference (compose, "Reading a Copy Value", "A State's Value Is a Place", "Writing a Value").

#### Scenario: Assigning the value of a read-only handle
- **WHEN** `fun f(s: &State<Int>) { s.value = 1 }` is checked
- **THEN** the type checker emits `E4003`, saying that a `State<Int>` is read-only, with help suggesting a `&MutableState<Int>` parameter

### Requirement: The UI Modules Are Bundled
The compiler MUST ship `/prism/compose/ui`, `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3` as bundled modules, all files of the package `prism.compose` beside `/prism/compose/runtime`. Their files MUST be checked, formatted and compiled by the library tests like every bundled module, and their externals MUST match `prism-ui`'s entry points.

#### Scenario: The UI modules' externals match the entry points
- **WHEN** `library_tests.rs` checks and compiles every bundled module
- **THEN** `prism/compose/ui.prism`, `prism/compose/foundation.prism`, `prism/compose/layout.prism` and `prism/compose/material3.prism` pass `prism check`, `prism fmt --check` and the backend's check of each external, with no `E6005`

#### Scenario: The UI modules are not in the prelude
- **WHEN** `prism check --explain-cache` runs on a file without imports, and then on the same file with `import /prism/compose/material3`
- **THEN** the first lists no file of `prism/compose/`, and the second lists `prism/compose/ui.prism`, `prism/compose/foundation.prism`, `prism/compose/layout.prism` and `prism/compose/material3.prism`, and not `prism/compose/runtime.prism`

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
The catalog MUST hold the public intrinsic properties `val Int.dp: Dp` and `val Int.sp: TextUnit` of `/prism/compose/ui`, lowered as the constructions `Dp(n)` and `TextUnit(n)`, and members of `Int` wherever that module is imported.

#### Scenario: Unit properties are members of Int
- **WHEN** a file declares `import /prism/compose/ui` and `fun Int.dp(): Int = 0`
- **THEN** the type checker emits `E4010`, saying that the member of `/prism/compose/ui` always takes precedence

### Requirement: Intrinsic Struct Markers Are Unconditional
The markers the catalog gives a generic intrinsic struct MUST hold for every type argument, unlike the conditional markers of declared generic structs: `WeakReference<MutableState<String>>` is `Copy`, and `MutableState<Int>` is not, although `Int` is.

#### Scenario: A weak reference of a String state is Copy
- **WHEN** `fun f(s: &MutableState<String>): Int { val r = s.weak(); val a = r; val b = r; return 1 }` is checked
- **THEN** the file typechecks

#### Scenario: A state of an Int is not Copy
- **WHEN** `fun g(s: MutableState<Int>): Int { val a = s; val b = s; return 1 }` is checked
- **THEN** the borrow checker emits `E5001` at the second `s`

### Requirement: Capture Functions Are Library Declarations
A `capture` function MUST be an intrinsic extension that the catalog lists as the capture function of its receiver's type, declared in that type's module, with a shared receiver, no parameters, no type parameters but its receiver's, and the result `WeakReference` of its receiver type. A type MUST have at most one. Any other `capture` declaration MUST be `E4029`. The compiler MUST find a type's capture function through its declarations, never by the type's name.

#### Scenario: A capture function the catalog does not list
- **WHEN** a library test module declares `public intrinsic capture fun String.weak(): WeakReference<String>`
- **THEN** the type checker emits `E4029`, saying that the compiler provides no capture function for `String`

#### Scenario: A capture function of the wrong shape
- **WHEN** a library test module of `/prism/compose/runtime` declares `public intrinsic capture fun <T> MutableState<T>.weak(n: Int): WeakReference<MutableState<T>>`
- **THEN** the type checker emits `E4029`, saying that a capture function takes no parameters

### Requirement: Library Names Are Imported
A file MUST see a public function, extension, struct, enum or value of a library module only through an import of that module, as for a workspace module: no library module is imported implicitly. Without the import, a use MUST be reported as for any unknown name (`E4001`) or member (`E4006`), and an annotation naming such a struct or enum MUST be `E4001`, never an opaque type, each with help naming the import (diagnostics, "Unresolved Library Names Suggest Their Import").

#### Scenario: Library functions are imported
- **WHEN** a file without imports declares `fun main() { println(1) }`, and the same file then declares `import /prism/core`
- **THEN** the first is rejected with `E4001` for `println`, with help naming `import /prism/core`, and the second runs and prints `1`

#### Scenario: Library helpers are imported
- **WHEN** a file without imports declares `fun f(xs: &List<Int>): Int = xs.sumOf { x -> x }`, and the same file then declares `import /prism/collections`
- **THEN** the first is rejected with `E4006` for `sumOf`, with help naming `import /prism/collections`, and the second typechecks

#### Scenario: Members of a library type are imported
- **WHEN** a file without imports declares `fun f(scope: RecomposeScope) { scope.invalidate() }`, and the same file then declares `import /prism/compose/runtime`
- **THEN** the first is rejected with `E4001` for `RecomposeScope` alone, with help naming `import /prism/compose/runtime`, and the second typechecks

#### Scenario: Library types are imported
- **WHEN** a file without imports declares `struct Holder { val c: Composition }` and `fun f(s: &MutableState<Int>?) {}`, the same file then declares `import /prism/compose/runtime`, and a file without imports declares its own `struct Composition(val n: Int)` and `fun size(c: &Composition): Int = c.n`
- **THEN** the first is rejected with `E4001` at `Composition` and at `MutableState`, each with help naming `import /prism/compose/runtime`, and the second and third typecheck

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
`/prism/collections` MUST declare, as public intrinsics, the members of `List<T>` (collections, "List Members Are Typed as Extensions", "Mutating Members", "Size and Indices Are Read Without Parentheses"), and, as public Prism functions, `listOf` (collections, "Lists Are Built With listOf") and the higher-order helpers (collections, "Higher-Order Helpers").

#### Scenario: Helpers are imported
- **WHEN** `import /prism/collections`, `import /prism/core` and `fun main() { val xs = listOf(1, 2, 3); println(xs.map { x -> x * 2 }.sumOf { x -> x }) }` run
- **THEN** it prints `12`

#### Scenario: listOf is written in Prism
- **WHEN** `cargo test -p prism-cli --test library_tests` checks the library, and `prism dump-typed-tree` runs on a file declaring `import /prism/collections` and `fun f(): List<Int> = listOf(1, 2)`
- **THEN** `library/prism/collections.prism` declares `public fun <T> listOf(vararg elements: T): List<T> = elements` and passes its checks, the intrinsic catalog has no `listOf`, and the body of `f` is a call of `prism/collections::listOf<Int>` whose argument is a call of `builtin varargList<Int>` with the literals `1` and `2`

### Requirement: The State Declarations of the Compose Runtime Module
`/prism/compose/runtime` MUST declare, as intrinsics, the structs `State<T>`, `MutableState<T>` and `WeakReference<T> : Copy`, the function `mutableStateOf`, the property `value` of `State<T>` and the writable property `value` of `MutableState<T>`, the member `update` of `MutableState<T>`, the capture functions `weak` of `State<T>` and of `MutableState<T>`, the member `get` of `WeakReference<T>`, and the composable `SideEffect`.

#### Scenario: Signatures of the state declarations
- **WHEN** `prism abi` prints the ABI of `/prism/compose/runtime`
- **THEN** it holds `public intrinsic struct State<T>`, `public intrinsic struct MutableState<T>`, `public intrinsic struct WeakReference<T> : Copy`, `public intrinsic fun <T> mutableStateOf(value: T): MutableState<T>`, `public intrinsic val <T> State<T>.value: T`, `public intrinsic var <T> MutableState<T>.value: T`, `public intrinsic fun <T> MutableState<T>.update(transform: (T) -> T)`, `public intrinsic capture fun <T> State<T>.weak(): WeakReference<State<T>>`, `public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>`, `public intrinsic fun <T> WeakReference<T>.get(): (&T)?` and `public intrinsic compose fun SideEffect(effect: attached () -> Unit)`

#### Scenario: State declarations are imported
- **WHEN** a file declaring `import /prism/compose/runtime` declares `compose fun Counter() { val count = remember { mutableStateOf(0) }; SideEffect { } }` and `public fun counter(): MutableState<Int> = mutableStateOf(0)`
- **THEN** it typechecks, and `prism abi` writes the signature of `counter` as `public fun counter(): MutableState<Int>`

### Requirement: Intrinsic Operators
An `intrinsic` declaration MAY carry `operator` when the catalog lists the intrinsic as an operator: `List<T>.contains` of `/prism/collections` is declared `public intrinsic operator fun <T> List<T>.contains(element: &T): Bool`. A declaration whose `operator` flag differs from the catalog's MUST be rejected with `E4029`, as any other signature difference.

#### Scenario: The list membership operator
- **WHEN** a file declares `fun f(xs: &List<Int>): Bool = 3 in xs`
- **THEN** the file typechecks, `in` calling the intrinsic member `contains` of `List`

#### Scenario: An intrinsic's operator flag must match the catalog
- **WHEN** a library test module declares `public intrinsic fun <T> List<T>.contains(element: &T): Bool` without `operator`, or `public intrinsic operator fun <T> List<T>.indexOf(element: &T): Int`
- **THEN** the type checker emits `E4029` for each, showing the signature the compiler provides

## Design Rationale

Decisions from `standard-library` (planned autonomously on 2026-10-18, implemented and reviewed the same day, and logged in `docs/decisions-for-review.md`). Why `prism` is a reserved root package is in the Design Rationale of `package-resolution`.

- **Prism declarations for every name a program uses without declaring it.** `print`, `toString()`, `length`, `listOf` and the list members were type-checker tables, each with a typing, a lowering and a hand-written precedence rule. They are now declarations of the library, so the compose engine adds modules rather than tables, and the library is checked, formatted, cached, instantiated and located in errors like user code.
- **Embedded in the executable** (D1; the location superseded by `explicit-library` D5, below). `crates/prism-library` held the sources under `prism/` and holds a static module list of `include_str!`s, so the compiler is one self-contained file that cannot be half-updated or point at the wrong library, reading the library touches no disk, and the compiler identity, which hashes the executable, covers it. A test checks the list against the directory, so no build script is needed. Rejected: a `lib/` directory next to the binary (an install layout that can drift), materializing the sources into the cache, and a `PRISM_LIBRARY_DIR` override (a second input to every build, a follow-up if library authors need it).
- **Ordinary units, no precompiled ABI** (D3). Library files go through Phase 1, Phase 2, instance planning and code generation like any file, keyed by the same inputs (module path, source text, imports' ABI hashes, compiler identity), so their entries are shared by every workspace using one compiler and cache. Only the reached modules are parsed: about 300 lines, a few hundred microseconds per process. An embedded serialized ABI was rejected: a second codec and a second source of truth for well under a millisecond.
- **A prelude found last, as one shared scope** (D4; superseded by `explicit-library` D1, below, which removed the prelude). `/prism/core` and `/prism/collections` are implicit imports of every workspace file, after its explicit imports, as Kotlin's default imports are. Their ABIs become one `PreludeScope` per graph, shared by every checker through an `Arc`, so a file pays a lookup on a miss rather than an ingestion of the prelude (measured at about 13 us per file whatever the workspace's size, against 28 us when each checker ingested it). A local, parameter, declaration or import shadows a prelude name without `E4010` or `E4012`, so adding a library function never breaks a program that declares that name. Library files import what they use, so the library's own dependency order is explicit. Prelude modules count as direct imports for check keys and instance planning. Rejected: per-file ingestion, a re-exporting `prism/prelude` module (Prism has no re-exports), and unshadowable prelude names.
- **`external fun` is a C symbol with Prism's own calling convention** (D5). An external is named by its symbol and called directly, with no slot and no reload, as runtime functions already were. Its machine signature is the one a Prism function of that signature has (scalars by value, everything in memory and every reference by address, a result held in memory through an address passed first), so there is one convention, and the backend compares it with the runtime's table (`E6005`); the library test compiles every external, so a mismatch fails `cargo test`. Externals are not public, generic, extensions, `compose`, defaulted or returning references (`E4029`): a Prism wrapper does each, and keeps runtime symbols out of the library's API. Rejected: symbols derived from module paths (a rename silently breaks the link), annotations (none exist yet), and a separate C-only convention.
- **`intrinsic` declarations checked against a catalog** (D6). An intrinsic declares the signature of a call the compiler lowers itself (`Callee::Builtin`), and the type checker's catalog holds each intrinsic's canonical signature text, so the declaration, its typing and its lowering cannot drift apart (`E4029`). Most intrinsics are typed by their declared signatures through the ordinary paths. `print`/`println` (a text-form argument, or none), `listOf` (any number of elements) and `contains`/`indexOf` (`==` on `T`) are typed by rules until overloading, `vararg` and traits exist; their declarations have the shape their future ones will. `intrinsic val` is the one new declaration form, for `length`, `size` and `indices`; ordinary extension properties stay a separate decision. Defaults on intrinsics are allowed and not compared, since a default is a generator of the declaring file (R8).
- **Intrinsic extensions are members; other library extensions are ordinary** (D7, R1). An intrinsic extension is found before every extension, after fields and the built-in `clone()` and `unbox()`, and a workspace extension of its name on its receiver is `E4010`: members belong to the type's representation, as in Kotlin where a member beats an extension. The helpers (`map`, `forEach`, ...) are ordinary extensions, imported with `/prism/collections` since `explicit-library` (they were prelude extensions before). `List` itself stays a reserved built-in type; only its members are declarations.
- **What moved, and what stays** (D8). `print`, `println`, the scalars' `toString()`, `String.length`, `listOf` and the list members moved as declarations. The built-in types, `Range`, `clone()` (defined by the `Clone` marker, not per type), `Heap(...)` and `unbox()` (the parser rejects `Heap` receivers), indexing, operators, `==` and templates stay in the compiler until a later change gives them declarations. Math (`abs`, `min`, `max`) waits for a decision on serving `Int` and `Double` without overloading.
- **`print` prints anything a template part can** (D9). `print(x)` and `println(x)` accept exactly what `"$x"` accepts and print what it would: a `String` borrowed, a scalar copied out, any other type through the `toString()` that `x.toString()` resolves, called on a shared borrow into a statement temporary, so MIR and the backend see a print of a `&String`. Nullables, `Unit`, function values and type parameters are `E4026`, as in templates, so the two never disagree; printing nullables is a follow-up for both.
- **`exitProcess` in `/prism/system`** (D10). Named after Kotlin's `kotlin.system.exitProcess` and, as there, imported explicitly. It flushes the output and exits with `status & 255`, running no drop and no allocation check, as a runtime error does, so statuses are the same on every platform; under `--watch` it ends the run as `main` returning does. It gives the external form a real user end to end. Rejected: `main(): Int` (a change of the entry point), and a runtime error for statuses outside `0..255` (an external has no caller location).
- **The helpers** (D11, R4). Kotlin's names, written over `for` loops and the members, each function parameter last so a trailing lambda binds to it and `it` works. Their function parameters borrow each element and are not `escaping`, so a closure borrows its captures. `filter` consumes its list and moves the kept elements without cloning. `joinToString` takes its transform, since a type parameter has no text form before traits. A closure passed to a helper is not a composable context, so composables are emitted from `for` loops.
- **Library-only forms** (D12). `external` and `intrinsic` are contextual modifiers, after the visibility and before `compose`, so programs using them as names keep working. A workspace file using either is `E3002` in Phase 1, which knows the file's origin, so no ABI ever carries a workspace intrinsic. A body-less `fun` without either is now `E2001`, where the parser accepted it silently.
- **How the library is tested** (D13). `library_tests.rs` checks and compiles every module, formats it, checks that the member modules' intrinsic extensions all have built-in receivers (the prelude modules' names before `explicit-library`), and compares the module list with `library/prism/`. Fixtures add test modules with a `// library:` header (`Library::with_modules`), never reachable from the command line, so library diagnostics have real-source cases.
- **Compose modules come with their changes** (D14). `/prism/compose` (with what was its prelude entry) and `/prism/ui` are created by `compose-transform`, `state-handles` and `ui-toolkit`, which fill them; empty stubs would specify nothing.
- **Reports and watch sessions** (D15, R3). `prism check`'s report lists workspace files only, and `--explain-cache` lists library files first. Diagnostics take sources from the graph's nodes, never from disk, which library modules and watch overlays need. Library nodes come from the library, never from a watch session's store, so they are never watched, re-parsed or reloaded.
- **`/prism/composer` is a protocol module outside the prelude** (`compose-runtime`, D10). It declares the compose runtime's entry points as private externals with public wrappers over `Int` words, the machine form generated composables pass, so fixtures can write composables in lowered form. Its names never reach a program that does not import it, and it is parsed only when imported. It is not `/prism/compose/runtime`, `compose-transform`'s module for applications.

Decisions from `compose-transform` (planned autonomously on 2026-10-18, reconciled with `compose-runtime`, `receiver-lambdas` and `compose-hot-reload` and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **`/prism/compose/runtime` was a prelude module** (D15, `ui-toolkit` D21; imported explicitly since `explicit-library`). It is the library's first module in a subdirectory (`prism/compose/runtime.prism`, package `prism.compose`), so `MODULES` and its test walk the library recursively; the other compose modules (`ui`, `foundation`, `layout`, `material3`) are `ui-toolkit`'s, outside the prelude. Its declarations are all intrinsics, and a file's own declaration or import of one of its names shadows it.
- **Intrinsic structs** (D16). `intrinsic struct Name<Parameters> : Markers` declares a type without fields whose representation, glue and operations the compiler provides, from a catalog of its own (`Composition`, neither `Copy` nor `Clone`; `RecomposeScope : Copy`); another name, parameter list or marker set in the library is `E4029`, and the form in the workspace `E3002`. `state-handles` declares its handle types with it. Rejected: user-defined destructors, built-in types without declarations, forgeable library structs.
- **`remember` keys are rule-typed** (D12): up to three keys, each a fresh type parameter bounded by `Copy`, so the existing `E4021` and `E4022` checks apply, and a key written as a reference is copied out like any argument of a type parameter.

Decisions from `state-handles` (planned autonomously on 2026-10-18, reconciled with the archived compose changes and implemented on 2026-10-19, refined at review, and logged in `docs/decisions-for-review.md`):

- **The state declarations** (D2). `State<T>` and `MutableState<T>` are `public intrinsic struct ... : Copy`, `mutableStateOf`, `read`, `update`, `edit` and the composable `SideEffect` are intrinsic functions of `/prism/compose/runtime`, after Jetpack's names; `read`, `update` and `edit` take a shared receiver, so a handler's captured `val` handle calls them. The catalog types `value` by its own rule (a copy, a place or an assignment target, compose D5) rather than as a computed property.
- **`intrinsic var`** (D4). `MutableState<T>.value` is assigned, so the property form gains `intrinsic var`, checked against the catalog's read-only or writable flag (`E4029` for a mismatch) and recorded in the ABI, as Kotlin declares `val value` on `State` and `var value` on `MutableState`. The writability of an assignment comes from the declaration found. Rejected: one `intrinsic val` whose writability depends on the receiver (the ABI would not say which handle is writable), and ordinary extension properties (a follow-up).

Decisions from `ui-toolkit` (planned on 2026-10-18 and revised on 2026-10-19 by user direction to follow Jetpack Compose's APIs and modules, implemented and reviewed on 2026-10-19, and logged in `docs/decisions-for-review.md`):

- **The toolkit modules** (D21, user direction). `/prism/compose/ui`, `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3` are files of the package directory `prism/compose/` beside `/prism/compose/runtime`, imported explicitly, importing each other with sibling imports (`import ui`) in one direction, as Compose's artifacts depend: `ui` on nothing of the toolkit, `foundation` and `layout` on `ui`, `material3` on all three. A program that imports none pays nothing; one importing `material3` loads four cached ABIs. Rejected: one `/prism/ui`, toolkit modules in the prelude.
- **The UI intrinsics** (D3, D7, D15, D23). `NodeId` (`package`) and `HandlerId` (public, whose catalog equality entry is `prism_ui_handlers_equal`) are one-word intrinsic structs; `Node` and `LeafNode` are `package` intrinsic composables; `runApp` is an intrinsic so that `E4030` names it and its call builds a root content adapter, its defaults being ordinary generators in `/prism/compose/ui` that `BuiltinFunction::declaration_with_defaults` names; `Int.dp` and `Int.sp` are intrinsic properties lowered as constructions. Setters are `package` externals, so only the toolkit's files reach them. `ModifierElement` and `HandlerId` stay public because programs read chains, until fields have visibility.

Decisions from `owned-state` (review items A2′, A2‴ and L5 of A4, agreed with the user on 2026-10-20 in `docs/milestone-2-review-notes.md`: `MutableState<T>` an owning state object as in Compose, no stale-state errors in UI code, weak capture declared by the library; the rest decided while proposing, implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **The library declares how a type is captured: `capture fun`** (D12, L5). `public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>` (and the same for `State<T>`) marks the function attached closures use to capture a value of the receiver's type; the type checker builds a table keyed by the receiver's type constructor from each module's declarations, so the capture pass never tests a type's name. The catalog checks the shape (`E4029`): one per type, a shared receiver, no parameters, no type parameters but the receiver's, a `WeakReference` result, in the type's module. Rejected: testing `IntrinsicStruct::MutableState` (what L5 forbids), a marker on the struct (markers take no arguments), an annotation (no attribute syntax).

Decisions from `explicit-library` (review item A4, agreed with the user on 2026-10-20 in `docs/milestone-2-review-notes.md`: the library's sources in a visible top-level directory, imported from any workspace as `/prism/...`, explicit imports everywhere and no prelude, to be revisited after seeing the result, built-in types and their members everywhere; the rest decided while proposing, implementing and reviewing the change, and logged in `docs/decisions-for-review.md`):

- **No prelude** (D1, user direction). Every library function, extension, struct, enum and value is seen only through an import of its module, which behaves as a workspace module's import. The examples gained 14 library imports over 8 of their 15 files, and 635 of about 2,300 fixture cases gained imports, most of them `import /prism/core` for `println`: the evidence for the user's review. A file pays for each library module it imports by ingesting its ABI, about 35 us for `/prism/collections`, where the prelude shared one conversion between files; a file importing nothing pays less than before. Sharing converted ABIs between files is a follow-up for every import. Rejected: a prelude of a few functions, an umbrella `prism/prelude` module (Prism has no re-exports).
- **Built-in types and their members are everywhere, through the member modules** (D2, user direction for the rule). The ten built-in types need no import, nor do their members: the compiler's (`clone()`, `unbox()`, indexing, operators, `for`) and the intrinsic extensions of the member modules `/prism/core` (scalars, `String`) and `/prism/collections` (`List`). Those two modules stay implicit inputs of every workspace file, contributing only a shared `MemberScope` of about twenty entries; their other names stay out of scope. The helpers, `Int.dp` and `Int.sp` are extensions, imported like Compose's `dp`. Rejected: members typed from the catalog with no module loaded (`library-in-prism` writes members with bodies), every intrinsic extension a member everywhere (no program could declare its own `dp`), and helpers as members.
- **Members of library types come with their module** (D3). `Composition.recompose` or `MutableState.value` is a member where the runtime module is imported; a value received through another module's signature without the import gets `E4014`, which names the import. Rejected: members of every library type found everywhere (a prelude for members).
- **Imported library names follow the rules of imports** (D4). A file's own declaration shadows an imported library name, as for every import (user decision of `shadowing-and-import-paths`; the packet first said `E4010`); two imports exporting a name or extension are `E4012` at use; a file that does not import a module may declare any of its names. Adding a library function can now change only files that import its module and the same name from elsewhere, which see `E4012`.
- **The sources live in `library/`, embedded from there** (D5, user direction for the location). The module `prism/<name>` is the file `library/prism/<name>.prism`, laid out as a workspace whose root holds the package `prism`; `crates/prism-library` embeds them with `include_str!`, rustc's dep-info rebuilds it when one changes, nothing is read from disk at run time, and the compiler identity, which hashes the executable, covers them, so a library edit invalidates the build cache. Rejected: a path relative to the executable (an install could pair one compiler with another library, and the identity would have to hash a directory), a build script, moving the crate into `library/`, and `PRISM_LIBRARY_DIR`. A published `prism-library` would need a build script copying `library/` in (a follow-up).
- **The library is not checked in place** (D6). `prism` stays a reserved root package, so `prism check library/prism/core.prism` is `E1008`: a copy of the sources is not the library, and `/prism/core` means the same code in every workspace. `library_tests.rs` checks the embedded modules. Rejected: exempting a directory named `library` (a marker by name) or files identical to an embedded module. `prism check --library` for library authors is a follow-up.
- **Library files still see nothing implicitly** (D7), the member modules included, so the library's own dependency order stays explicit.
- **Import help from an index built on demand** (D8). An unresolved name (`E4001`), member (`E4006`) or annotation's type (`E4001`, D15) that a library module declares publicly gets one help line per module: `'println' is declared by the library module '/prism/core': add 'import /prism/core'`, quoted with `'...'` like every help line. The index is built from the program's `Library` (injected test modules included) by parsing every module once per process, on the first diagnostic that needs it; functions, values, types and extensions (by receiver constructor, or any for a type parameter) are indexed apart, and an extension matches only a library type or a built-in one. The type checker stays free of library knowledge. Rejected: a static table checked by a test (two places to edit), a generated table, ingesting every library ABI (a prelude by another name), and suggesting workspace modules (an LSP follow-up).
- **Scenarios omit library imports** (D12). A scenario showing no import is read with the imports of the library modules it uses, which its fixture declares; scenarios about imports show them.
- **Module names unchanged** (D13). `print` stays in `/prism/core`; splitting the core is `library-in-prism`'s decision.
- **A library type named without its import is `E4001`** (D15, at review). Unknown type names stay opaque (type-system), so `fun f(s: MutableState<Int>)` without the runtime import typechecked until a member use, and six fixtures passed after the migration only for that reason. The checker records the annotation names that lower to an opaque type, and the driver reports those the library declares as a public struct or enum as `E4001` (`UnresolvedTypeName`) with the import help, dropping the later `E4006` on that value; other unknown names stay opaque. Cost: none unless a file names an unknown type. Rejected: rejecting every unknown name now (it breaks the `E6001` scenarios, which reach the backend through unknown types; the ROADMAP item stays), and giving the checker the library's type names (the index on every program's success path).

Decisions from `operator-conventions` (planned autonomously on 2026-10-20 for part L1 of the review's agreed item A4, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`; the conventions themselves are in `operators`):

- **Intrinsic operators** (D8). The catalog records whether an intrinsic is an operator, and a declaration whose `operator` flag differs is `E4029`; `List<T>.contains` is the only one, serving `in` and `!in`.

Decisions from `vararg` (part L4 of the review's agreed item A4, user direction 2026-10-20; planned autonomously on 2026-10-20, refined while implementing and reviewing the change, and logged in `docs/decisions-for-review.md`; the rules themselves are in `vararg-parameters`):

- **`listOf` leaves the catalog** (D10, D11). `/prism/collections` declares `listOf` in Prism with a `vararg`, so its typing rule and catalog entry are gone. `print`, `println`, `remember` and `key` stay rule-typed: they take arguments of different types (Kotlin's `Any?`), which a homogeneous vararg cannot type.
