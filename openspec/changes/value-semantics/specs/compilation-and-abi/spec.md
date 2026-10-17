# Spec Delta

## ADDED Requirements

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
