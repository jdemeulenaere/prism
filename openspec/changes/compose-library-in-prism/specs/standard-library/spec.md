## ADDED Requirements

### Requirement: Library Annotations
The library MAY use three annotations: `@ExplicitGroupsComposable` on a `compose fun` (compose, "Composables With Explicit Groups"), `@Keeper` on a struct, and `@DrivesComposition` on a function that is not composable. A struct annotated `@Keeper` MUST be a keeper (references, "Weak Captures Keep Their States Borrowed"). Calling a `@DrivesComposition` function in a composable context MUST be `E4030`. The compiler MUST find these properties only through the annotations.

#### Scenario: A keeper declared by its annotation
- **WHEN** a library test module `/prism/test` declares `@Keeper public struct Holder(private val n: Int)` and `public fun hold(content: attached () -> Unit): Holder = Holder(1)`, and a workspace file importing it and `/prism/compose/runtime` declares `fun consume(s: MutableState<Int>) {}` and `fun main() { val s = mutableStateOf(0); val h = hold { println(s.value) }; consume(s); val k = h }`
- **THEN** the borrow checker emits `E5005` at `s` in `consume(s)`, labelling the weak capture of `s` and the later use of `h`

#### Scenario: A function that drives compositions
- **WHEN** a library test module `/prism/test` declares `@DrivesComposition public fun drive(): Unit {}`, and a workspace file importing it declares `compose fun Screen() { drive() }` and `fun later() { drive() }`
- **THEN** the type checker emits `E4030` at `drive` in `Screen`, and accepts `later`

### Requirement: Annotation Rules in the Library
In a library module, an annotation of another name, an annotation on a kind of declaration it does not apply to, an annotation written twice on one declaration, and `@DrivesComposition` on a composable MUST be rejected with `E4029`, naming the rule.

#### Scenario: Misused annotations in the library
- **WHEN** a library test module declares `@Stable public fun a(): Int = 1`, `@Keeper public fun b(): Int = 1`, `@ExplicitGroupsComposable public fun c(): Unit {}`, `@Keeper @Keeper public struct D(private val n: Int)` and `@DrivesComposition public compose fun E(): Unit {}`
- **THEN** the type checker emits `E4029` for each of the five

### Requirement: The Current Composer
`/prism/composer` MUST declare `public intrinsic compose fun currentComposer(): Int`, which gives the composer word of the body running it, the word the protocol functions take. Its call MUST add no group and make no region need one.

#### Scenario: Protocol calls inside a composable
- **WHEN** `compose fun Cell(n: Int) { val k = currentComposer(); val v = rememberInt(k, 7) { println("computed"); 42 }; println(v) }` and `fun main() { val c = composition { Cell(1) }; c.setContent { Cell(2) } }` run with `import /prism/composer`, `import /prism/compose/runtime` and `import /prism/core`, under `--check-allocations`
- **THEN** stdout is `computed`, `42` and `42`, one per line: the word slot was kept in `Cell`'s group across the passes

### Requirement: Externals Take Composable Content
An external MAY take a parameter of composable function type, passed by address and moved like any function value. `/prism/compose/runtime`'s `composition` and `setContent` and `/prism/compose/ui`'s `runApp` MUST give their content to the runtime this way, and the runtime MUST run such content as a composition's root (compose, "Root Content Is Called With the Force Bit").

#### Scenario: Content given to an external
- **WHEN** a library test module declares `private external fun prism_compose_start_composable(content: attached compose () -> Unit): Int` and `public fun start(content: attached compose () -> Unit): Int = prism_compose_start_composable(content)`, and its unit is compiled
- **THEN** the external is declared with one address parameter and a 64-bit result, which matches the runtime's entry, and `start` passes it the address of its content, moved

### Requirement: The UI Handles and Node Functions Are Written in Prism
`/prism/compose/ui` MUST declare, in Prism and with no intrinsic: `NodeId : Copy`, with `package` visibility, and `HandlerId : Copy`, public, each over a `package` word; `HandlerId`'s `equals` operator over `prism_ui_handlers_equal`; the `package` composables `Node` and `LeafNode`, annotated `@ExplicitGroupsComposable`, which start node groups through the protocol; and `runApp`, annotated `@DrivesComposition`.

#### Scenario: The public UI declarations
- **WHEN** `prism abi` prints the ABI of `/prism/compose/ui`
- **THEN** it holds `@DrivesComposition public fun runApp` with the parameters `title: &String`, `width: Int` and `height: Int`, each with a default, and `content: attached compose () -> Unit`, the public struct `HandlerId : Copy` with its `package` field `word`, and `public operator fun HandlerId.equals(other: &HandlerId): Bool`, and none of `NodeId`, `Node` and `LeafNode`, which are not public

