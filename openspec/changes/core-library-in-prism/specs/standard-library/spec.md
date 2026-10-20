## ADDED Requirements

### Requirement: Members Written in Prism
The members of the built-in types that `/prism/core` writes in Prism MUST behave as the built-ins they replace, and a use of one MUST be a call of its declaration, never a built-in call:
- `toString()` of an `Int`, `Double`, `Bool` or `Char` gives the text `print` writes, through the runtime's `prism_rt_*_to_string` entries, declared as private externals;
- `isEmpty()` and `isNotEmpty()` compare `size` with `0`;
- `indices` is `0..<size`.

#### Scenario: Members written in Prism run
- **WHEN** `fun main() { val n: Int? = -3; println(n?.toString() ?: ""); println(1.5.toString() + true.toString() + 'c'.toString()); val xs = listOf(4, 5); println(xs.isEmpty()); println(xs.isNotEmpty()); for (i in xs.indices) print(i); println() }` runs with `--check-allocations`
- **THEN** stdout is `-3`, `1.5truec`, `false`, `true` and `01`, one per line, and the exit status is 0

#### Scenario: Members written in Prism are calls of their declarations
- **WHEN** `prism dump-typed-tree` runs on `fun f(n: Int, xs: &List<Int>): Bool = n.toString().length > 0 && xs.isEmpty() && xs.indices.start == 0`
- **THEN** the body calls `prism/core::Int.toString` with a shared receiver borrow of the binding `n`, `prism/core::List.isEmpty` and `prism/core::List.indices`, each with the type argument `Int` and a shared receiver borrow of the dereferenced binding `xs`, and no `builtin toString`, `builtin isEmpty` or `builtin indices`

### Requirement: Unit Properties Are Written in Prism
`/prism/compose/ui` MUST declare `public val Int.dp: Dp get() = Dp(this)` and `public val Int.sp: TextUnit get() = TextUnit(this)`, ordinary extension properties written in Prism and used where that module is imported. A file's own extension property `dp` or `sp` on `Int` MUST shadow them, as for any imported extension.

#### Scenario: A file's own unit property shadows the import
- **WHEN** a file declares `import /prism/compose/ui`, `import /prism/core`, `val Int.dp: Int get() = this * 2` and `fun main() { println(3.dp); println(4.sp.value) }`, and runs with `--check-allocations`
- **THEN** stdout is `6` then `4`, and the exit status is 0

## MODIFIED Requirements

### Requirement: Library Modules Compile Like Workspace Files
A library module MUST go through every stage a workspace file does: Phase 1 extracts its ABI, Phase 2 checks its bodies, its generics are instantiated per program, and its code is a unit of its own, cached like a workspace file's. A program MUST parse only the library modules it reaches: the member module `/prism/core`, and the modules imported, directly or transitively.

#### Scenario: Unreached modules are not parsed
- **WHEN** `prism check --explain-cache main.prism` runs on a file without imports
- **THEN** its file lines name `prism/core.prism` and `main.prism`, and no other library module

#### Scenario: An imported module is parsed
- **WHEN** `prism check --explain-cache main.prism` runs on a file that declares `import /prism/compose/runtime`
- **THEN** its file lines name `prism/core.prism`, `prism/compose/runtime.prism` and `main.prism`, and no other library module

### Requirement: Intrinsic Extensions Are Members
An intrinsic extension function or property, and every extension function or property that `/prism/core` declares on a built-in type, MUST be a member of its receiver type wherever it is in scope: in every workspace file for those of `/prism/core`, and where its module is imported for any other. A call or read of that name on that type, through `&`, `&mut` and `Heap<T>`, MUST use it before any extension, after fields and the built-in `clone()` and `unbox()`. A workspace extension of that name on that receiver type constructor, where the member is in scope, MUST be `E4010`.

#### Scenario: A workspace extension named like a member
- **WHEN** a file declares `fun Int.toString(): String = "x"`, `fun String.length(): Int = 0`, `fun <T> List<T>.add(element: T) {}` and `fun <T> List<T>.isNotEmpty(): Bool = false`
- **THEN** the type checker emits `E4010` for each, saying that the member of `/prism/core` always takes precedence

