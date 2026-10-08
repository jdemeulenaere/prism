# ADR-0001: Bootstrap Compiler in Rust and Three-Tier Execution

## Context
We are creating a new statically typed programming language with Kotlin-like ergonomics, Rust/C++ performance, and first-class Compose-like UI trees.
The compiler needs to be fast, safe, incremental, and provide both interpreter and native code generation modes.

## Decision
1. **Bootstrap Implementation Language**: We will write the first compiler (`langc`) and CLI (`lang`) in **Rust**.
   - Rust offers memory safety, zero-cost abstractions, excellent performance, rich compiler libraries (`logos`, `rowan` / `chumsky` / hand-written Pratt, `cranelift`), and great cross-compilation support.
2. **Execution Architecture**:
   - **Tier 1 (AST / Bytecode Interpreter)**: Built into the compiler binary for instantaneous scripting, formatting, and unit testing.
   - **Tier 2 (Cranelift Backend)**: Fast native code generator for debug builds and hot reload.
   - **Tier 3 (LLVM / Release Backend)**: Highly optimized release compiler for production binaries.
3. **Compilation Unit Model**:
   - Every file is an individual compilation unit.
   - Signature extraction (declarations only) is decoupled from body type checking to support compilation avoidance via ABI hashing.

## Consequences
- **Positive**:
  - Fast developer feedback loops during early language development.
  - Rust ecosystem provides production-ready JIT/codegen infrastructure (`cranelift`).
  - Strict separation of ABI and implementation guarantees fast incremental builds from day one.
- **Negative / Tradeoffs**:
  - Two compiler implementations over time (Bootstrap in Rust -> Self-hosted compiler).
  - Maintaining consistency between Interpreter and Native backends requires a shared IR (Intermediate Representation) and compliance test suite.
