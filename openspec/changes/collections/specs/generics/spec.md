# Generics Specification Delta

## MODIFIED Requirements

### Requirement: Non-Regular Generic Types Are Rejected
A generic struct or enum MUST NOT reach itself through its fields or payloads, `Heap<T>` and `List<T>` included, with a type argument that holds one of its own type parameters nested in another type, since its layout, drop glue and equality would then need infinitely many types. Such a field or payload MUST be `PolymorphicRecursion` (`E4027`) at the declaration. A type that `E4023` already rejects is not reported again.

#### Scenario: A generic type reaching itself through a box with a growing argument
- **WHEN** `struct Box<T>(val v: T)` and `struct Nest<T>(val v: T, val next: Heap<Nest<Box<T>>>?)` are checked
- **THEN** the type checker emits `E4027` at the field `next`, saying that `Nest` contains `Nest<Box<T>>`

#### Scenario: A generic type reaching itself through a list with a growing argument
- **WHEN** `struct Box<T>(val v: T)` and `struct Nest<T>(val v: T, val more: List<Nest<Box<T>>>)` are checked
- **THEN** the type checker emits `E4027` at the field `more`, saying that `Nest` contains `Nest<Box<T>>`

#### Scenario: A regular recursive generic type accepted
- **WHEN** `enum Chain<T> { Cons(T, Heap<Chain<T>>), Nil }`, `struct Tree<T>(val v: T, val kids: Heap<Chain<Tree<T>>>?)` and `struct Forest<T>(val v: T, val trees: List<Forest<T>>)` are checked
- **THEN** the file passes `prism check`
