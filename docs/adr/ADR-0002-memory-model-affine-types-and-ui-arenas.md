# ADR-0002: Affine Types, Lifetimes, and UI Arenas

## Status
Accepted

## Context
Prism aims for native C++/Rust performance and zero-cost memory safety, combined with Kotlin-like ergonomics and first-class Compose-like UI trees.
In standard Rust, combining borrow checking with UI trees (event listeners, closures capturing parent state, cyclic references) is notoriously verbose (`Rc<RefCell<...>>`, complex lifetime annotations).
We need a memory model that preserves compile-time safety and deterministic destruction without ruining UI authoring ergonomics.

## Decision

### 1. Affine Types, Ownership, and First-Class Lifetimes
- Variables are owned by default. Passing an owned non-`Copy` value moves it.
- Prism supports immutable borrows (`&T` or `val` view) and mutable borrows (`&mut T` or `var` inout).
- Structs can hold borrowed references by declaring explicit lifetime parameters:
  ```kotlin
  public struct StringSlice<'a> {
      val data: &'a str
  }
  ```
- Lifetime elision is applied to standard function signatures to minimize syntactic clutter.
- Values are dropped deterministically at the end of their scope. No tracing GC, no hidden background pauses.

### 2. Composable Recomposition & Closure Escapes
- Composable functions (`@compose fun`) are re-entrant and can execute repeatedly upon state invalidation.
- **Escaping UI Event Closures** (such as `Button(onClick = { ... })`) outlive the stack frame of the composable function:
  - An escaping closure **cannot** borrow stack-local variables whose lifetime ends with the current composition pass.
  - An escaping closure **can** capture:
    1. Generational slot-table state handles (`State<T>`, `NodeId`), which are lightweight 64-bit integer values implementing `Copy`.
    2. Explicitly cloned/moved data.
    3. Shared reference-counted pointers (`Shared<T>`).

### 3. Slot Table Lifecycle & Generational Handles
- Declarative UI nodes and reactive state values (`remember { mutableStateOf(...) }`) live in a managed runtime Slot Table.
- **Deterministic Drops**: When a composable group or node is unmounted from the tree during recomposition, the slot table executes deterministic drops (`drop(&mut value)`) on all owned non-primitive resources registered in its slots.
- **Stale Handle Safety**: If an asynchronous task or detached callback invokes a method on a stale generational state handle, it safely reads the last recorded snapshot or no-ops writes with a diagnostic warning, preventing use-after-free and abrupt process panics.

## Consequences
- **Positive**:
  - Full systems programming capabilities with zero-cost memory abstractions, views, and string slices.
  - Zero garbage collector overhead, deterministic performance, tiny memory footprint.
  - Idiomatic Kotlin-like syntax for UI composition without exposing users to lifetime struggles in standard UI code.
- **Negative / Tradeoffs**:
  - Systems-level code using borrowed references in structs requires explicit lifetime annotations (`'a`).
  - Developers writing UI event handlers must be aware that escaping closures require `Copy` state handles or clones rather than borrowing temporary stack locals.
