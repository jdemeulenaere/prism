# Prism Syntax & Grammar Specification (Draft)

## 1. Overview
Prism (`.prism`) adopts a syntax deeply inspired by Kotlin, prioritizing clean readability, minimal punctuation noise, and high parsing throughput.
At the same time, Prism's semantic engine enforces Rust-level safety (ownership, first-class lifetimes `'a`, borrow checking, no data races) without runtime GC pauses.

---

## 2. Sample Code

### 2.1. Basic Functions, Immutability & Expressions
```kotlin
package demo.math

import /core/fmt

// Default visibility is 'package'
fun square(x: Int): Int = x * x

// Public function with explicit return type (required for ABI export)
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

### 2.2. Data Structures, Ownership & Lifetimes
```kotlin
package demo.models

// Value struct (allocated on stack or inlined)
public struct Point {
    val x: Double
    val y: Double
}

// Struct with owned fields
public struct User {
    val id: UInt64
    var name: String
    val email: String?   // Nullable type
}

// Borrowing & mutation
public fun updateName(user: &mut User, newName: String) {
    user.name = newName
}

// Struct holding a borrowed reference with an explicit lifetime parameter
public struct StringSlice<'a> {
    val text: &'a str
    val length: UInt
}

// Lifetime elision on functions:
public fun createSlice(source: &str): StringSlice {
    return StringSlice(text = source, length = source.len())
}
```

### 2.3. Reactive Tree & Compose Support
```kotlin
package demo.ui

import /prism/ui/widgets
import /prism/ui/state

// '@compose' tells the compiler to inject slot-table memoization and re-entrant tracking
@compose
public fun CounterView(initialCount: Int = 0) {
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
- Default (no keyword): `package` — accessible to all `.prism` files in the exact same package directory.
- `public`: Accessible across any package.
- `private`: Accessible only within the file.
- **Export Rule**: All `public` and `package` declarations must have explicit parameter and return types for independent ABI extraction.

### 3.2. Types & Null Safety
- Primitive types: `Int`, `Int8`, `Int16`, `Int32`, `Int64`, `UInt`, `UInt64`, `Float`, `Double`, `Bool`, `Char`, `String`, `str`.
- Nullable types: `T?` represents optionality (lowered to zero-cost `Option<T>` in the backend).
- Elvis operator `?:` and safe navigation `?.` work as in Kotlin:
  ```kotlin
  val len = name?.length ?: 0
  ```

### 3.3. Ownership, References & Lifetimes
- Variables are immutable by default (`val`).
- Mutable bindings use `var`.
- Shared immutable reference: `&T` or `&'a T` (read-only borrow).
- Exclusive mutable reference: `&mut T` or `&'a mut T` (read-write borrow).
- Move semantics by default for heap resources (`String`, `Vector<T>`).
- Primitive types and Slot-Table State handles implement `Copy`.

### 3.4. Imports & Workspace Anchor
- Sibling import: `import doo` imports `doo.prism` in the same package.
- Root import: `import /core/fmt` imports `<workspace_root>/core/fmt.prism`.
- The leading `/` anchors to workspace root cleanly without colliding with `//` line comments.
