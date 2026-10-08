# Prism Memory Model & Resource Management Specification

**Status**: Living Architecture Specification (The Elephant)  
**Applies To**: Compiler Frontend, Type Checker, Borrow Checker, and Runtime  
**Governing Invariants**: [Invariant 5 (`docs/INVARIANTS.md`)](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md#L34), [Invariant 6 (`docs/INVARIANTS.md`)](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md#L40)

---

## 1. Overview & Guiding Philosophy

Prism achieves bare-metal C++/Rust execution speeds and compile-time memory safety without:
1. A runtime **Tracing Garbage Collector** (GC).
2. Hidden, ubiquitous **Automatic Reference Counting** (ARC / retain-release traffic).
3. Complex **Lifetime Annotations** (`'a`) polluting types and structs.

Memory safety is guaranteed through **single-owner value semantics**, deterministic scope-based destruction, strictly stack-confined borrows with automatic lifetime elision, and data-oriented typed handles for cyclic or graph data structures.

---

## 2. Value Ownership & Deterministic Destruction

### 2.1. Single Owner Semantics
- Every resource in Prism has a single owning binding.
- By default, variables are immutable (`val`). Mutable variables are declared with `var`.
- Structs own their fields inline (by value).
- Passing or assigning an owned non-primitive value moves ownership by default.

### 2.2. Deterministic Destruction (RAII)
- When an owning variable leaves its lexical scope, its destructor is invoked immediately and deterministically.
- Nested struct fields and owned heap buffers (`Heap<T>`) are recursively dropped in reverse declaration order.
- No stop-the-world pauses, no finalizer threads, and no atomic counter cache bouncing.

---

## 3. Recursive Types & Heap Indirection (`Heap<T>`)

Because structs contain their fields inline by value, self-referential or recursive structures (e.g. linked lists, AST expressions, tree nodes) would produce infinite compile-time size:

```kotlin
// Requires heap indirection to achieve a finite memory layout:
public enum Expr {
    Literal(Int)
    Add(Heap<Expr>, Heap<Expr>)
    Multiply(Heap<Expr>, Heap<Expr>)
}
```

### 3.1. `Heap<T>` Semantics
- `Heap<T>` is a standard-library primitive providing unique, owned heap allocation.
- **Memory Footprint**: `sizeof(Heap<T>) == sizeof(usize)` (a single 64-bit pointer on 64-bit systems).
- **Affine Ownership**: Maintains strict single ownership. Moving a `Heap<T>` transfers the underlying heap pointer without copying the payload.
- **Deterministic Cleanup**: When `Heap<T>` drops, it frees the allocated heap memory and drops `T` deterministically with zero GC and zero ARC.
- **Zero Lifetime Syntax**: `Heap<T>` acts as an owned value, requiring zero lifetime parameters (`'a`).

---

## 4. Borrowing & Automatic Lifetime Elision

Prism supports lightweight borrowing to allow functions to inspect or mutate values without taking ownership.

### 4.1. Borrow Types & Constraints
- `&T`: Shared, immutable borrow. Multiple concurrent shared borrows are permitted.
- `&mut T`: Exclusive, mutable borrow. While an exclusive borrow is active, no other borrows or access through the owner are permitted.
- **Explicit Mutation at Call Sites**: Passing a mutable borrow requires explicit syntax:
  ```kotlin
  updateUser(&mut user, "New Name")
  ```
- **No Lifetimes in Structs**: Structs cannot store borrowed references (`&T` or `&mut T`). Structs only hold data by value, via `Heap<T>`, or via typed handles. Borrows are strictly confined to function arguments and local execution scopes.

### 4.2. The Four Lifetime Elision Rules
When a function returns a borrowed reference (`&T` or `&mut T`), its lifetime is automatically inferred relative to the input arguments without explicit `'a` syntax:

1. **Rule 1 (Single Input Borrow)**:
   If a function signature contains exactly one input borrow parameter, the return borrow is implicitly tied to that parameter's scope at the call site:
   ```kotlin
   public fun first(items: &Vector<String>): &String?
   ```
2. **Rule 2 (Method Receiver)**:
   In a method with an explicit or implicit receiver borrow (`this: &T` or `this: &mut T`), an output borrow is implicitly tied to the receiver's scope.
3. **Rule 3 (Multiple Input Borrows)**:
   If a function accepts multiple input borrow parameters, an output borrow is conservatively tied to the intersection of all input borrow scopes (all inputs must remain borrowed while the return reference is live).
4. **Rule 4 (Zero Input Borrows)**:
   A function cannot return a borrowed reference if there are zero input borrows. Returning a reference to a local stack-allocated variable is a compile-time error (`Error: DanglingBorrowReturn`).

---

## 5. Data-Oriented Design & Typed Handles for Graph Relationships

Traditional object-oriented designs model cyclic graphs and peer relationships using raw pointers, garbage collection, or reference-counted pointer webs (`Rc<RefCell<T>>`), creating cache misses and reference cycles.

Prism resolves this using **Data-Oriented Typed Handles**:
- Graph nodes, parent-child trees, and peer relationships store typed ID handles rather than pointers:
  ```kotlin
  public struct EmployeeId(val id: UInt32)

  public struct Employee {
      val id: EmployeeId
      val name: String
      val managerId: EmployeeId?   // References another Employee by ID without cycles or GC!
  }
  ```
- Handles are stored within an owning collection, arena, or table.
- Benefits:
  - Eliminates cyclic memory leaks.
  - Maximizes CPU cache locality.
  - Enables effortless serialization and IPC.
  - 100% borrow-checker safe with zero lifetime annotations.

---

## 6. Reactive Tree & Compose Slot-Table State Handles

Prism provides first-class language support for declarative UI via the `compose` modifier (`compose fun`).

### 6.1. Slot Table State Model
- Composable functions execute re-entrantly against a thread-local Slot Table runtime.
- State allocated via `remember { mutableStateOf(initial) }` is stored in the Slot Table rather than on the call stack.

### 6.2. `Copy` State Handles
- The value returned by `remember` is a lightweight generational handle (`State<T>`):
  - `sizeof(State<T>) == sizeof(u64)` (Slot index + generation counter).
  - Implements the `Copy` trait.
- UI event closures capture `State<T>` handles by value:
  ```kotlin
  public compose fun CounterView(initial: Int = 0) {
      val count = remember { mutableStateOf(initial) }
      Button(onClick = { count.update { it + 1 } }) {
          Text("Count: ${count.value}")
      }
  }
  ```
- Because state handles are `Copy` IDs, escaping event closures never entangle with stack lifetimes, require no reference-counting wrappers, and do not fight the borrow checker.

---

## 7. Design Rationale & Tradeoffs

| Mechanism | Alternative Considered | Why Rejected / Prism Choice |
| :--- | :--- | :--- |
| **Value Ownership & `Heap<T>`** | Tracing Garbage Collector (Java/Go) | Stop-the-world pauses and GC stutters are unacceptable for 120 FPS UI rendering and native systems performance. |
| **Deterministic RAII** | Automatic Reference Counting (Swift) | Atomic retain/release counter updates on every assignment create severe CPU cache contention and memory fragmentation. |
| **Stack-Scoped Borrows with Elision** | Explicit Lifetime Annotations (`'a`) (Rust) | Explicit lifetimes create significant cognitive overhead and contaminate struct definitions throughout application code. |
| **Typed Handles (`NodeId`)** | `Rc<RefCell<Node>>` / Pointer Graphs | Pointer graphs lead to memory leaks on cycles, cache thrashing, and runtime borrow panic failures. |
