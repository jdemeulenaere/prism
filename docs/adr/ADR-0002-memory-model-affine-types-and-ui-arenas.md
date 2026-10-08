# ADR-0002: Zero-Cost Memory Architecture (Zero GC, Zero ARC, Data-Oriented Handles, Box)

## Status
Accepted

## Context
Prism aims for native C++/Rust performance and zero-cost memory safety, combined with Kotlin-like ergonomics and first-class Compose UI trees.
In standard Rust, combining borrow checking with UI trees and structs holding references leads to complex lifetime parameters (`'a`) and verbose escape hatches (`Rc<RefCell<...>>`).
Conversely, garbage-collected languages (Java/Kotlin) and Swift's Automatic Reference Counting (ARC) introduce cache-contention and throughput pauses.
We need a memory model that achieves bare-metal C++/Rust execution speeds without forcing developers to manage lifetimes or fight a borrow checker.

## Decision

### 1. Pure Value Ownership & Deterministic Destruction
- **Zero GC & Zero Implicit ARC**: The Prism runtime includes no garbage collector and introduces no hidden atomic retain/release counter instructions.
- Structs own their fields directly. When a variable goes out of scope, its memory is cleaned up deterministically.
- Move semantics apply to owned non-primitive values. Passing an owned value moves it without copying.

### 2. Recursive Types & Heap Indirection (`Box<T>`)
- Because structs own their fields by value, recursive types (such as linked lists, AST expressions, or JSON trees) require heap indirection to achieve a finite compile-time memory layout:
  ```kotlin
  public enum Expr {
      Literal(Int)
      Add(Box<Expr>, Box<Expr>)
  }
  ```
- `Box<T>` is a unique, owned heap allocation with **zero GC and zero ARC**:
  - `sizeof(Box<T>) == sizeof(usize)` (single 64-bit pointer).
  - Maintains strict unique affine ownership.
  - When `Box<T>` drops, it frees the heap buffer and drops `T` deterministically.
  - Requires **zero lifetime syntax** (`'a`).

### 3. Zero Lifetime Syntax (`'a`) & Function Lifetime Elision
- **No Lifetimes on Structs**: Structs cannot declare lifetime parameters (`'a`) or hold long-lived borrowed references. Structs either contain their data by value, use `Box<T>` for heap indirection, or reference external entities using typed handles/IDs.
- **Function Borrows & Lifetime Elision**: Functions may borrow arguments immutably (`&T`) or mutably (`&mut T`). When a function returns a borrowed reference, lifetime elision resolves the scope automatically:
  - **Rule 1 (Single Input Borrow)**: If a function accepts exactly one borrow parameter, an output borrow is implicitly tied to that parameter's scope at the call site (e.g. `fun first(items: &Vector<String>): &String`).
  - **Rule 2 (Method Receiver)**: In a method with a receiver borrow (`this: &T`), an output borrow is implicitly tied to the receiver.
  - **Rule 3 (Multiple Input Borrows)**: An output borrow is tied to the intersection of all input borrow scopes (all inputs remain borrowed while the return reference is live).
  - **Rule 4 (Zero Input Borrows)**: A function cannot return a borrowed reference if there are zero input borrows (prevents returning dangling pointers to local stack variables).

### 4. Data-Oriented Design & Typed Handles for Graph Relationships
- Graphs, many-to-one relationships, and UI component hierarchies use **Typed IDs/Handles** (e.g., `NodeId`, `EmployeeId`) within an owning collection or Arena.
- Avoids cyclic memory leaks, maximizes CPU cache locality, enables trivial serialization, and guarantees safety.

### 5. First-Class `compose fun` & Slot Table Handles
- The `compose` keyword designates re-entrant composable functions (`compose fun`).
- State variables (`remember { mutableStateOf(...) }`) reside in the UI runtime's Slot Table.
- State references returned by `remember` are lightweight 64-bit `Copy` handles (`State<T>`).
- UI event callbacks (e.g. `Button(onClick = { ... })`) capture state handles by value cleanly without pointer entanglement or lifetime issues.

## Consequences
- **Positive**:
  - Predictable, microsecond-latency execution matching C++ and Rust.
  - Zero GC pause micro-stutters during 120 FPS UI rendering.
  - Clean, approachable syntax matching Kotlin's developer experience.
  - No "lifetime contagion" across struct hierarchies.
  - Clear, finite layout for recursive types via `Box<T>`.
- **Negative / Tradeoffs**:
  - Developers cannot store a raw stack reference (`&T`) inside a struct; data models must use value containment, `Box<T>`, or ID handles.
