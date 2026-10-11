# Prism Memory Model & Resource Management

## Purpose
The memory model specification defines Prism's compile-time memory safety architecture: zero runtime garbage collection, zero hidden automatic reference counting (ARC), single-owner value semantics, stack-confined borrows with automatic lifetime elision, and data-oriented handles.

## Requirements

### Requirement: Single-Owner Value Semantics and RAII
Every non-primitive resource MUST have a single owning binding, and resources MUST be dropped deterministically when their owner leaves lexical scope.

#### Scenario: Value move on assignment
- **WHEN** an owned non-primitive value is passed to a function by value or assigned to a new variable
- **THEN** ownership moves to the target, and subsequent access through the original binding is rejected at compile time

#### Scenario: Deterministic scope exit cleanup
- **WHEN** an owning variable leaves its enclosing block scope
- **THEN** its destructor is executed immediately and deterministically, dropping owned fields and heap allocations without garbage collector intervention

### Requirement: Recursive Heap Indirection via Heap<T>
Self-referential or recursive structures MUST use the standard `Heap<T>` primitive to provide single-owner heap allocation.

#### Scenario: Dropping Heap<T> allocation
- **WHEN** an instance of `Heap<T>` leaves scope
- **THEN** the allocated heap buffer and the inner value `T` are freed deterministically with zero reference-counter traffic

### Requirement: Stack-Scoped Borrowing and Mutation Rules
The type checker MUST enforce borrowing rules: arbitrary concurrent shared borrows (`&T`), or exactly one exclusive mutable borrow (`&mut T`), strictly confined to function arguments and local execution scopes.

#### Scenario: Concurrent shared borrows
- **WHEN** multiple functions inspect the same resource concurrently via `&T`
- **THEN** the compiler allows the shared reads

#### Scenario: Exclusive mutable borrow prevents concurrent access
- **WHEN** an exclusive borrow `&mut T` is active on a value
- **THEN** any other borrow or access through the owner is rejected at compile time

#### Scenario: Explicit mutation at call sites
- **WHEN** calling a function that accepts `&mut T`
- **THEN** the caller MUST pass the argument with explicit `&mut` syntax (`updateUser(&mut user)`)

### Requirement: Structs Prohibited from Storing Borrows
Struct definitions MUST NOT contain borrowed references (`&T` or `&mut T`), eliminating the need for explicit lifetime annotations (`'a`).

#### Scenario: Attempting to declare borrowed reference field in struct
- **WHEN** a struct attempts to declare a field of reference type (`struct View { val data: &String }`)
- **THEN** the compiler MUST reject the struct declaration at compile time

### Requirement: The Four Lifetime Elision Rules
When a function returns a borrowed reference, its lifetime MUST be inferred deterministically according to the four lifetime elision rules without explicit lifetime annotations.

#### Scenario: Rule 1 - Single input borrow parameter
- **WHEN** a function signature has exactly one input borrow (`fun first(items: &Vector<String>): &String?`)
- **THEN** the returned borrow's lifetime is automatically tied to the input parameter's lifetime at the call site

#### Scenario: Rule 2 - Method receiver borrow
- **WHEN** a method has a receiver borrow `this: &T` and returns a reference
- **THEN** the returned reference's lifetime is automatically tied to the receiver's lifetime

#### Scenario: Rule 3 - Multiple input borrow parameters
- **WHEN** a function accepts multiple input borrows and returns a borrow
- **THEN** the returned reference's lifetime is tied to the intersection of all input borrow scopes

#### Scenario: Rule 4 - Zero input borrows returning a borrow
- **WHEN** a function has zero input borrow parameters and attempts to return a borrow to a local value
- **THEN** compilation MUST fail with diagnostic `DanglingBorrowReturn`

### Requirement: Data-Oriented Typed Handles for Graphs
Graph topologies and cyclic data structures MUST use typed integer/handle IDs instead of raw pointers or reference-counting pointer cycles.

#### Scenario: Peer node reference via typed ID
- **WHEN** a struct references another struct via a typed identifier (`val parentId: NodeId?`)
- **THEN** no cyclic pointer graphs are formed and memory is managed by the owning collection

### Requirement: Reactive Tree Copy State Handles
Slot-table state handles (`State<T>`, `NodeId`) used in UI components MUST implement `Copy` and store generational indices rather than raw heap pointers.

#### Scenario: Capturing State<T> handle in event closure
- **WHEN** an escaping UI event closure captures a `State<T>` handle (`Button(onClick = { count.update { it + 1 } })`)
- **THEN** the handle is copied by value as a 64-bit ID, avoiding heap-allocation lifetime issues