#### Scenario: Calling runApp with its defaults
- **WHEN** `fun main() { runApp { } }` runs with `import /prism/compose/ui` under `prism run --headless` with the script `dump`
- **THEN** stdout is `window 800x600`: `width` and `height` took their defaults

#### Scenario: A handler id cannot be made by a program
- **WHEN** a file importing `/prism/compose/ui` declares `fun forge(): HandlerId = HandlerId(1)` and `fun word(h: HandlerId): Int = h.word`
- **THEN** the type checker emits `E4007` for the construction and for `word`, each naming the `package` field `word` of `prism/compose/ui`

## MODIFIED Requirements

### Requirement: Library-Only Declarations Are Rejected in the Workspace
A workspace file declaring anything `external` or `intrinsic`, or carrying an annotation, MUST be rejected in Phase 1 with `E3002` (`LibraryOnlyDeclaration`), at the modifier or the annotation's `@`, with help saying that only the bundled library may declare them.

#### Scenario: External declarations outside the bundled library rejected
- **WHEN** a workspace file declares `external fun f(): Int` or `public intrinsic fun g(): Int`
- **THEN** `prism check` reports `E3002` at `external`, and at `intrinsic`

#### Scenario: Annotations outside the bundled library rejected
- **WHEN** a workspace file declares `@Keeper struct S(val n: Int)` or `@Stable fun f(): Int = 1`
- **THEN** `prism check` reports `E3002` at the `@` of each

### Requirement: Intrinsic Structs
An `intrinsic struct` MUST declare a type whose representation, glue and operations the compiler provides from its catalog, with the markers the catalog gives it. The catalog MUST hold `State`, `MutableState` and `WeakReference` only. A library declaration naming no intrinsic struct of the catalog, or with other type parameters or markers, MUST be rejected with `E4029`. No program MUST construct a value of an intrinsic struct, read a field of one, or take one apart: only its intrinsic functions make and use its values.

#### Scenario: An unknown intrinsic struct is rejected
- **WHEN** a library test module declares `public intrinsic struct Window` or `public intrinsic struct Composition`
- **THEN** the type checker emits `E4029` for each, saying that the compiler provides no intrinsic struct of that name

#### Scenario: An intrinsic struct cannot be constructed
- **WHEN** a workspace file declares `fun f() { val s = MutableState<Int>() }` or `fun g(r: WeakReference<MutableState<Int>>) { val (h) = r }`
- **THEN** the type checker rejects each, saying that `MutableState` and `WeakReference` have no constructor and no fields

### Requirement: The Compose Module
`/prism/compose/runtime` MUST declare the composables `remember` and `key` as intrinsics, and in Prism: `Composition`, a `@Keeper` struct with a private handle and a drop hook disposing it; `composition`, `setContent` and `recompose`, annotated `@DrivesComposition`; `dump`; `RecomposeScope : Copy` over a private word, with `invalidate`; and `currentRecomposeScope`, annotated `@ExplicitGroupsComposable`.

#### Scenario: Signatures of the compose module
- **WHEN** `prism abi` prints the ABI of `/prism/compose/runtime`
- **THEN** it holds `@Keeper public struct Composition` with the private field `handle: Int`, `@DrivesComposition public fun composition(content: attached compose () -> Unit): Composition`, `@DrivesComposition public fun Composition.setContent(content: attached compose () -> Unit): Unit`, `@DrivesComposition public fun Composition.recompose(): Bool`, `public fun Composition.dump(): String`, `public intrinsic compose fun <T> remember(calculation: () -> T): T`, `public intrinsic compose fun key(value: Int, content: compose () -> Unit)`, `public struct RecomposeScope : Copy` with the private field `word: Int`, `public compose fun currentRecomposeScope(): RecomposeScope` and `public fun RecomposeScope.invalidate(): Unit`

#### Scenario: Members of a composition
- **WHEN** a file declares `import /prism/compose/runtime`, `import /prism/core`, `fun Composition.recompose(): Bool = false` and `fun main() { val c = composition { }; println(c.recompose()) }`, and runs with `--check-allocations`
- **THEN** stdout is `false`: the file's own extension shadows the imported one, as for any imported extension

