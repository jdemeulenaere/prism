# ADR-0002: Zero-Cost Memory Architecture (Zero GC, Zero ARC, Data-Oriented Handles)

## Status
Accepted (Supersedes previous lifetime annotation draft)

## Context
Prism aims for native C++/Rust performance and zero-cost memory safety, combined with Kotlin-like ergonomics and first-class Compose UI trees.
In standard Rust, combining borrow checking with UI trees and structs holding references leads to complex lifetime parameters (`'a`) and verbose escape hatches (`Rc<RefCell<...>>`).
Conversely, garbage-collected languages (Java/Kotlin) and Swift's Automatic Reference Counting (ARC) introduce cache-contention and throughput pauses.
We need a memory model that achieves bare-metal C++/Rust execution speeds without forcing developers to manage lifetimes or fight a borrow checker.

## Decision

### 1. Pure Value Ownership & Deterministic Destruction
- **Zero GC & Zero Implicit ARC**: The Prism runtime includes no garbage collector and introduces no hidden atomic retain/release counter instructions.
- Structs and composite types own their fields directly. When a variable goes out of scope, its memory is cleaned up deterministically.
- Move semantics apply to owned non-primitive values. Passing an owned value moves it without copying.

### 2. Zero Lifetime Syntax (`'a`) & Stack-Only Borrows
- **No Lifetimes on Structs**: Structs cannot declare lifetime parameters (`'a`) or hold long-lived borrowed references. Structs either contain their data by value or reference external entities using typed handles/IDs.
- **Function Borrows**: Functions may borrow arguments immutably (`&T`) or mutably (`&mut T`). Because these borrows are bounded by the call stack of the function, the compiler validates safety automatically without any lifetime syntax.

### 3. Data-Oriented Design & Typed Handles for Graph Relationships
- Graphs, many-to-one relationships, and UI component hierarchies use **Typed IDs/Handles** (e.g., `NodeId`, `EmployeeId`) within an owning collection or Arena.
- Avoids cyclic memory leaks, maximizes CPU cache locality, enables trivial serialization, and guarantees safety.

### 4. First-Class `compose fun` & Slot Table Handles
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
- **Negative / Tradeoffs**:
  - Developers cannot store a raw stack reference (`&T`) inside a struct; data models must use value containment or ID handles.
