# Prism Syntax & Grammar Specification (Draft)

## 1. Overview
Prism (`.prism`) adopts a syntax deeply inspired by Kotlin, prioritizing clean readability, minimal punctuation noise, and high parsing throughput.
At the same time, Prism's semantic engine enforces Rust-level safety (value ownership, deterministic destruction, no data races) without runtime GC, without hidden ARC, and without complex lifetime annotations (`'a`).

---

## 2. Sample Code

### 2.1. Basic Functions, Immutability & Expressions
```kotlin
package demo.math

import /core/fmt

// Default visibility is 'private' (accessible only within this file)
fun square(x: Int): Int = x * x

// 'package' visibility: accessible to any file in 'demo.math'
package fun internalHelper(x: Int): Int = square(x) + 1

// 'public' visibility with explicit return type (required for exported ABI)
public fun hypotenuse(a: Double, b: Double): Double {
    val a2 = a * a
    val b2 = b * b
    return (a2 + b2).sqrt()
}

// When expression & pattern matching
public fun describe(value: Int): String = when (value) {
    0 -> "zero"
    1, 2, 3 -> "small number"
    in 4..10 -> "medium number"
    else -> "large number"
}
```

### 2.2. Data Structures, Ownership, Enums & Box
```kotlin
package demo.models

// Value struct (allocated inline on stack or inside parent struct)
public struct Point {
    val x: Double
    val y: Double
}

// Struct with owned fields (zero GC, zero ARC, deterministic cleanup)
public struct User {
    val id: UInt64
    var name: String
    val email: String?   // Nullable type
}

// Function borrowing & mutation with explicit call-site borrow
public fun updateName(user: &mut User, newName: String) {
    user.name = newName
}

// Algebraic Data Types (Enums) & Recursive Types via Box<T>
public enum Expr {
    Literal(Int)
    Add(Box<Expr>, Box<Expr>)
    Multiply(Box<Expr>, Box<Expr>)
}

// Returning borrows with automatic lifetime elision
public fun first(items: &Vector<String>): &String? {
    return items.get(0)
}

// Data-Oriented Design: Referencing other objects via Typed IDs/Handles
public struct EmployeeId(val id: UInt32)

public struct Employee {
    val id: EmployeeId
    val name: String
    val managerId: EmployeeId?   // References another Employee by ID without cycles or GC!
}
```

### 2.3. Reactive Tree & Compose Support (`compose fun`)
```kotlin
package demo.ui

import /prism/ui/widgets
import /prism/ui/state

// First-class 'compose' keyword designates re-entrant UI functions
public compose fun CounterView(initialCount: Int = 0) {
    // 'remember' allocates a slot in the current composition's Slot Table.
    // 'count' is a lightweight Copy handle (State<Int>), safe to capture in escaping closures!
    val count = remember { mutableStateOf(initialCount) }

    Column(padding = 16) {
        Text(text = "Current count: ${count.value}")
        
        Row(spacing = 8) {
            Button(onClick = { count.update { it - 1 } }) {
                Text("-")
            }
            Button(onClick = { count.update { it + 1 } }) {
                Text("+")
            }
        }
    }
}
```

---

## 3. Lexical Structure & Grammar Highlights

### 3.1. Visibilities
- **Default (no keyword)**: `private` — accessible only within the declaring source file.
- `package`: Accessible to all `.prism` files in the exact same package directory.
- `public`: Accessible across any package.
- **Export Rule**: All `public` and `package` declarations must have explicit parameter and return types for independent ABI extraction.

### 3.2. Types & Null Safety
- Primitive types: `Int`, `Int8`, `Int16`, `Int32`, `Int64`, `UInt`, `UInt64`, `Float`, `Double`, `Bool`, `Char`, `String`.
- Nullable types: `T?` represents optionality (lowered to zero-cost `Option<T>` in the backend).
- Elvis operator `?:` and safe navigation `?.` work as in Kotlin:
  ```kotlin
  val len = name?.length ?: 0
  ```

### 3.3. Pure Zero-Cost Memory (Zero GC, Zero ARC, No Lifetimes)
- Variables are immutable by default (`val`).
- Mutable bindings use `var`.
- Move semantics for owned non-primitive values.
- Shared immutable reference: `&T` (strictly stack/function-scoped).
- Exclusive mutable reference: `&mut T` (strictly stack/function-scoped).
- Explicit call-site borrows for mutations: `updateName(&mut user, "Alice")`.
- Lifetime elision: Return borrows automatically tied to input borrow scopes.
- Heap indirection for recursive types: `Box<T>`.
- No explicit lifetime parameters (`'a`) anywhere in the language.
- Primitive types and Slot-Table State handles implement `Copy`.

### 3.4. Imports & Workspace Anchor
- Sibling import: `import doo` imports `doo.prism` in the same package.
- Root import: `import /core/fmt` imports `<workspace_root>/core/fmt.prism`.
- The leading `/` anchors to workspace root cleanly without colliding with `//` line comments.
- Workspace root is dynamically discovered from the file's path and its `package` statement with **zero marker files**.