#### Scenario: A workspace extension named like a helper
- **WHEN** a file declares `fun <T> List<T>.map(f: (&T) -> Int): Int = 0` and `fun f(xs: &List<Int>): Int = xs.map { x -> x }`
- **THEN** the file typechecks, and the call resolves to the file's own extension

### Requirement: The Core Module
`/prism/core` MUST declare `print` and `println` as public intrinsics, and every member of the built-in types:
- `toString()` of `Int`, `Double`, `Bool` and `Char`, written in Prism;
- the intrinsic property `val String.length: Int`;
- the members of `List<T>` (collections, "List Members Are Typed as Extensions", "Size and Indices Are Read Without Parentheses").

Every extension it declares MUST have a built-in receiver type. Their behavior is that of the type-system, collections and execution requirements for them.

#### Scenario: Core names need no import
- **WHEN** a file without imports declares `fun f(): Int { val s = 42.toString(); return s.length }`
- **THEN** it typechecks: `toString()` and `length` are members of built-in types

#### Scenario: Core functions are imported
- **WHEN** `import /prism/core` and `fun main() { val s = 42.toString(); println(s.length) }` run, and the same `main` is checked without the import
- **THEN** the first prints `2`, and the second is rejected with `E4001` for `println` alone

#### Scenario: Declarations of the core module
- **WHEN** `prism abi` prints the ABI of `/prism/core`
- **THEN** it holds `public fun Int.toString(): String`, `public intrinsic val String.length: Int`, `public intrinsic val <T> List<T>.size: Int`, `public fun <T> List<T>.isEmpty(): Bool`, `public val <T> List<T>.indices: Range` and `public intrinsic mut fun <T> List<T>.add(element: T)`, and no external

### Requirement: Built-In Types and Their Members Need No Import
The built-in types `Int`, `Double`, `Bool`, `Char`, `String`, `Unit`, `Nothing`, `List`, `Heap` and `Range`, and their members, MUST be in scope in every workspace file without an import. A built-in type's members MUST be the compiler's built-in members (`clone()`, `unbox()`) and the extensions that the member module `/prism/core` declares on it, intrinsic or written in Prism.

#### Scenario: Members need no import
- **WHEN** a file without imports declares `fun f(xs: &mut List<Int>, s: &String): Int { xs.add(1); if (xs.isEmpty()) return 0; return xs.size + s.length + 42.toString().length + xs.indexOf(1) + xs.indices.end }`
- **THEN** the file typechecks

#### Scenario: An extension of a built-in type from another module is imported
- **WHEN** a file without imports declares `fun f(): Int { val d = 16.dp; return 0 }`
- **THEN** the type checker emits `E4006` for `dp`, with help naming `import /prism/compose/ui`

### Requirement: The Collections Module Declares Lists and Their Helpers
`/prism/collections` MUST declare `listOf` (collections, "Lists Are Built With listOf") and the higher-order helpers (collections, "Higher-Order Helpers") as public Prism functions. It MUST declare no member of `List<T>`: `/prism/core` declares those, and `/prism/collections` imports `/prism/core` to use them. It is an ordinary module, which a file uses only through an import.

#### Scenario: Helpers are imported
- **WHEN** `import /prism/collections`, `import /prism/core` and `fun main() { val xs = listOf(1, 2, 3); println(xs.map { x -> x * 2 }.sumOf { x -> x }) }` run
- **THEN** it prints `12`

#### Scenario: listOf is written in Prism
- **WHEN** `cargo test -p prism-cli --test library_tests` checks the library, and `prism dump-typed-tree` runs on a file declaring `import /prism/collections` and `fun f(): List<Int> = listOf(1, 2)`
- **THEN** `library/prism/collections.prism` declares `public fun <T> listOf(vararg elements: T): List<T> = elements` and passes its checks, the intrinsic catalog has no `listOf`, and the body of `f` is a call of `prism/collections::listOf<Int>` whose argument is a call of `builtin varargList<Int>` with the literals `1` and `2`

