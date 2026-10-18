# Compilation and ABI Specification Delta

## ADDED Requirements

### Requirement: Definitions of Reachable Types in the ABI
Each file's ABI MUST record the definition (kind, type parameters, marker, and fields or variants with their bound types) of every struct and enum that a value reached through its exports can hold, transitively through fields and payloads, other than its own exported structs and enums. Private types and types of modules the file does not import MUST be included when reached; each definition MUST come from its defining file's declarations alone. A definition an importer knows only this way MUST serve layout alone: its fields and variants MUST stay unknown to the importer's type checker, as if the definition were absent.

#### Scenario: A private field type is recorded
- **WHEN** `lib/a.prism` declares `struct Secret : Copy { val k: Int }`, `public struct Outer : Copy { val s: Secret, val n: Int }` and the private `struct Helper { val h: Int }`, and `prism abi lib/a.prism` runs
- **THEN** it lists `Outer` among the exported symbols, and among the reachable types the definition of `Secret` from module `lib/a` with its field `k: Int`, and does not list `Helper`

#### Scenario: A type of a module the importer does not import
- **WHEN** `geo/core.prism` exports `public struct Vec : Copy { val x: Int }`, and `geo/shapes.prism` imports `/geo/core` and exports `public struct Circle : Copy { val center: Vec, val r: Int }`
- **THEN** the ABI of `geo/shapes` records the definition of `Vec` from module `geo/core`, so a file importing only `/geo/shapes` can lay out a `Circle`

#### Scenario: A reachable private type keeps its members hidden
- **WHEN** `lib/a.prism` declares `struct Secret : Copy { val k: Int }`, `enum Mode : Copy { On, Off }`, `public struct Outer : Copy { val s: Secret, val m: Mode }` and `public fun make(): Outer = Outer(Secret(1), Mode.On)`, and `app/main.prism` imports `/lib/a` and prints `make().s.k`, or prints `when (make().m) { On -> 1; else -> 2 }`
- **THEN** `prism check app/main.prism` reports `E4006` for the field `k`, or `E4001` for the variant name `On`, as it does for any type whose definition it does not know

#### Scenario: Changing a reachable type changes the hashes
- **WHEN** in that workspace `Vec` gains a field `val y: Int`, or the private `Secret` of `lib/a` gains a field
- **THEN** the interface and ABI hashes of `geo/core` and `geo/shapes` change, or those of `lib/a`, while changing the private `Helper` changes no hash

### Requirement: Reachable Type Changes Re-Check Importers
A change to a struct or enum MUST re-check every file whose direct import's ABI records that type, as a symbol or as a reachable definition, through the existing interface-hash keys of the build cache, and no other file because of it.

#### Scenario: A field added to a transitively reached type
- **WHEN** `geo/core.prism` also exports `public fun origin(): Vec = Vec(0)`, `geo/shapes.prism` exports `public fun unit(): Circle = Circle(origin(), 1)`, `app/main.prism` imports only `/geo/shapes` and prints `unit().r`, all three have been checked, and then only `geo/core.prism` changes: `Vec` gains a field `val y: Int` and `origin` returns `Vec(0, 0)`
- **THEN** the next `prism check --explain-cache app/main.prism` prints `  checked geo/core.prism`, `  checked geo/shapes.prism` and `  checked app/main.prism`, and `prism run app/main.prism` prints `1` before and after the change

#### Scenario: A body change in the defining file
- **WHEN** in that workspace only the body of a function of `geo/core` changes
- **THEN** the next `prism check --explain-cache app/main.prism` checks `geo/core` again and reuses `geo/shapes` and `app/main`

## MODIFIED Requirements

### Requirement: Phase 1 Exported Surface Extraction
The compiler MUST extract each source file's exported ABI surface (`public` and `package` structs, enums and function signatures) from its AST without typechecking function bodies.

#### Scenario: Phase 1 surface extraction
- **WHEN** the compiler extracts the ABI of a source file
- **THEN** it records the file's package and the signatures of its `public` and `package` structs, enums and functions, without typechecking function bodies

#### Scenario: Private symbols excluded from the ABI
- **WHEN** a source file declares functions, structs or enums without a visibility modifier
- **THEN** those declarations are absent from the extracted ABI's exported symbols, and a private struct or enum is recorded only as a reachable type definition, when an export reaches it

#### Scenario: Exported function without explicit return type rejected
- **WHEN** a `public` or `package` function omits its return type (`public fun add(a: Int, b: Int) = a + b`)
- **THEN** ABI extraction fails with a missing explicit return type error naming the function and its visibility

### Requirement: Deterministic ABI Hash
The compiler MUST compute a deterministic 256-bit BLAKE3 hash of each file's exported ABI that covers exported signatures, enum variant shapes, default parameter values and the definitions of reachable types, and excludes every other private declaration, non-generic function bodies and source positions.

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

#### Scenario: Hashes printed in full
- **WHEN** a developer runs `prism abi` on a source file, or `prism check` on a workspace
- **THEN** each ABI hash is printed as `0x` followed by 64 lowercase hexadecimal digits
