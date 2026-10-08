# Prism Compilation Pipeline, ABI Hashing & Execution Tiers Specification

**Status**: Living Architecture Specification (The Elephant)  
**Applies To**: Compiler Driver, CLI (`prism`), ABI Extractor, and Code Generation Backends  
**Governing Invariants**: [Invariant 1 (`docs/INVARIANTS.md`)](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md#L7), [Invariant 3 (`docs/INVARIANTS.md`)](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md#L22), [Invariant 6 (`docs/INVARIANTS.md`)](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md#L40)

---

## 1. Overview & Architectural Goals

Prism delivers **sub-second incremental compile times** (<15ms for Tier 1 script execution and hot reloads) while generating **bare-metal optimized binaries** matching C++/Rust speed for release builds.

To achieve this without build configuration files or cascading recompilation storms, Prism enforces:
1. **Single File as Compilation Unit**: Every `.prism` source file is an independently compiled unit with clear interface boundaries.
2. **Two-Phase Compilation Pipeline**: Phase 1 extracts exported ABI interfaces and interface hashes; Phase 2 compiles bodies in parallel against the imported interface hashes.
3. **Compilation Avoidance via ABI Stability**: Modifying a function body or private helper never invalidates downstream files.
4. **Two-Tier Native Execution Engine**: Tier 1 (Cranelift JIT) for development velocity and Tier 2 (LLVM AOT) for release optimization.

---

## 2. Visibility Model & Explicit ABI Typing

### 2.1. Visibility Rules
- **Default Visibility is `private`**: Any declaration without an explicit visibility modifier is private to its source file.
- `package`: Accessible to any file sharing the exact same package directory.
- `public`: Accessible across any package via module imports.

### 2.2. Explicit Type Rule on Exports
To guarantee independent Phase 1 ABI extraction without inspecting or type-checking function bodies:
- **All `public` and `package`-visible declarations must declare explicit parameter and return types**.
- Type inference is strictly permitted only for `private` declarations and local variable bindings within function bodies.

---

## 3. Two-Phase Compilation Pipeline

```
+--------------------------------------------------------------+
| Phase 1: Surface Declaration & ABI Extraction                |
| - Fast lex and surface parse across source files             |
| - Extract struct definitions, enums, function signatures      |
| - Compute authoritative exported ABI Hash (.abi)             |
+--------------------------------------------------------------+
                                |
                                v
+--------------------------------------------------------------+
| Phase 2: Function Body Lowering & Codegen                    |
| - Typecheck function bodies against imported ABI signatures   |
| - Generate machine code (Cranelift JIT or LLVM IR)           |
| - Cache compilation artifacts keyed by (file_content_hash,    |
|   imported_abi_hashes, compiler_version)                     |
+--------------------------------------------------------------+
```

### 3.1. Authoritative ABI Hash Computation
The exported ABI hash of a `.prism` file is a cryptographic hash (BLAKE3) of:
1. All `public` and `package`-visible symbol names and visibilities.
2. Field names, order, and resolved type signatures of exported structs and enums.
3. Parameter names, order, parameter types, and return types of exported functions.
4. Monomorphization AST hashes of exported generic functions (see Section 4).

**Excluded from the ABI Hash**:
- All `private` declarations, structs, and helper functions.
- Non-generic function bodies and implementation details.
- Comments, whitespace, and formatting.

### 3.2. Strict Compilation Avoidance
When compiling a project:
1. If file `A.prism` changes, its Phase 1 ABI is computed.
2. If `A.prism`'s ABI hash is identical to its cached hash (e.g. only private helpers or function bodies were edited), **zero downstream files that import `A.prism` are recompiled**.
3. The compiler re-lowers only `A.prism`.

---

## 4. Generics & Monomorphization Invalidation

Prism employs compile-time monomorphization of generic functions and types to guarantee zero runtime dispatch overhead and unboxed memory layouts.

### 4.1. Monomorphization Hash
Because generic functions must be instantiated into machine code by their downstream call sites:
- The exported ABI of a generic function records a **Monomorphization Hash** computed from the normalized AST of the generic function's body.
- If a generic function's body changes, the Monomorphization Hash changes, triggering downstream files that instantiate that generic specialization to re-monomorphize.
- Non-generic functions never produce downstream invalidations.

---

## 5. Two-Tier Native Execution Engine

Prism implements a two-tier execution engine to decouple inner-loop developer velocity from production runtime throughput.

```
                      +-------------------+
                      |   Prism Source    |
                      +-------------------+
                                |
               +----------------+----------------+
               |                                 |
               v                                 v
     [ Tier 1: Cranelift JIT ]         [ Tier 2: LLVM AOT ]
     - Instant script execution        - Release builds (prism build --release)
     - Sub-second hot reload (<15ms)   - Whole-program optimization & LTO
     - Cross-file inlining disabled    - Cross-file inlining enabled
```

### 5.1. Tier 1: Cranelift JIT & Dylib Backend
- Embedded directly inside the `prism` compiler CLI binary without external heavy toolchain dependencies.
- **Supported Tier 1 Architectures**: `x86_64` and `aarch64` (ARM64).
- Compiles native machine code directly to memory in 5–15 milliseconds.
- Used for `prism run <file.prism>`, test execution (`prism test`), and interactive UI hot reloading.
- **Inlining Rule**: Cross-file inlining across compilation units is strictly disabled in Tier 1 to preserve 100% compilation avoidance and enable dynamic function-pointer table swapping during live hot reload.

### 5.2. Tier 2: LLVM AOT Release Backend
- Invoked via `prism build --release`.
- Generates optimized machine code with vectorization, aggressive inlining across compilation units, and Link-Time Optimization (LTO).
- Matches or exceeds standard C++ (`clang -O3`) and Rust (`rustc --release`) runtime throughput.

---

## 6. Design Rationale & Tradeoffs

| Architecture Choice | Alternative Considered | Why Rejected / Prism Choice |
| :--- | :--- | :--- |
| **Two-Tier Engine (Cranelift + LLVM)** | Single Interpreter + LLVM | Maintaining an AST or bytecode interpreter creates severe semantic drift, duplicates the runtime model, and requires maintaining two memory representations. Cranelift compiles native machine code in milliseconds, giving native speed from day 1. |
| **Default `private` Visibility** | Default `public` (Python/JS) or Default `package` (Java/Kotlin) | Default `private` prevents unintentional API leaks, keeps ABI hashes minimal, and maximizes compilation avoidance. |
| **Explicit Types on Exports** | Full Global Type Inference (Haskell/OCaml) | Global type inference couples function bodies to module signatures, forcing whole-project re-typechecking whenever any function body changes. Explicit types decouple Phase 1 ABI extraction from Phase 2 body checking. |
