# ADR-0002: Affine Types, Borrow Checking, and UI Arenas

## Status
Accepted

## Context
Prism aims for native C++/Rust performance and zero-cost memory safety, combined with Kotlin-like ergonomics and first-class Compose-like UI trees.
In standard Rust, combining borrow checking with UI trees (event listeners, closures capturing parent state, cyclic references) is notoriously verbose (`Rc<RefCell<...>>`, complex lifetime annotations).
We need a memory model that preserves compile-time safety and deterministic destruction without ruining UI authoring ergonomics.

## Decision
1. **Affine Types & Ownership as Core Language Semantics**:
   - Variables are owned by default. Passing an owned non-`Copy` value moves it.
   - Prism supports immutable borrows (`&T` or `val` view) and mutable borrows (`&mut T` or `var` inout).
   - Values are dropped deterministically at the end of their scope. No tracing GC, no hidden background pauses.

2. **UI State & Trees via Generational Arena / Slot-Table Handles**:
   - Declarative UI nodes and reactive state values (`remember { mutableStateOf(...) }`) live in a managed runtime Slot Table / Arena.
   - Handles to reactive states (e.g., `State<T>`, `NodeId`) are lightweight 64-bit integer values implementing `Copy`.
   - Event closures (e.g. `Button(onClick = { count.set(count.get() + 1) })`) can capture state handles by value without any lifetime fighting or reference counting cycles.

3. **Compiler Ergonomics**:
   - For pure value types and POD (Plain Old Data) structs, Prism provides automatic `Copy` or implicit clone where safe.
   - For closures, state handles are captured by value automatically.

## Consequences
- **Positive**:
  - Zero garbage collector overhead, deterministic performance, tiny memory footprint.
  - Idiomatic Kotlin-like syntax for UI composition without exposing users to lifetime struggles.
  - Predictable cache-locality and performance.
- **Negative / Tradeoffs**:
  - Non-UI system code requires developers to understand ownership and borrowing (same as Rust).
  - Slot table state handles require runtime generation checks to detect stale or invalid handles (preventing use-after-free safely).
