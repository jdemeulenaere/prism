## ADDED Requirements

### Requirement: Library Annotations in the ABI
An exported struct annotated `@Keeper` MUST carry a keeper flag in its ABI entry and in every reachable definition of it, and an exported function annotated `@DrivesComposition` MUST carry its flag. The ABI hash MUST cover both, and `prism abi` MUST print them as the annotations. `@ExplicitGroupsComposable` MUST NOT be part of the ABI.

#### Scenario: A keeper reached without its module
- **WHEN** `app/screen.prism` imports `/prism/compose/runtime` and `/prism/core` and exports `public fun state(): MutableState<Int> = mutableStateOf(0)`, `public fun show(s: &MutableState<Int>): Composition = composition { println(s.value) }` and `public fun consume(s: MutableState<Int>): Unit {}`, and `app/main.prism` imports only `/app/screen` and declares `fun main() { val s = state(); val c = show(s); consume(s); val d = c }`
- **THEN** the borrow checker emits `E5005` at `s` in `consume(s)`: `app/main` knows from the reachable definition of `Composition` that it is a keeper, although it does not import `/prism/compose/runtime`

#### Scenario: Annotations change the ABI hash
- **WHEN** a library test module's struct `@Keeper public struct H(private val n: Int)` loses its annotation
- **THEN** the module's ABI hash changes, and `prism abi` printed `@Keeper public struct H` before the edit and `public struct H` after it

## MODIFIED Requirements

### Requirement: Intrinsic Structs in the ABI
An exported intrinsic struct MUST be an ABI entry with its name, type parameters and markers, marked intrinsic, covered by the ABI hash and shown by `prism abi` with its modifier (`public intrinsic struct WeakReference<T> : Copy`). Its layout MUST NOT be part of the ABI: it comes from the compiler, whose version every artifact key holds.

#### Scenario: The ABI of the compose module
- **WHEN** the ABI of `/prism/compose/runtime` is extracted
- **THEN** it holds `public intrinsic struct State<T>`, `public intrinsic struct WeakReference<T> : Copy` and `public intrinsic compose fun <T> remember(calculation: () -> T): T`, and removing the `Copy` marker of `WeakReference` changes the module's ABI hash
