# Spec Delta: compilation-and-abi

## ADDED Requirements

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
