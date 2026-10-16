# Prism

**Prism** (`.prism`) is a safe, expressive, statically typed programming language designed for high-performance native systems and first-class reactive UI application development.

It combines:
- The **ergonomics and readability of Kotlin** (`fun`, `val`/`var`, `when`, expression bodies, null safety).
- The **zero-cost safety and native performance of Rust** (value ownership, deterministic destruction, zero GC, zero hidden ARC, C++/Rust execution speed).
- The **developer velocity of zero-config conventions and instant hot reload**.
- Native, first-class compiler integration for **declarative tree transformations (Compose-style)** via the `compose` keyword and slot-table generational handles.

---

## The Elephant-Goldfish Operating Model via OpenSpec

Prism follows the **Elephant-Goldfish model** (see [Rensin, 2024](https://drensin.medium.com/elephants-goldfish-and-the-new-golden-age-of-software-engineering-c33641a48874)) using [OpenSpec](https://github.com/Fission-AI/openspec):
- **The Elephant (Durable Memory)**: The repository maintains permanent, living architecture specifications in [`openspec/specs/`](openspec/specs).
- **The Goldfish (Ephemeral Execution)**: Tasks are broken down into self-contained work packets in `openspec/changes/<change-name>/` with concrete requirements, scenarios, technical decisions, and testable tasks.

See [`AGENTS.md`](AGENTS.md) for contributor and agent workflow guidelines.

---

## Core Architectural Tenets

### 1. Zero-Config Convention over Configuration & Pure Path Discovery
- No mandatory build configuration files (`build.gradle`, `Cargo.toml`, `CMakeLists.txt`) and **no marker files** (`.prism-root`, `.git`, etc.).
- The filesystem is the package hierarchy:
  - `package foo.bar` $\iff$ file lives in `<workspace_root>/foo/bar/*.prism`.
  - **Zero-Marker Workspace Root Discovery**: Discovered directly by comparing the entry file's filesystem path and its `package` declaration.
  - Sibling imports use local identifiers: `import doo`.
  - Root-anchored imports use a leading slash: `import /core/fmt`.
- **Default Visibility is `private`**: A declaration without a modifier is visible only within its source file. Modifiers: `package` (package-visible) and `public` (cross-package visible).

### 2. File-as-Compilation-Unit & ABI Stability
- Every `.prism` file is an independent compilation unit for body code generation and artifact caching.
- Package directories form the declaration symbol boundary:
  - Phase 1: Fast declaration extraction (structs, functions, explicit signatures).
  - Phase 2: Function body lowering & typechecking against imported ABI signatures.
- Modifying private symbols or non-generic function bodies produces **zero downstream recompilations**.
- Generic functions record a Monomorphization Hash of their body; downstream files re-monomorphize only when generic bodies change.

### 3. Pure Zero-Cost Memory: Value Ownership, Zero GC, Zero Hidden ARC
- **No Garbage Collector & No Hidden ARC**: Memory is freed deterministically with zero runtime pauses and zero atomic counter overhead.
- **No Lifetime Syntax (`'a`)**: Structs own their data by value, use `Heap<T>` for recursive types, and use typed IDs/handles for graph topologies (data-oriented design).
- **Stack-Scoped Borrows**: References (`&T`, `&mut T`) are strictly scoped to function arguments and local calls, with automatic lifetime elision for returning borrows.

### 4. First-Class `compose` Primitives & Hot Reload
- The `compose` keyword (`compose fun`) is a native language modifier.
- Recomposition keying, parameter change tracking, and slot-table tree reconciliation are intrinsic to the compiler and runtime.
- Generational slot-table handles (`State<T>`, `NodeId`) are lightweight `Copy` IDs, allowing event callbacks to capture state without pointers or borrow-checker fighting.

### 5. Two-Tier Native Execution Engine
- **Tier 1 (Cranelift JIT & Dylib)**: Sub-second compilation (<15ms) directly into memory for instant scripting (`prism run`), unit tests, and function-pointer hot reloading during UI development.
- **Tier 2 (LLVM / Optimized AOT)**: Maximum-throughput optimized native binaries matching C++/Rust speed with whole-program optimization and LTO (`prism build --release`).

---

## Specifications & Invariants

Implemented behavior is specified in [`openspec/specs/`](openspec/specs). Planned subsystems (code generation, compilation avoidance, execution tiers, the memory model and the compose engine) are designed in [`docs/architecture.md`](docs/architecture.md). The tenets above are the vision; see [`ROADMAP.md`](ROADMAP.md) for what exists today.
- [`system-invariants`](openspec/specs/system-invariants/spec.md): The 6 non-negotiable architectural rules.
- [`package-resolution`](openspec/specs/package-resolution/spec.md): Zero-marker discovery and import resolution.
- [`syntax`](openspec/specs/syntax/spec.md): Concrete grammar, tokens, and language constructs.
- [`type-system`](openspec/specs/type-system/spec.md): Bidirectional typing, Nothing bottom type, and nominal types.
- [`compilation-and-abi`](openspec/specs/compilation-and-abi/spec.md): Phase 1 ABI extraction and deterministic ABI hashing.
- [`diagnostics`](openspec/specs/diagnostics/spec.md): The catalog of compiler error codes.

---

## Bootstrapping Roadmap

Development progress is tracked in [`ROADMAP.md`](ROADMAP.md):
- **Milestone 1**: Rust-based bootstrap compiler (`prismc`) and developer CLI (`prism`).
- **Milestone 2**: First-class Jetpack Compose UI engine, reactive tree runtime, and native hot-reload framework.
- **Milestone 3**: Self-hosting compiler written in Prism itself.
