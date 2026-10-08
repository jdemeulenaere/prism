# ADR-0003: Convention-over-Configuration Module Resolution and ABI Invalidation

## Status
Accepted

## Context
Prism aims for sub-second incremental compile times and a zero-configuration build experience.
Traditional build systems (CMake, Gradle, Cargo) require manual configuration files (`Cargo.toml`, `build.gradle.kts`, `CMakeLists.txt`), which slow down project bootstrapping and create maintenance overhead.
Furthermore, changing a private implementation detail in one file often triggers cascading recompilations across entire projects in naive build systems.

## Decision

### 1. Zero-Config Directory Conventions & Pure Path Root Discovery
- Source files use the `.prism` extension.
- Package declarations map directly to directory paths:
  - `package foo.bar` $\iff$ file must reside in `<workspace_root>/foo/bar/*.prism`.
  - An empty package statement means the file is in `<workspace_root>/` and can only use relative imports.
- **Zero-Marker Workspace Root Discovery**:
  - The workspace root is discovered strictly from the path of the entry file and its `package` declaration.
  - No marker files (`.prism-root`, `.git`, etc.) are needed or expected.
  - Given file `/workspace/projects/demo/foo/bar/app.prism` with `package foo.bar`, the compiler strips the package segment `foo/bar` from the directory path to obtain the workspace root `/workspace/projects/demo`.
- Import resolution rules:
  - `import doo`: Resolves to sibling file `doo.prism` in the current package directory.
  - `import /abc/xyz/util`: Resolves to `<workspace_root>/abc/xyz/util.prism` (a leading `/` anchors to the discovered workspace root, matching standard POSIX root notation without colliding with `//` comments).

### 2. Default Visibility: `private`
- **Default visibility is `private`**: Any declaration without an explicit visibility modifier is private to its source file.
- `package`: Accessible by any file sharing the same `package` directory.
- `public`: Accessible across any package via imports.
- **Explicit Type Rule**: To enable independent Phase 1 ABI extraction without inspecting function bodies, all `public` and `package`-visible declarations must declare explicit parameter and return types. Type inference is permitted only within function bodies and `private` declarations.

### 3. File as Independent Compilation Unit & Two-Phase Pipeline
1. **Phase 1: Package Declaration Extraction & ABI Hashing**:
   - The compiler lexes and parses surface declarations (`struct`, `fun`, `val`, `enum`, signatures) across all `.prism` files in the target package.
   - Computes an **ABI Hash** representing the public and package interface of each file.
2. **Phase 2: Body Typechecking and Codegen**:
   - Compiles individual file bodies against the package symbol table and imported ABI signatures.
   - Downstream files depend strictly on the ABI hashes of their imports.

### 4. Generics & Monomorphization Invalidation
- Generics are monomorphized at compile-time for zero-cost runtime performance.
- The exported ABI of a generic function includes a **Monomorphization Hash** of its AST body.
- If a generic function's body changes, downstream files instantiating that generic specialization must re-monomorphize. Non-generic functions maintain strict compilation avoidance.

### 5. Inlining Decoupled Across Tiers
- **Tier 1 (Interpreter) & Tier 2 (Cranelift Debug / Hot Reload)**: Cross-file inlining across compilation units is disabled to guarantee maximum compilation avoidance and predictable hot reload patching.
- **Tier 3 (LLVM Release)**: Cross-file inlining is enabled via Link-Time Optimization (LTO).

## Consequences
- **Positive**:
  - Zero build files and zero marker files required to start coding, test, or run a project.
  - Root discovery is 100% deterministic and portable across containers, monorepos, and subfolders.
  - Safe by default: private members cannot accidentally leak into package or public APIs.
  - Massive compilation avoidance during inner-loop development.
- **Negative / Tradeoffs**:
  - Requires directory paths to match `package` statements.
  - Generics require callers to re-monomorphize when their implementation changes.
