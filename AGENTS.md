# Prism Agent Guidelines (The Elephant & Goldfish Operating Manual)

Welcome to the **Prism** language repository. This document serves as the primary onboarding directive for AI agents and human contributors.

---

## 1. The Core Philosophy: The Elephant & Goldfish Model via OpenSpec
This project strictly follows the **Elephant-Goldfish model** (see [Rensin, 2024](https://drensin.medium.com/elephants-goldfish-and-the-new-golden-age-of-software-engineering-c33641a48874)) using [OpenSpec](https://github.com/Fission-AI/openspec):
- **The Elephant (Durable Memory)**: All architectural invariants, grammar specifications, decisions, and roadmaps are permanently documented in [`openspec/specs/`](openspec/specs). When starting a task, **always consult the Elephant specs** to ensure alignment.
- **The Goldfish (Ephemeral Execution)**: Tasks are proposed and executed via [`openspec/changes/<change-name>/`](openspec/changes) work packets:
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
  - Claude Code uses a colon instead of a dash (`/opsx:propose`, `/opsx:apply`, ...); its skills and commands live in `.claude/`, while `.agents/` holds the Antigravity copies. Refresh both with `openspec update`; to change which tools are generated, run `openspec init --tools claude,antigravity`.

---

## 2. Architectural Invariants (Non-Negotiable)

Every code, grammar, or compiler change must uphold these. The normative text, with scenarios, is [`openspec/specs/system-invariants/spec.md`](openspec/specs/system-invariants/spec.md); keep this list as a one-line summary of it.

1. **Single-file compilation units with explicit export boundaries**: each `.prism` file compiles independently. Default visibility is `private`; `package` is visible within the package directory, `public` across packages. `public` and `package` declarations must declare explicit parameter and return types.
2. **Zero build files, zero marker files**: the workspace root comes from matching the file's path to its `package` declaration. Root-anchored imports use a leading slash (`import /core/fmt`), sibling imports a plain name (`import doo`).
3. **Deterministic hermetic builds and compilation avoidance**: a file's artifact depends only on its content, its direct imports' ABI hashes, the compiler version and the target. Phase 1 extracts ABI hashes, Phase 2 checks bodies; editing a non-generic body causes zero downstream recompilation.
4. **Kotlin-like syntax without ambiguity**: `fun`, `val`/`var`, `struct`, `enum`, `when`, `if` and `T?`, parsed deterministically.
5. **Zero-cost memory safety**: zero GC, zero hidden ARC, no lifetime annotations. Single-owner values, `Heap<T>` for recursion, stack-scoped borrows with elision, typed handles for graphs.
6. **First-class compose and hot reload**: `compose fun` is a native modifier; slot-table handles (`State<T>`, `NodeId`) are `Copy` IDs; the Tier 1 JIT swaps composable bodies while keeping UI state.

In code comments and diagnostics, cite an invariant by name ("the zero-marker invariant"), not by number.

---

## 3. Key Reference Documents ("The Elephant")

What is implemented and what is planned live in different places. Never treat a planned design as a description of the code.

- **Implemented behavior: [`openspec/specs/`](openspec/specs)**, the source of truth for what the compiler does today. Each spec's `## Design Rationale` section records why past decisions were made:
  - [`package-resolution`](openspec/specs/package-resolution/spec.md): Zero-marker root discovery, import resolution, and package scanning.
  - [`syntax`](openspec/specs/syntax/spec.md): Concrete grammar, tokens, and language constructs.
  - [`type-system`](openspec/specs/type-system/spec.md): Bidirectional typing, Nothing bottom type, and nominal types.
  - [`compilation-and-abi`](openspec/specs/compilation-and-abi/spec.md): Phase 1 ABI extraction, deterministic ABI hashing, and monomorphization hashes.
  - [`diagnostics`](openspec/specs/diagnostics/spec.md): The catalog of every compiler error code. A change that adds an error adds its code here.
  - [`system-invariants`](openspec/specs/system-invariants/spec.md): The 6 non-negotiable invariants. These are design constraints, so they also cover subsystems not built yet.
- **Planned design: [`docs/architecture.md`](docs/architecture.md)**: code generation, compilation avoidance, execution tiers, the memory model and borrow checker, and the compose engine. A change that implements an item moves its requirements into `openspec/specs/` as delta specs and removes them from this document.
- **Progress: [`ROADMAP.md`](ROADMAP.md)**: current phase and milestones. [`README.md`](README.md) is the high-level vision.
- **History: `openspec/changes/archive/`**: completed change packets. Historical only and never authoritative: their delta specs and task lists are snapshots that may since have been superseded. Do not read them for current behavior. Consult them only to research why a past decision was made.

---

## 4. Current Milestone & Focus
- **Milestone 1**: Minimal Viable Bootstrap Compiler in Rust (`prismc` / `prism` CLI).
- **Milestone 2**: First-class Jetpack Compose UI engine, reactive tree runtime, and native hot-reload framework.
- **Milestone 3**: Self-hosting compiler written in Prism itself.

When writing or reviewing code, always ensure existing tests pass and new language features include executable test fixtures.
