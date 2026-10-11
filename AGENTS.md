# Prism Agent Guidelines (The Elephant & Goldfish Operating Manual)

Welcome to the **Prism** language repository. This document serves as the primary onboarding directive for AI agents and human contributors.

---

## 1. The Core Philosophy: The Elephant & Goldfish Model via OpenSpec
This project strictly follows the **Elephant-Goldfish model** (see [Rensin, 2024](https://drensin.medium.com/elephants-goldfish-and-the-new-golden-age-of-software-engineering-c33641a48874)) using [OpenSpec](https://github.com/Fission-AI/openspec):
- **The Elephant (Durable Memory)**: All architectural invariants, grammar specifications, decisions, and roadmaps are permanently documented in [`openspec/specs/`](file:///Users/jdemeulenaere/Development/lang/openspec/specs). When starting a task, **always consult the Elephant specs** to ensure alignment.
- **The Goldfish (Ephemeral Execution)**: Tasks are proposed and executed via [`openspec/changes/<change-name>/`](file:///Users/jdemeulenaere/Development/lang/openspec/changes) work packets:
  - `proposal.md`: Motivation and scope boundary.
  - `design.md`: Technical approach, trade-offs, and design decisions (eliminates separate ADR drift).
  - `tasks.md`: Atomic, test-driven checklist.
  - `specs/`: Delta specifications with concrete scenarios (WHEN/THEN).
  - Once verified, the change is archived to `openspec/changes/archive/` and its delta specs merge into `openspec/specs/`.
- **OpenSpec Workflows / Slash Commands**:
  - `/opsx-propose "change-name"`: Creates and populates the change packet.
  - `/opsx-apply`: Executes the tasks against the spec.
  - `/opsx-archive`: Merges deltas and archives the change.
  - `/opsx-sync`: Syncs delta specs from a change into main specs without archiving.
  - `/opsx-explore`: Interactive thinking partner for clarifying ideas before drafting specs.
  - `/opsx-update`: Revises existing planning artifacts to reconcile changes.

---

## 2. Architectural Invariants Quick-Sheet (Non-Negotiable)

Any proposed code, grammar, or compiler change must uphold these rules (detailed in [`openspec/specs/system-invariants/spec.md`](file:///Users/jdemeulenaere/Development/lang/openspec/specs/system-invariants/spec.md)):

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

- [`README.md`](file:///Users/jdemeulenaere/Development/lang/README.md): High-level vision, principles, and two-tier execution model.
- [`ROADMAP.md`](file:///Users/jdemeulenaere/Development/lang/ROADMAP.md): Current phase, progress, and milestone tracking.
- **Living Subsystem Specifications (`openspec/specs/`)**:
  - [`package-resolution`](file:///Users/jdemeulenaere/Development/lang/openspec/specs/package-resolution/spec.md): Zero-marker root discovery, import resolution, and package scanning.
  - [`syntax`](file:///Users/jdemeulenaere/Development/lang/openspec/specs/syntax/spec.md): Concrete grammar, keyword list, and syntax examples.
  - [`memory-model`](file:///Users/jdemeulenaere/Development/lang/openspec/specs/memory-model/spec.md): Zero-cost memory, `Heap<T>`, borrow checking, lifetime elision, and UI slot handles.
  - [`compilation-and-abi`](file:///Users/jdemeulenaere/Development/lang/openspec/specs/compilation-and-abi/spec.md): 2-tier execution, compilation avoidance, ABI hashing, and monomorphization.
  - [`system-invariants`](file:///Users/jdemeulenaere/Development/lang/openspec/specs/system-invariants/spec.md): Formal requirements and scenarios for all 6 non-negotiable invariants.

---

## 4. Current Milestone & Focus
- **Milestone 1**: Minimal Viable Bootstrap Compiler in Rust (`prismc` / `prism` CLI).
- **Milestone 2**: First-class Jetpack Compose UI engine, reactive tree runtime, and native hot-reload framework.
- **Milestone 3**: Self-hosting compiler written in Prism itself.

When writing or reviewing code, always ensure existing tests pass and new language features include executable test fixtures.