### Requirement: Intrinsic Calls Are Typed by Their Signatures
A call of an intrinsic MUST be checked as a call of its declared signature, except for those the catalog types by a rule:
- `print` and `println` (`fun <T> print(value: &T)`) take an argument with a text form (type-system, "Output Built-In Arguments"), and `println` also none;
- `contains` and `indexOf` of `List<T>` also need `==` on `T`;
- `remember` also takes keys ("Remember Takes Up to Three Keys").

#### Scenario: A signature-typed intrinsic
- **WHEN** `fun f(xs: &mut List<Int>): Int = xs.removeLast(1)` is checked
- **THEN** the type checker emits `E4004`, since `removeLast` is declared with no parameter

#### Scenario: A rule-typed intrinsic
- **WHEN** `fun f() { println(); println(1) }` is checked
- **THEN** the file typechecks, although `println` is declared with one parameter

### Requirement: Intrinsic Operators
An `intrinsic` declaration MAY carry `operator` when the catalog lists the intrinsic as an operator: `List<T>.contains` of `/prism/core` is declared `public intrinsic operator fun <T> List<T>.contains(element: &T): Bool`. A declaration whose `operator` flag differs from the catalog's MUST be rejected with `E4029`, as any other signature difference.

#### Scenario: The list membership operator
- **WHEN** a file declares `fun f(xs: &List<Int>): Bool = 3 in xs`
- **THEN** the file typechecks, `in` calling the intrinsic member `contains` of `List`

#### Scenario: An intrinsic's operator flag must match the catalog
- **WHEN** a library test module declares `public intrinsic fun <T> List<T>.contains(element: &T): Bool` without `operator`, or `public intrinsic operator fun <T> List<T>.indexOf(element: &T): Int`
- **THEN** the type checker emits `E4029` for each, showing the signature the compiler provides

### Requirement: The UI Intrinsics
The intrinsic catalog MUST hold, as intrinsics of `/prism/compose/ui`:
- the one-word structs `NodeId : Copy`, with `package` visibility, and `HandlerId : Copy`, public, whose equality is the runtime entry `prism_ui_handlers_equal`;
- the `package` composables `Node(kind: Int, update: (NodeId) -> Unit, content: compose () -> Unit)` and `LeafNode(kind: Int, update: (NodeId) -> Unit)`;
- the function `runApp`.

A declaration of one of them with another signature or in another module MUST be `E4029`.

#### Scenario: The public signature of runApp
- **WHEN** `prism abi` prints the ABI of `/prism/compose/ui`
- **THEN** it holds the public intrinsic function `runApp` with the parameters `title: &String`, `width: Int` and `height: Int`, each with a default, and `content: escaping compose () -> Unit`, the public intrinsic struct `HandlerId : Copy`, and the public extension properties `val Int.dp: Dp` and `val Int.sp: TextUnit`, and none of `NodeId`, `Node` and `LeafNode`, which are not public

#### Scenario: A UI intrinsic in another library module
- **WHEN** a library test module declares `private intrinsic compose fun LeafNode(kind: Int, update: (Int) -> Unit)`
- **THEN** the type checker emits `E4029`, saying that the compiler provides `LeafNode` only in `/prism/compose/ui`

#### Scenario: Calling runApp with its defaults
- **WHEN** `fun main() { runApp { } }` runs with `import /prism/compose/ui` under `prism run --headless` with the script `dump`
- **THEN** stdout is `window 800x600`: `width` and `height` took their defaults

## REMOVED Requirements

### Requirement: The Unit Properties
**Reason**: `Int.dp` and `Int.sp` are no longer intrinsic members of `Int`: they are ordinary extension properties of `/prism/compose/ui` written in Prism ("Unit Properties Are Written in Prism"), which a file's own declaration shadows instead of being `E4010`.
**Migration**: The scenario "Unit properties are members of Int" and its fixture are replaced by "A file's own unit property shadows the import".
