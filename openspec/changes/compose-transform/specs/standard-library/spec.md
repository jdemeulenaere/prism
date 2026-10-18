# Standard Library Delta

## ADDED Requirements

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

## MODIFIED Requirements

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

### Requirement: Intrinsic Calls Are Typed by Their Signatures
A call of an intrinsic MUST be checked as a call of its declared signature, except for those the catalog types by a rule:
- `print` and `println` (`fun <T> print(value: &T)`) take an argument with a text form (type-system, "Output Built-In Arguments"), and `println` also none;
- `listOf` (`fun <T> listOf(element: T): List<T>`) takes any number of elements;
- `contains` and `indexOf` of `List<T>` also need `==` on `T`;
- `remember` (`fun <T : Copy> remember(calculation: () -> T): T`) also takes up to three keys before the calculation, each typed by a type parameter of its own bounded by `Copy` and borrow-free (compose, "Remembering With Keys").

#### Scenario: A signature-typed intrinsic
- **WHEN** `fun f(xs: &List<Int>): Bool = xs.isEmpty(1)` is checked
- **THEN** the type checker emits `E4004`, since `isEmpty` is declared with no parameter

#### Scenario: A rule-typed intrinsic
- **WHEN** `fun f() { println(); println(1); val xs = listOf(1, 2, 3) }` is checked
- **THEN** the file typechecks, although `println` and `listOf` are declared with one parameter

#### Scenario: Keys of remember
- **WHEN** `compose fun f(a: Int, b: Bool) { val x = remember(a, b) { 1 }; val y = remember(a, b, 'c', 1.5) { 2 } }` is checked
- **THEN** the type checker accepts the first call and reports an argument count error (`E4004`) for the second, which has four keys
