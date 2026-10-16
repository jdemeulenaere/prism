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

## Design Rationale
- **The ABI comes from the AST, not from type checking.** Explicit types on exported declarations let Phase 1 run on every file independently and before any body is checked. This is what lets the Phase 2 type checker handle import cycles (`type-checker-core`).
- **The hash ignores spans and formatting.** Source positions, comments and whitespace are excluded, including from a generic function's body hash, so reformatting never invalidates downstream files.
- **Block-bodied exports default to `Unit` here.** ABI extraction only rejects expression-bodied exports without a return type. The type checker rejects both forms (`E4011`). Whether to tighten the ABI rule or relax the specs is an open item in `ROADMAP.md` (`resolve-nominal-type-annotations`).
- **The hash algorithm is not final.** The code uses a 128-bit FNV-1a hash. The original design called for BLAKE3, and the choice must be made before the `.abi` cache exists (`docs/architecture.md` section 1).
