# Compilation and ABI Specification Delta

## ADDED Requirements

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

## MODIFIED Requirements

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
