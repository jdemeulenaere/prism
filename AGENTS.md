# Prism Agent Guidelines (The Elephant & Goldfish Operating Manual)

Welcome to the **Prism** language repository. This document serves as the primary onboarding directive for AI agents and human contributors.

---

## 1. The Core Philosophy: The Elephant & Goldfish Model
This project strictly follows the **Elephant-Goldfish model** (see [Rensin, 2024](https://drensin.medium.com/elephants-goldfish-and-the-new-golden-age-of-software-engineering-c33641a48874)):
- **The Elephant (Durable Memory)**: All architectural invariants, grammar specifications, decisions, and roadmaps are permanently documented in `docs/`. When starting a task, **always consult the Elephant docs** to ensure alignment. Never introduce architectural changes without updating the relevant specification or invariant document.
- **The Goldfish (Ephemeral Execution)**: Individual tasks (lexing, parsing, lowering, codegen, tests) are executed as self-contained, test-driven units of work. Avoid bloated context; rely on the Elephant docs and test suites.

---

## 2. Architectural Invariants Quick-Sheet (Non-Negotiable)

Any proposed code, grammar, or compiler change must uphold these rules (detailed in [`docs/INVARIANTS.md`](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md)):

1. **File Extension**: Canonical extension is `.prism`.
2. **Default Visibility is `private`**:
   - No modifier $\implies$ private to the file.
   - `package` $\implies$ visible to files in the exact same package directory.
   - `public` $\implies$ visible across packages.
   - All `public` and `package` declarations **must** declare explicit parameter and return types for independent ABI extraction.
3. **Zero-Config, Zero-Marker Root Discovery**:
   - No build configuration files and **no marker files** (`.prism-root`, `.git`, etc.).
   - The workspace root is discovered by matching the source file's directory path with its `package` declaration (e.g. `/path/to/project/foo/bar/file.prism` with `package foo.bar` $\implies$ workspace root is `/path/to/project`).
   - Root-anchored imports use a leading slash: `import /core/fmt`.
   - Sibling imports use standard identifiers: `import doo`.
4. **Memory Architecture: Zero GC, Zero Hidden ARC, No Lifetime Annotations (`'a`)**:
   - Memory is freed deterministically with single-owner value semantics.
   - No tracing garbage collector and no implicit atomic retain/release counter overhead.
   - Structs own their fields by value; recursive types use `Heap<T>`.
   - Borrows (`&T`, `&mut T`) are strictly function/stack-scoped with automatic lifetime elision.
   - Graphs, peer relationships, and UI nodes use typed ID handles (Data-Oriented Design).
5. **First-Class UI Primitives**:
   - `compose` is a native language modifier (`compose fun`).
   - Slot-table state handles (`State<T>`, `NodeId`) are lightweight `Copy` IDs.
6. **File as Compilation Unit**:
   - Phase 1 extracts declaration ABI hashes; Phase 2 typechecks bodies.
   - Modifying non-generic function bodies produces zero downstream recompilations.

---

## 3. Key Reference Documents ("The Elephant")

- [`docs/VISION.md`](file:///Users/jdemeulenaere/Development/lang/docs/VISION.md): High-level vision, principles, and two-tier execution model.
- [`docs/INVARIANTS.md`](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md): Formal architectural contracts.
- [`docs/ROADMAP.md`](file:///Users/jdemeulenaere/Development/lang/docs/ROADMAP.md): Current phase, progress, and milestone tracking.
- **Living Subsystem Specifications (`docs/specs/`)**:
  - [`package-resolution.md`](file:///Users/jdemeulenaere/Development/lang/docs/specs/package-resolution.md): Zero-marker root discovery, import resolution, and package scanning.
  - [`syntax.md`](file:///Users/jdemeulenaere/Development/lang/docs/specs/syntax.md): Concrete grammar, keyword list, and syntax examples.
  - [`memory-model.md`](file:///Users/jdemeulenaere/Development/lang/docs/specs/memory-model.md): Zero-cost memory, `Heap<T>`, borrow checking, lifetime elision, and UI slot handles.
  - [`compilation-and-abi.md`](file:///Users/jdemeulenaere/Development/lang/docs/specs/compilation-and-abi.md): 2-tier execution, compilation avoidance, ABI hashing, and monomorphization.

---

## 4. Current Milestone & Focus
- **Milestone 1**: Minimal Viable Bootstrap Compiler in Rust (`prismc` / `prism` CLI).
- **Milestone 2**: First-class Jetpack Compose UI engine, reactive tree runtime, and native hot-reload framework.
- **Milestone 3**: Self-hosting compiler written in Prism itself.

When writing or reviewing code, always ensure existing tests pass and new language features include executable test fixtures.
