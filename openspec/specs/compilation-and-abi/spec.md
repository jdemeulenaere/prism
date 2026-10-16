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

## Design Rationale
- **The ABI comes from the AST and the direct imports' exported type names, never from bodies.** Explicit types on exported declarations let Phase 1 run before any body is checked (`type-checker-core`). Phase 1a reads each file's exported struct and enum names from its AST alone. Phase 1b then binds each nominal name in an exported signature to the file's own declarations or to the one direct import exporting it, reading only those 1a outputs, so import cycles still work and no body is read (`typechecker-soundness`, D4).
- **ABI types are structured trees, not strings** (`typechecker-soundness`, D4). Importers once re-parsed `Display` strings, which split higher-order types wrongly and resolved names in the importer's scope. A tree recorded once is exact, hashing it structurally (a tag byte per node) covers structure rather than formatting, and each bound nominal type records whether it is a struct or an enum, so an importer can type a value of a type whose module it does not import. A balanced-parenthesis string parser was rejected: two parsers can drift, and names would still bind in the wrong scope. The module path is part of the hash, because it is part of every exported type's identity.
- **The hash ignores spans and formatting.** Source positions, comments and whitespace are excluded, including from a generic function's body hash, so reformatting never invalidates downstream files.
- **Block-bodied exports default to `Unit` here.** ABI extraction only rejects expression-bodied exports without a return type. The type checker rejects both forms (`E4011`). Whether to tighten the ABI rule or relax the specs is an open item in `ROADMAP.md` (`resolve-nominal-type-annotations`).
- **The hash algorithm is not final.** The code uses a 128-bit FNV-1a hash. The original design called for BLAKE3, and the choice must be made before the `.abi` cache exists (`docs/architecture.md` section 1).
