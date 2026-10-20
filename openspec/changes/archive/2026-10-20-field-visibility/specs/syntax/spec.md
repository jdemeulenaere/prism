## ADDED Requirements

### Requirement: Field Visibility Modifiers
A struct field MAY start with one visibility modifier, `private`, `package` or `public`, followed by `val` or `var`, in block structs and in shorthand structs (`struct Session(private val id: Int)`). A field without a modifier MUST parse as before, with no recorded visibility.

#### Scenario: Field modifiers in both struct forms
- **WHEN** parsing `struct Account { private var balance: Int, package val bank: String, public val owner: String, val note: String }` and `struct Session(private val id: Int, val name: String)`
- **THEN** both declarations parse, with `balance` private and mutable, `bank` package, `owner` public, `id` private, and `note` and `name` without a modifier

### Requirement: Misplaced Field Modifiers Are Rejected
Two visibility modifiers on one field, a modifier written twice, a modifier not followed by `val` or `var`, and a modifier on an enum variant's payload MUST be rejected with `E2001`.

#### Scenario: Misplaced field modifiers rejected
- **WHEN** parsing `struct A { private private val x: Int }`, `struct B { private public val x: Int }`, `struct C(private x: Int)` and `enum E { X(private Int) }`
- **THEN** the parser reports `E2001` for each, saying that a field takes one visibility modifier, followed by `val` or `var`, and that enum payloads take none
