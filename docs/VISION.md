# Prism Language Vision & Guiding Principles

## 1. High-Level Vision
**Prism** (`.pm`) is a safe, expressive, statically typed programming language designed for high-performance native systems and first-class reactive UI application development.
It combines:
- The **ergonomics and readability of Kotlin** (`fun`, `val`/`var`, `when`, expression bodies, null safety).
- The **zero-cost safety and native performance of Rust** (affine types, ownership, borrow checking, no garbage collector pauses).
- The **developer velocity of zero-config conventions and instant hot reload**.
- Native, first-class compiler integration for **declarative tree transformations (Compose-style)** using an arena/slot-table handle architecture.

---

## 2. The Elephant-Goldfish Operating Model
As defined in [Elephants, Goldfish, and the New Golden Age of Software Engineering](https://drensin.medium.com/elephants-goldfish-and-the-new-golden-age-of-software-engineering-c33641a48874):
- **The Elephant (Durable Memory)**: The repository maintains permanent, living architecture specifications, invariants (`docs/INVARIANTS.md`), decision records (`docs/adr/`), and executable language test suites. Any contributor or AI agent can reorient itself in seconds.
- **The Goldfish (Ephemeral Execution)**: Tasks are broken down into small, isolated, test-driven units of work executed with minimal context bloat.

---

## 3. Core Architectural Tenets

### 3.1. Zero-Config Convention over Configuration
- No mandatory build configuration files (`build.gradle`, `Cargo.toml`, `CMakeLists.txt`) for standard single-workspace development.
- The file system is the package hierarchy:
  - `package foo.bar` $\iff$ file lives in `<workspace_root>/foo/bar/*.pm`.
  - `import doo` imports sibling module `doo.pm` in `foo/bar/`.
  - `import //abc/xyz/util` imports `<workspace_root>/abc/xyz/util.pm`.
  - Empty package allows only relative imports.
- Default symbol visibility is `package`. Explicit visibilities: `public`, `private`.

### 3.2. File-as-Compilation-Unit & ABI Stability
- Every `.pm` file is an independent compilation unit.
- Two-phase compilation pipeline:
  1. **Signature/ABI extraction**: Fast lex & parse of declarations to yield a stable ABI hash (`.abi`).
  2. **Body lowering & typechecking**: Depends solely on imported ABI hashes.
- Internal body changes do not trigger recompilation of downstream files.

### 3.3. Memory Model: Affine Types + Slotmap Reactive Handles
- Strict ownership and borrow checking without a tracing garbage collector or runtime overhead.
- UI trees and reactive states use **Slot-Table / Generational Arena handles** (lightweight `Copy` ID types) so event closures can access state seamlessly without lifetime entanglement.

### 3.4. Three-Tier Execution Engine
- **Tier 1 (Bytecode Interpreter)**: Instant execution for scripting, CLI scripts, formatting, and unit tests.
- **Tier 2 (Cranelift JIT & Dylib)**: Sub-second compilation and hot-reloading for UI iteration.
- **Tier 3 (LLVM / Optimized AOT)**: Maximum-throughput optimized native binaries matching C++/Rust speed.

---

## 4. Bootstrapping Roadmap
- **Milestone 1**: Rust-based bootstrap compiler (`prismc`) and developer CLI (`prism`).
- **Milestone 2**: Self-hosting compiler written in Prism itself.
- **Milestone 3**: First-class Jetpack Compose UI engine and native hot-reload framework.
