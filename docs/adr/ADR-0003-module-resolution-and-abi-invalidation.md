# ADR-0003: Convention-over-Configuration Module Resolution and ABI Invalidation

## Status
Accepted

## Context
Prism aims for sub-second incremental compile times and a zero-configuration build experience.
Traditional build systems (CMake, Gradle, Cargo) require manual configuration files (`Cargo.toml`, `build.gradle.kts`, `CMakeLists.txt`), which slow down project bootstrapping and create maintenance overhead.
Furthermore, changing a private implementation detail in one file often triggers cascading recompilations across entire projects in naive build systems.

## Decision

### 1. Zero-Config Directory Conventions
- Source files use the `.pm` extension.
- Package declarations map directly to directory paths:
  - `package foo.bar` $\iff$ file must reside in `<workspace_root>/foo/bar/*.pm`.
  - An empty package statement means the file is in `<workspace_root>/` and can only use relative imports.
- Import resolution rules:
  - `import doo`: Resolves to sibling file `doo.pm` in the current package directory.
  - `import //abc/xyz/util`: Resolves to `<workspace_root>/abc/xyz/util.pm` (the `//` prefix anchors to the workspace root).
- Workspace root is automatically discovered by locating `.git` or an optional `.prism-root` marker file.

### 2. Default Visibility
- Default symbol visibility is `package` (accessible by any file sharing the same `package` declaration).
- `public`: Accessible across any package via imports.
- `private`: Accessible only within the declaring `.pm` file.

### 3. File as Independent Compilation Unit & Two-Phase ABI Extraction
Each `.pm` file compiles independently according to a two-phase pipeline:
1. **Phase 1: Header / Surface Signature Extraction**:
   - The compiler lexes and parses declarations (`struct`, `class`, `fun`, `val`, `enum`, signatures).
   - Private members and function bodies are skipped or hashed separately.
   - Computes an **ABI Hash** representing the public and package interface of the file.
2. **Phase 2: Body Typechecking and Codegen**:
   - Compiles function bodies against the ABI signatures of its imported dependencies.
   - Downstream files depend **strictly** on the ABI hash of their imports.

### 4. Compilation Avoidance Rule
- If file `A.pm` is edited such that only private functions or function bodies change, its exported **ABI hash remains identical**.
- Downstream file `B.pm` (which imports `A.pm`) is **never re-typechecked or recompiled**.
- The cache key for `B.pm` is `Hash(Content(B.pm) + TargetConfig + ABI_Hash(A.pm))`.

## Consequences
- **Positive**:
  - No build files required to start coding or running a project.
  - Massive compilation avoidance during typical inner-loop development (editing function bodies).
  - Clean separation of interface and implementation.
- **Negative / Tradeoffs**:
  - Requires strict validation that directory path matches `package` statement (compiler raises an error if mismatched).
  - External 3P dependencies will need a clear conventions-based scheme (to be decided in a future ADR).
