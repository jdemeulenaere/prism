# Prism System Invariants

These invariants are permanent, non-negotiable architectural rules. All compiler components, tooling, and runtime layers must uphold them.

---

### Invariant 1: Single File as Compilation Unit & Explicit Export ABI Boundaries
- Any `.prism` source file is compiled into machine code/bytecode independently.
- **Default visibility is `private`**: Declarations without an explicit visibility modifier are private to the declaring file.
- `package` visibility explicitly exposes a declaration to sibling files sharing the identical package.
- `public` visibility explicitly exposes a declaration across packages.
- To guarantee independent Phase 1 ABI extraction without inspecting function bodies, **all `public` and `package`-visible declarations must declare explicit parameter and return types**.
- The exported ABI hash of a file depends **only** on its `public` and `package`-visible declarations, signatures, and generic AST representations. Private symbols and non-generic function bodies are excluded from the exported ABI.

### Invariant 2: Zero Mandatory Build Files & Zero Marker Files
- A Prism project requires no build configuration files (`build.gradle`, `Cargo.toml`, `CMakeLists.txt`) and **no marker files** (`.prism-root`, etc.).
- **Pure Path Root Discovery**: The workspace root is deterministically discovered by comparing the filesystem path of the source file against its `package` declaration.
  - Example: A file at `/workspace/app/ui/views.prism` declaring `package app.ui` maps directly to root `/workspace`.
  - A file with an empty `package` declaration is located directly at `<workspace_root>/`.
- Leading `/` in an import (e.g. `import /core/fmt`) anchors to this discovered workspace root.

### Invariant 3: Deterministic Hermetic Builds & Compilation Avoidance
- The compilation artifact of a file is a pure function of:
  1. The file's content.
  2. The resolved ABI hashes of its direct imports (including generic monomorphization hashes).
  3. The compiler version and target triple.
- If those inputs do not change, compilation must be completely avoided (cache hit).
- In Tier 1 (interpreter) and Tier 2 (Cranelift debug/hot reload), cross-file inlining is disabled across compilation unit boundaries to guarantee maximum compilation avoidance. Tier 3 (LLVM release) enables cross-file inlining via LTO.

### Invariant 4: Kotlin Syntax Familiarity without Grammatical Ambiguity
- Prism syntax should feel native to a Kotlin developer (`fun`, `val`, `var`, `struct`, `enum`, `when`, `if` expressions, nullable types `T?`).
- Parsing must be strictly deterministic and fast (Pratt expression parsing, unambiguous grammar rules).

### Invariant 5: Zero GC, Zero Hidden ARC, No Lifetime Annotations
- Prism enforces memory safety through **value ownership and deterministic destruction**.
- **No runtime Tracing Garbage Collector** and **no implicit Automatic Reference Counting (ARC)**.
- **No explicit lifetime syntax (`'a`)**: Structs own their data by value, use `Box<T>` for recursive types, or use typed handles/IDs for graphs and relationships.
- Borrowing (`&T`, `&mut T`) is strictly confined to function arguments and local scopes, with automatic lifetime elision for returning borrows.

### Invariant 6: First-Class `compose` Primitives & Native Hot Reload
- `compose` is a native language modifier (`compose fun`).
- Recomposition keying, parameter change tracking, and slot-table tree reconciliation are intrinsic to the compiler and runtime.
- Slot-table state handles (`State<T>`, `NodeId`) are lightweight `Copy` IDs that eliminate pointer cycle issues.
- Hot reload during development swaps function pointers in Cranelift JIT / interpreter tables while keeping slot-table state intact.
