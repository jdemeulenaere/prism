# ADR-0003: Convention-over-Configuration Module Resolution and ABI Invalidation

## Status
Accepted

## Context
Prism aims for sub-second incremental compile times and a zero-configuration build experience.
Traditional build systems (CMake, Gradle, Cargo) require manual configuration files (`Cargo.toml`, `build.gradle.kts`, `CMakeLists.txt`), which slow down project bootstrapping and create maintenance overhead.
Furthermore, changing a private implementation detail in one file often triggers cascading recompilations across entire projects in naive build systems.

## Decision

### 1. Zero-Config Directory Conventions & Syntax
- Source files use the `.prism` extension.
- Package declarations map directly to directory paths:
  - `package foo.bar` $\iff$ file must reside in `<workspace_root>/foo/bar/*.prism`.
  - An empty package statement means the file is in `<workspace_root>/` and can only use relative imports.
- Import resolution rules:
  - `import doo`: Resolves to sibling file `doo.prism` in the current package directory.
  - `import /abc/xyz/util`: Resolves to `<workspace_root>/abc/xyz/util.prism` (a leading `/` anchors to the workspace root, matching standard POSIX filesystem root notation without colliding with `//` comments).
- Workspace root is automatically discovered by locating `.git`, a `.prism-root` marker file, or falling back to the compiler invocation directory.

### 2. Default Visibility & Explicit Signatures
- Default symbol visibility is `package` (accessible by any file sharing the same `package` directory).
- `public`: Accessible across any package via imports.
- `private`: Accessible only within the declaring `.prism` file.
- **Explicit Type Rule**: To enable independent Phase 1 ABI extraction without inspecting function bodies, all `public` and `package`-visible declarations must declare explicit parameter and return types. Type inference is permitted only within function bodies and `private` declarations.

### 3. File as Independent Compilation Unit & Two-Phase Pipeline
1. **Phase 1: Package Declaration Extraction & ABI Hashing**:
   - The compiler lexes and parses surface declarations (`struct`, `class`, `fun`, `val`, `enum`, signatures) across all `.prism` files in the target package.
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
  - No build files required to start coding or running a project.
  - Massive compilation avoidance during inner-loop development.
  - Unambiguous syntax (`import /core/fmt` avoids `//` comment collisions).
  - Clean separation of interface and implementation.
- **Negative / Tradeoffs**:
  - Requires directory paths to match `package` statements.
  - Generics require callers to re-monomorphize when their implementation changes.
