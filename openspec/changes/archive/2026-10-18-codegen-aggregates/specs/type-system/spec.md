# Type System Specification Delta

## ADDED Requirements

### Requirement: Infinitely Sized Types Are Rejected
A struct or enum MUST NOT contain itself by value: through its fields or payloads, through nullables, and through other structs and enums, with type arguments substituted, but not through `Heap<T>`, references or function types. Each declaration of the file on such a cycle MUST be reported with `E4023` (`InfinitelySizedType`) at its name, with help suggesting a `Heap<...>` field.

#### Scenario: An enum containing itself
- **WHEN** `enum E { X(E), Y }` is checked
- **THEN** the type checker emits `E4023` at `E`

#### Scenario: A cycle through a nullable and another struct
- **WHEN** `struct A { val b: B? }` and `struct B { val a: A }` are checked
- **THEN** the type checker emits `E4023` at `A` and at `B`

#### Scenario: A cycle through a generic instance
- **WHEN** `struct Box<T>(val v: T)` and `struct A { val b: Box<A> }` are checked
- **THEN** the type checker emits `E4023` at `A`, and not at `Box`

#### Scenario: Heap breaks the cycle
- **WHEN** `enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }`, `struct Node { val next: Heap<Node>? }` and `struct Pair<A, B>(val a: A, val b: B)` with `struct Q { val p: Pair<Pair<Int, Int>, Int> }` are checked
- **THEN** the file typechecks

#### Scenario: A cycle across modules
- **WHEN** `a/one.prism` declares `public struct One { val two: Two? }` and imports `/b/two`, which declares `public struct Two { val one: One }` and imports `/a/one`
- **THEN** checking `a/one.prism` reports `E4023` at `One` and checking `b/two.prism` reports it at `Two`, each file seeing the other's definition through its ABI, and `prism check`, which stops at the first file with errors, reports the first of them
