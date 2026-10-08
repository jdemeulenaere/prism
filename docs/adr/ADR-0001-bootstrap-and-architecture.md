# ADR-0001: Bootstrap Compiler in Rust and Two-Tier Native Execution

## Context
We are creating a new statically typed programming language with Kotlin-like ergonomics, Rust/C++ performance, and first-class Compose-like UI trees.
The compiler needs to be fast, safe, incremental, and provide sub-second development loops, instant scripting, and high-performance native binaries.
An initial consideration of a separate interpreter was evaluated and rejected: maintaining both an interpreter and native codegen creates severe semantic drift and duplicates runtime maintenance, while Cranelift JIT already compiles native machine code directly to memory in 5–15 milliseconds.

## Decision
1. **Bootstrap Implementation Language**: We will write the first compiler (`prismc`) and CLI (`prism`) in **Rust**.
   - Rust offers memory safety, zero-cost abstractions, excellent performance, rich compiler libraries (`logos`, `rowan` / `chumsky` / hand-written Pratt, `cranelift`), and great cross-compilation support.
2. **Two-Tier Native Execution Architecture**:
   - **Tier 1 (Cranelift JIT & Dylib Backend)**: Built into the compiler binary for instant scripting (`prism run`), unit testing, and sub-second function-pointer hot reloading during UI development. Compiles directly into memory in milliseconds.
   - **Tier 2 (LLVM AOT / Release Backend)**: Highly optimized native compiler for production binaries with whole-program optimization and Link-Time Optimization (LTO).
3. **Compilation Unit Model**:
   - Every file is an individual compilation unit.
   - Signature extraction (declarations only) is decoupled from body type checking to support compilation avoidance via ABI hashing.

## Consequences
- **Positive**:
  - Fast developer feedback loops (<15ms) without the maintenance overhead or semantic drift of a separate interpreter.
  - Native machine code execution and identical memory layouts from Day 1.
  - Rust ecosystem provides production-ready JIT/codegen infrastructure (`cranelift`).
  - Strict separation of ABI and implementation guarantees fast incremental builds from day one.
- **Negative / Tradeoffs**:
  - Two compiler implementations over time (Bootstrap in Rust -> Self-hosted compiler).
  - Tier 1 relies on Cranelift platform target support (Tier 1 platforms: x86_64, aarch64/ARM64).
