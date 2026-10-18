# Standard Library Specification

## Purpose
The standard library is the Prism source the compiler ships under the reserved root package `prism`: which modules exist, which of their names every file sees without an import, the `external` and `intrinsic` declaration forms that only the library may use, and how the library is checked.

## ADDED Requirements

### Requirement: The Compiler Ships the Library
The `prism` executable MUST contain the source of every library module and MUST NOT read library files from disk. The modules MUST be `/prism/core` and `/prism/collections`, the prelude modules, and `/prism/system`. Each MUST be a file of the package `prism`, or of a package below it, whose module path is `prism/<name>`.

#### Scenario: A library module is imported
- **WHEN** `app/main.prism` declares `import /prism/system` and `fun main() { println(1); exitProcess(3) }`, in a workspace with no `prism` directory
- **THEN** `prism run app/main.prism` prints `1` and exits with status 3

### Requirement: Library Modules Compile Like Workspace Files
A library module MUST go through every stage a workspace file does: Phase 1 extracts its ABI, Phase 2 checks its bodies, its generics are instantiated per program, and its code is a unit of its own, cached like a workspace file's. A program MUST parse only the library modules it reaches: the prelude modules and those imported, directly or transitively.

#### Scenario: Unreached modules are not parsed
- **WHEN** `prism check --explain-cache main.prism` runs on a file without imports
- **THEN** its file lines name `prism/core.prism`, `prism/collections.prism` and `main.prism`, and no other library module

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
- `contains` and `indexOf` of `List<T>` also need `==` on `T`.

#### Scenario: A signature-typed intrinsic
- **WHEN** `fun f(xs: &List<Int>): Bool = xs.isEmpty(1)` is checked
- **THEN** the type checker emits `E4004`, since `isEmpty` is declared with no parameter

#### Scenario: A rule-typed intrinsic
- **WHEN** `fun f() { println(); println(1); val xs = listOf(1, 2, 3) }` is checked
- **THEN** the file typechecks, although `println` and `listOf` are declared with one parameter

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