### Requirement: The State Declarations of the Compose Runtime Module
`/prism/compose/runtime` MUST declare, as intrinsics, the structs `State<T>`, `MutableState<T>` and `WeakReference<T> : Copy`, the function `mutableStateOf`, the property `value` of `State<T>` and the writable property `value` of `MutableState<T>`, the member `update` of `MutableState<T>`, the capture functions `weak` of `State<T>` and of `MutableState<T>`, and the member `get` of `WeakReference<T>`; and, in Prism, the composable `SideEffect`, annotated `@ExplicitGroupsComposable`.

#### Scenario: Signatures of the state declarations
- **WHEN** `prism abi` prints the ABI of `/prism/compose/runtime`
- **THEN** it holds `public intrinsic struct State<T>`, `public intrinsic struct MutableState<T>`, `public intrinsic struct WeakReference<T> : Copy`, `public intrinsic fun <T> mutableStateOf(value: T): MutableState<T>`, `public intrinsic val <T> State<T>.value: T`, `public intrinsic var <T> MutableState<T>.value: T`, `public intrinsic fun <T> MutableState<T>.update(transform: (T) -> T)`, `public intrinsic capture fun <T> State<T>.weak(): WeakReference<State<T>>`, `public intrinsic capture fun <T> MutableState<T>.weak(): WeakReference<MutableState<T>>`, `public intrinsic fun <T> WeakReference<T>.get(): (&T)?` and `public compose fun SideEffect(effect: attached () -> Unit): Unit`

#### Scenario: State declarations are imported
- **WHEN** a file declaring `import /prism/compose/runtime` declares `compose fun Counter() { val count = remember { mutableStateOf(0) }; SideEffect { } }` and `public fun counter(): MutableState<Int> = mutableStateOf(0)`
- **THEN** it typechecks, and `prism abi` writes the signature of `counter` as `public fun counter(): MutableState<Int>`

### Requirement: Library Modules Compile Like Workspace Files
A library module MUST go through every stage a workspace file does: Phase 1 extracts its ABI, Phase 2 checks its bodies, its generics are instantiated per program, and its code is a unit of its own, cached like a workspace file's. A program MUST parse only the library modules it reaches: the member module `/prism/core`, and the modules imported, directly or transitively.

#### Scenario: Unreached modules are not parsed
- **WHEN** `prism check --explain-cache main.prism` runs on a file without imports
- **THEN** its file lines name `prism/core.prism` and `main.prism`, and no other library module

#### Scenario: An imported module is parsed
- **WHEN** `prism check --explain-cache main.prism` runs on a file that declares `import /prism/compose/runtime`
- **THEN** its file lines name `prism/core.prism`, `prism/composer.prism`, which the compose runtime module imports, `prism/compose/runtime.prism` and `main.prism`, and no other library module

### Requirement: The UI Modules Are Bundled
The compiler MUST ship `/prism/compose/ui`, `/prism/compose/foundation`, `/prism/compose/layout` and `/prism/compose/material3` as bundled modules, all files of the package `prism.compose` beside `/prism/compose/runtime`. Their files MUST be checked, formatted and compiled by the library tests like every bundled module, and their externals MUST match `prism-ui`'s and `prism-compose`'s entry points.

#### Scenario: The UI modules' externals match the entry points
- **WHEN** `library_tests.rs` checks and compiles every bundled module
- **THEN** `prism/compose/ui.prism`, `prism/compose/foundation.prism`, `prism/compose/layout.prism` and `prism/compose/material3.prism` pass `prism check`, `prism fmt --check` and the backend's check of each external, with no `E6005`

#### Scenario: The UI modules are not in the prelude
- **WHEN** `prism check --explain-cache` runs on a file without imports, and then on the same file with `import /prism/compose/material3`
- **THEN** the first lists no file of `prism/compose/` and no `prism/composer.prism`, and the second lists `prism/composer.prism`, `prism/compose/ui.prism`, `prism/compose/foundation.prism`, `prism/compose/layout.prism` and `prism/compose/material3.prism`, and not `prism/compose/runtime.prism`

## REMOVED Requirements

### Requirement: The UI Intrinsics
**Reason**: `/prism/compose/ui` declares no intrinsic any more: `NodeId`, `HandlerId`, `Node`, `LeafNode` and `runApp` are written in Prism (design D8 to D10).
**Migration**: "The UI Handles and Node Functions Are Written in Prism" replaces it. Its scenarios "The public signature of runApp" and "Calling runApp with its defaults" move there (the first as "The public UI declarations"); "A UI intrinsic in another library module" is dropped, since there is no such intrinsic, and "An unknown intrinsic is rejected" covers a declaration of one.
