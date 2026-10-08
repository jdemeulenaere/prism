# Prism Roadmap & Milestones

Following the **Elephant-Goldfish model**, this roadmap represents the long-term tracking memory (The Elephant). Each phase is broken down into small, verifiable, test-driven execution units (The Goldfish).

---

## Phase 0: Foundations & Architecture (Current)
- [x] Project Vision & Invariants established (`VISION.md`, `INVARIANTS.md`).
- [x] ADR-0001: Bootstrap in Rust & 3-Tier Execution Engine.
- [x] ADR-0002: Zero-Cost Memory Architecture (Zero GC, Zero ARC, Data-Oriented Handles, Box).
- [x] ADR-0003: Zero-Config Conventions, Module Resolution & ABI Invalidation.
- [x] Initial Syntax Specification (`SYNTAX_SPEC.md`).
- [x] AI Pair-Programming Guide (`AGENTS.md`).
- [ ] Initialize Rust workspace (`prism-compiler`):
  - `prism-lexer`: Fast tokenizer.
  - `prism-parser`: Pratt parser + AST generation.
  - `prism-ast`: Abstract Syntax Tree definitions.
  - `prism-abi`: Surface signature extraction & ABI hashing.
  - `prism-cli`: Developer command line interface (`prism`).

---

## Phase 1: Minimal Viable Bootstrap Compiler (Tier 1: Interpreter)
- [ ] Lexer and Parser for basic language subset:
  - Primitive types, `val`/`var`, basic arithmetic, `fun`, `if`, `when`, `return`, `struct`, `enum`.
  - Default `private` visibility, explicit `package` and `public`.
- [ ] Package & import resolver conforming to zero-config, zero-marker directory rules.
- [ ] AST / Bytecode Interpreter (`prism run file.prism`).
- [ ] CLI commands: `prism run`, `prism check`, `prism fmt`.
- [ ] Integration test suite running executable `.prism` fixtures.

---

## Phase 2: Type System, Value Ownership & ABI Invalidation
- [ ] Type checker with bidirectional type inference and lifetime elision rules.
- [ ] Value ownership transfer, move semantics, and borrow checker for functions.
- [ ] ABI interface extraction & ABI hashing engine (`.abi` cache).
- [ ] Incremental compilation verification: ensuring body edits produce zero downstream recompilations.

---

## Phase 3: Native Backend & Hot Reload (Tier 2: Cranelift)
- [ ] Intermediate Representation (Prism IR / SSA).
- [ ] Cranelift JIT code generation backend.
- [ ] Native execution via `prism run --native file.prism`.
- [ ] Hot-reload runtime prototype: dynamic module reloading with state preservation.

---

## Phase 4: First-Class Compose Engine & UI Stack (Milestone 2)
- [ ] Compose runtime library: Slot table, group keys, memoization engine.
- [ ] Compiler `compose fun` transform: synthetic change-tracking bitmasks and slot injection.
- [ ] State handle system (`remember { mutableStateOf(...) }`).
- [ ] Declarative UI prototype with hot-reload rendering (desktop/mobile UI application).
- [ ] Verification of real-world reactive UI apps running natively with zero GC pauses.

---

## Phase 5: Self-Hosting Compiler & LSP (Milestone 3)
- [ ] Prism Language Server Protocol (LSP) for VS Code / IDEs.
- [ ] Port bootstrap Rust compiler to native Prism.
- [ ] Full self-hosting compiler compilation and bootstrap verification.
