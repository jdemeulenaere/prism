# Prism Roadmap & Milestones

Following the **Elephant-Goldfish model** via OpenSpec, this roadmap represents the high-level milestone tracking memory. Each phase is broken down into small, verifiable, test-driven work packets in `openspec/changes/`.

- [`openspec/specs/`](openspec/specs) describes only what is **implemented**.
- [`docs/architecture.md`](docs/architecture.md) describes what is **planned** for the unchecked items below. A change that implements a planned item moves its requirements from there into the specs.

---

## Phase 0: Foundations & Architecture (Completed)
- [x] Project Vision & Invariants established ([`README.md`](README.md), [`openspec/specs/system-invariants`](openspec/specs/system-invariants/spec.md)).
- [x] Compilation & Execution Architecture design ([`docs/architecture.md`](docs/architecture.md) §1–2).
- [x] Zero-Cost Memory Architecture design ([`docs/architecture.md`](docs/architecture.md) §3).
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

## Phase 1: Front-End Parser, AST & Package Resolver
- [x] Lexer and Parser for basic language subset:
  - Primitive types, `val`/`var`, basic arithmetic, `fun`, `if`, `when`, `return`, `struct`, `enum`.
  - Default `private` visibility, explicit `package` and `public`.
- [x] Package & import resolver conforming to zero-config, zero-marker directory rules (`crates/prism-resolver`).
- [x] CLI multi-file workspace and package checking (`prism check`).
- [x] Parsing and AST validation test suite.
- [ ] Code formatting command (`prism fmt`).

---

## Phase 2: Type System, Value Ownership & ABI Invalidation (Current)
- [x] Type checker with bidirectional type inference, nominal types, scoping, and diagnostics (`crates/prism-typechecker`).
- [x] ABI surface extraction and deterministic ABI hashing (`crates/prism-abi`, [`compilation-and-abi`](openspec/specs/compilation-and-abi/spec.md)).
- [ ] Value ownership transfer, move semantics, borrow checker, and the 4 lifetime elision rules for functions ([`docs/architecture.md`](docs/architecture.md) §3).
- [ ] Decide the authoritative ABI hash algorithm: BLAKE3 (original design) or the current FNV-1a ([`docs/architecture.md`](docs/architecture.md) §1).
- [ ] `.abi` cache and compilation avoidance ([`docs/architecture.md`](docs/architecture.md) §1).
- [ ] Incremental compilation verification: ensuring body edits produce zero downstream recompilations.
- [ ] Compound assignment (`+=`, `-=`) and bitwise operators (deferred by the `lexer-parser` change).
- [ ] Generics: type parameter checking and monomorphization (deferred by `type-checker-core`).
- [ ] Reject unknown type names in annotations (`E4001`). This needs a builtin type list, including sized integers like `UInt64`, plus generic scoping; until then undeclared names are accepted as opaque nominal types (deferred by `resolve-nominal-type-annotations`).
- [ ] Qualified type paths such as `models.User`; imports currently bring symbols in by bare name (deferred by `resolve-nominal-type-annotations`).
- [ ] Fix: calling a struct-typed value (`p()` where `p: Point`) goes through the constructor branch of `infer_call_expr` (noted in `resolve-nominal-type-annotations`).
- [ ] Decide whether safe-navigation assignment (`a?.b = c`, allowed in Kotlin) is valid; it is currently rejected (from the parser plain fixes, `ecc5da3`).
- [ ] Revisit the parser's 256-level nesting limit (E2002) before the LSP: a debug build needs about 2.75 MB of stack at the limit, more than a default 2 MB worker thread (from `8f1f932`).
- [ ] Unify the missing-return-type rule: `prism-abi` accepts a block-bodied exported function without a return type as `Unit` and only rejects expression bodies (`E3001`), while the type checker rejects both (`E4011`). Tighten the ABI rule or relax the specs (open question in `resolve-nominal-type-annotations`).
- [ ] Reject recursive struct and enum layouts that lack `Heap<T>` indirection (deferred by `fix-cli-workspace-audit`).
- [ ] Parser error recovery: report several syntax errors per file instead of stopping at the first, needed for the LSP (deferred by `lexer-parser`).
- [ ] Traits or interfaces for polymorphism and dynamic dispatch; not designed yet (deferred by `type-checker-core`).

---

## Phase 3: Tier 1 Native Execution & Hot Reload (Cranelift JIT)
- [ ] Intermediate Representation (Prism IR / SSA).
- [ ] Cranelift JIT native code generator ([`docs/architecture.md`](docs/architecture.md) §2, §4).
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
