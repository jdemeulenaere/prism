# Prism Compilation Pipeline & ABI Extraction

## Purpose
The compilation and ABI specification defines Phase 1 of the two-phase pipeline as implemented in `crates/prism-abi`: extracting each file's exported declaration surface and computing a deterministic ABI hash that ignores private symbols and non-generic function bodies. Code generation, the `.abi` cache, compilation avoidance and the execution tiers are planned and described in [`docs/architecture.md`](../../../docs/architecture.md).

## Requirements

### Requirement: Phase 1 Exported Surface Extraction
The compiler MUST extract each source file's exported ABI surface (`public` and `package` structs, enums and function signatures) from its AST without typechecking function bodies.

#### Scenario: Phase 1 surface extraction
- **WHEN** the compiler extracts the ABI of a source file
- **THEN** it records the file's package and the signatures of its `public` and `package` structs, enums and functions, without typechecking function bodies

#### Scenario: Private symbols excluded from the ABI
- **WHEN** a source file declares functions, structs or enums without a visibility modifier
- **THEN** those declarations are absent from the extracted ABI

#### Scenario: Exported function without explicit return type rejected
- **WHEN** a `public` or `package` function omits its return type (`public fun add(a: Int, b: Int) = a + b`)
- **THEN** ABI extraction fails with a missing explicit return type error naming the function and its visibility

### Requirement: Deterministic ABI Hash
The compiler MUST compute a deterministic 128-bit hash of each file's exported ABI that covers exported signatures, enum variant shapes and default parameter values, and excludes private symbols, non-generic function bodies and source positions.

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

### Requirement: Monomorphization Hash for Generics
Exported generic functions MUST record a Monomorphization Hash of their body in their ABI signature, computed independently of source positions, comments and whitespace.

#### Scenario: Changing generic function body
- **WHEN** the body of an exported generic function changes
- **THEN** its Monomorphization Hash and the file's ABI hash change

#### Scenario: Generic body hash ignores formatting
- **WHEN** comments or blank lines are added to an exported generic function without changing its code
- **THEN** its Monomorphization Hash and the file's ABI hash remain unchanged

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

## Design Rationale
- **The ABI comes from the AST and the direct imports' exported type names, never from bodies.** Explicit types on exported declarations let Phase 1 run before any body is checked (`type-checker-core`). Phase 1a reads each file's exported struct and enum names from its AST alone. Phase 1b then binds each nominal name in an exported signature to the file's own declarations or to the one direct import exporting it, reading only those 1a outputs, so import cycles still work and no body is read (`typechecker-soundness`, D4).
- **ABI types are structured trees, not strings** (`typechecker-soundness`, D4). Importers once re-parsed `Display` strings, which split higher-order types wrongly and resolved names in the importer's scope. A tree recorded once is exact, hashing it structurally (a tag byte per node) covers structure rather than formatting, and each bound nominal type records whether it is a struct or an enum, so an importer can type a value of a type whose module it does not import. A balanced-parenthesis string parser was rejected: two parsers can drift, and names would still bind in the wrong scope. The module path is part of the hash, because it is part of every exported type's identity.
- **The hash ignores spans and formatting.** Source positions, comments and whitespace are excluded, including from a generic function's body hash, so reformatting never invalidates downstream files.
- **Block-bodied exports default to `Unit` here.** ABI extraction only rejects expression-bodied exports without a return type. The type checker rejects both forms (`E4011`). Whether to tighten the ABI rule or relax the specs is an open item in `ROADMAP.md` (`resolve-nominal-type-annotations`).
- **The hash algorithm is not final.** The code uses a 128-bit FNV-1a hash. The original design called for BLAKE3, and the choice must be made before the `.abi` cache exists (`docs/architecture.md` section 1).
- **Markers on declarations and on every reference** (`value-semantics`, decided by the user). Each exported struct and enum records its marker (`Copy`, `Clone` or none), and so does each nominal reference in an exported signature, so an importer knows how values of a type may be duplicated without importing its module. Phase 1a reads markers from the AST alone, so no body is read and import cycles still work. Changing a type's marker therefore changes the ABI hash of every file whose exported signatures mention it, which is correct, because what importers may do with those values changes. Requiring the import (`E4014`) whenever Copy-ness is needed was rejected because an import would change whether unrelated code compiles, and treating an unknown marker as "none" because it gives false errors.
- **The built-in `Range` has its own ABI node** (`value-semantics`). It was recorded as an unbound name and came back to importers as an opaque type, which would have made an imported `Range` field move-only. A user-declared or imported `Range` still binds first.
- **An extension is a function record with a receiver** (`extension-methods`). `ExportedSymbol::Function` carries an optional receiver mode and type, the type bound like any signature type, and the hash covers a presence byte, the mode and the type. The sort key adds the receiver's module-qualified type text after the name, so `describe`, `User.describe` and `Team.describe` hash the same in any order. Struct and enum records never depend on extensions, so an extension in another file never changes the receiver type's file hash. Rejected: a separate `Extension` symbol kind, which would duplicate the parameter and generic-body handling, and attaching extensions to their receiver's record, which would make every extension change the receiver's ABI. Elision rule 2 needs nothing more, since it reads the receiver mode.
