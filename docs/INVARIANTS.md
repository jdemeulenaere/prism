# Prism System Invariants

These invariants are permanent, non-negotiable architectural rules. All compiler components, tooling, and runtime layers must uphold them.

---

### Invariant 1: Single File as Compilation Unit
- Any `.pm` source file can be parsed into its ABI interface signature independently of the implementation bodies of other files.
- The ABI hash of a file depends **only** on its `public` and `package`-visible declarations and signatures, never on `private` symbols or function bodies.

### Invariant 2: Zero Mandatory Build Files
- A Prism project must be buildable, testable, and runnable using only directory conventions and import paths.
- Workspace root discovery detects version control (`.git`) or an optional root boundary marker.

### Invariant 3: Deterministic Hermetic Builds & Compilation Avoidance
- The compilation artifact of a file is a pure function of:
  1. The file's content.
  2. The resolved ABI hashes of its direct imports.
  3. The compiler version and target triple.
- If those inputs do not change, compilation must be completely avoided (cache hit).

### Invariant 4: Kotlin Syntax Familiarity without Grammatical Ambiguity
- Prism syntax should feel native to a Kotlin developer (`fun`, `val`, `var`, `class`, `when`, `if` expressions, nullable types `T?`).
- Parsing must be strictly deterministic and fast (Pratt expression parsing, unambiguous grammar rules).

### Invariant 5: Safe Affine Memory without Garbage Collector Pauses
- Prism uses affine types, move semantics, and borrow checking to guarantee memory safety at compile time without a tracing garbage collector.
- Long-lived UI trees and reactive state variables use generational arena / slot-table handles (lightweight `Copy` handle types) so closures can capture state without borrow-checker friction.

### Invariant 6: First-Class Compose Engine & Native Hot Reload
- Recomposition keying, parameter change tracking, and slot-table tree reconciliation are intrinsic to the compiler and runtime.
- Hot reload during development swaps function pointers in Cranelift JIT / interpreter tables while keeping slot-table state intact.
