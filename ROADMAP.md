# Prism Roadmap & Milestones

Following the **Elephant-Goldfish model** via OpenSpec, this roadmap represents the high-level milestone tracking memory. Each phase is broken down into small, verifiable, test-driven work packets in `openspec/changes/`.

---

## Phase 0: Foundations & Architecture (Completed)
- [x] Project Vision & Invariants established ([`README.md`](README.md), [`openspec/specs/system-invariants`](openspec/specs/system-invariants/spec.md)).
- [x] Compilation & Execution Architecture Specification ([`openspec/specs/compilation-and-abi`](openspec/specs/compilation-and-abi/spec.md)).
- [x] Zero-Cost Memory Architecture Specification ([`openspec/specs/memory-model`](openspec/specs/memory-model/spec.md)).
- [x] Package Resolution & Root Discovery Specification ([`openspec/specs/package-resolution`](openspec/specs/package-resolution/spec.md)).
- [x] Syntax & Grammar Specification ([`openspec/specs/syntax`](openspec/specs/syntax/spec.md)).
- [x] AI Pair-Programming Guide ([`AGENTS.md`](AGENTS.md)).
- [x] Initialize Rust workspace (`prism-compiler`):
  - `prism-lexer`: Fast tokenizer.
  - `prism-parser`: Pratt parser + AST generation.
  - `prism-ast`: Abstract Syntax Tree definitions.
  - `prism-abi`: Surface signature extraction & ABI hashing.
  - `prism-cli`: Developer command line interface (`prism`).

---

## Phase 1: Front-End Parser, AST & Package Resolver (Current)
- [ ] Lexer and Parser for basic language subset:
  - Primitive types, `val`/`var`, basic arithmetic, `fun`, `if`, `when`, `return`, `struct`, `enum`.
  - Default `private` visibility, explicit `package` and `public`.
- [x] Package & import resolver conforming to zero-config, zero-marker directory rules (`crates/prism-resolver`).
- [ ] CLI commands: `prism check`, `prism fmt`.
- [ ] Parsing and AST validation test suite.

---

## Phase 2: Type System, Value Ownership & ABI Invalidation
- [ ] Type checker with bidirectional type inference and lifetime elision rules.
- [ ] Value ownership transfer, move semantics, and borrow checker for functions.
- [ ] ABI interface extraction & ABI hashing engine (`.abi` cache).
- [ ] Incremental compilation verification: ensuring body edits produce zero downstream recompilations.

---

## Phase 3: Tier 1 Native Execution & Hot Reload (Cranelift JIT)
- [ ] Intermediate Representation (Prism IR / SSA).
- [ ] Cranelift JIT native code generator.
- [ ] In-memory native execution via `prism run file.prism` (<15ms startup).
- [ ] Hot-reload runtime prototype: dynamic function-pointer patching with state preservation.
- [ ] Integration test suite running executable `.prism` native test fixtures.

---

## Phase 4: First-Class Compose Engine & UI Stack (Milestone 2)
- [ ] Compose runtime library: Slot table, group keys, memoization engine.
- [ ] Compiler `compose fun` transform: synthetic change-tracking bitmasks and slot injection.
- [ ] State handle system (`remember { mutableStateOf(...) }`).
- [ ] Declarative UI prototype with hot-reload rendering (desktop/mobile UI application).
- [ ] Verification of real-world reactive UI apps running natively with zero GC pauses.

---

## Phase 5: Tier 2 Release Backend (LLVM AOT), LSP & Self-Hosting (Milestone 3)
- [ ] Tier 2 LLVM AOT native code generator (`prism build --release`) with whole-program optimization and LTO.
- [ ] Prism Language Server Protocol (LSP) for VS Code / IDEs.
- [ ] Port bootstrap Rust compiler to native Prism.
- [ ] Full self-hosting compiler compilation and bootstrap verification.
