# Prism Package Resolution & Zero-Marker Root Discovery Specification

**Status**: Living Architecture Specification (The Elephant)  
**Applies To**: Milestone 1, Phase 1 (`prism-resolver`, `prism-cli`, and runtime loaders)  
**Governing Invariants**: [Invariant 2 (`docs/INVARIANTS.md`)](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md#L15), [ADR-0003 (`docs/adr/ADR-0003`)](file:///Users/jdemeulenaere/Development/lang/docs/adr/ADR-0003-module-resolution-and-abi-invalidation.md)

---

## 1. Overview & Architectural Principles

Prism adopts a strict **convention-over-configuration** filesystem contract designed to eliminate build configuration files (`Cargo.toml`, `build.gradle.kts`, `CMakeLists.txt`) and **eliminate marker files** (`.git`, `.prism-root`, `package.json`).

The filesystem hierarchy and source code declarations form a bidirectional, self-authenticating contract:
`Directory Hierarchy` <===> `Package Hierarchy`

This document serves as the permanent reference specification (**The Elephant**) for implementing and verifying the package resolver (`prism-resolver`).

---

## 2. Formal Specification & Algorithms

### 2.1. Pure Path Workspace Root Discovery

#### Algorithm
Given:
- Source file path `P = dir_1/dir_2/.../dir_m/filename.prism`
- Declared package path segments `K = [k_1, k_2, ..., k_n]` (e.g. `["foo", "bar"]` for `package foo.bar`)
- Let `dir = parent(P)` be the directory containing `P`.

1. **Path Canonicalization & Normalization**:
   - Normalize `P` to resolve relative components (`.`, `..`), redundant separators, and canonicalize path separators (handling POSIX `/` and Windows `\`).
   - Obtain absolute directory path `dir = parent(P)`.

2. **Empty Package (Root Files)**:
   - If `K` is empty (the file omits `package` or declares empty package):
     `WorkspaceRoot(P) = dir`
   - The file resides directly in the workspace root. It may only use sibling relative imports.

3. **Non-Empty Package**:
   - If `n > m` (the package path has more segments than the filesystem path):
     ==> Emits diagnostic `Error: PackagePathMismatch`.
   - Compare the trailing `n` directory segments of `dir` against `K`:
     Every segment `dir[m - n + i]` must exactly equal `k_i`.
   - If all segments match:
     `WorkspaceRoot(P)` is obtained by stripping the trailing `n` segments from `dir`.
   - If any segment does not match:
     ==> Emits diagnostic `Error: PackagePathMismatch`.

#### Concrete Example
Given file at `/workspace/projects/demo/foo/bar/app.prism` with `package foo.bar`:
- `dir = /workspace/projects/demo/foo/bar`
- `K = ["foo", "bar"]` (length 2)
- Suffix check: `dir`'s trailing 2 segments are `foo/bar`, which matches `K`.
- Suffix stripped: `WorkspaceRoot = /workspace/projects/demo`.

#### Error Handling: Package-Directory Mismatch
If a file at `/workspace/foo/bar/app.prism` declares `package foo.baz`:
- Expected directory suffix: `foo/baz`
- Actual directory: `/workspace/foo/bar`
- The resolver must abort with diagnostic:
  ```
  Error: Package path mismatch in '/workspace/foo/bar/app.prism'
    Declared: package foo.baz (expected directory suffix: 'foo/baz')
    Actual:   /workspace/foo/bar
  ```

---

### 2.2. Import Resolution Rules

Imports fall into exactly two mutually exclusive categories:

| Import Syntax | Classification | Root Anchor | Resolved Filesystem Target |
| :--- | :--- | :---: | :--- |
| `import doo` | **Sibling Import** | Current Package Dir | `<current_dir>/doo.prism` |
| `import utils/math` | **Subdir Relative** | Current Package Dir | `<current_dir>/utils/math.prism` |
| `import /core/fmt` | **Root-Anchored** | `<workspace_root>` | `<workspace_root>/core/fmt.prism` |
| `import /app/ui/views` | **Root-Anchored** | `<workspace_root>` | `<workspace_root>/app/ui/views.prism` |

#### Formal Resolution Rules
Given:
- Source file `P` located in directory `dir = parent(P)`.
- Discovered workspace root `root = WorkspaceRoot(P)`.
- Parsed `ImportDecl { is_root, path }`.

1. **Root-Anchored Import (`is_root == true`)**:
   - Begins with leading slash: `import /core/fmt`.
   - Path components: `["core", "fmt"]`.
   - Primary candidate: `T_1 = root/core/fmt.prism`.
   - Secondary candidate (Directory package default): `T_2 = root/core/fmt/package.prism`.
   - If `T_1` exists as a file, resolve to `T_1`.
   - Else if `T_2` exists as a file, resolve to `T_2`.
   - Else emit `Error: ImportNotFound`.

2. **Sibling Import (`is_root == false`)**:
   - Does NOT begin with leading slash: `import doo`.
   - Path components: `["doo"]`.
   - Primary candidate: `T_1 = dir/doo.prism`.
   - Secondary candidate: `T_2 = dir/doo/package.prism`.
   - If `T_1` exists as a file, resolve to `T_1`.
   - Else if `T_2` exists as a file, resolve to `T_2`.
   - Else emit `Error: ImportNotFound`.

---

### 2.3. Package Boundary Scanning

In Prism, a **package directory** forms the collective declaration boundary:
- All `.prism` files located directly within the same directory share `package` visibility.
- Private symbols (`private` / default) remain strictly file-scoped.
- Subdirectories are distinct packages and are **never** implicitly included into parent packages.

#### Algorithm: `scan_package_directory(package_dir: &Path) -> Vec<PathBuf>`
1. Read all directory entries in `package_dir`.
2. Filter entries:
   - Exclude subdirectories.
   - Exclude hidden files (starting with `.`, e.g. `.DS_Store`).
   - Include files with exact canonical extension `.prism`.
3. Sort file paths deterministically (lexicographically) to guarantee hermetic builds.
4. Verify every discovered file in the directory declares the identical `package` statement matching this directory.

---

### 2.4. Dependency Graph & Cycle Detection

#### Compilation Unit Order
Prism decouples declarations (Phase 1 ABI) from implementations (Phase 2 Body Typechecking):
- Phase 1 allows circular package references for declarations if mutually recursive types use `Heap<T>` or ID handles.
- At the file level, Phase 2 compilation avoidance requires a **Directed Acyclic Graph (DAG)** of file ABI dependencies.

#### Graph Construction:
1. Entry point: List of input `.prism` files.
2. For each file `P`:
   - Discover workspace root `root`.
   - Extract imports.
   - Resolve each import to target path `T`.
   - Add directed edge: `P -> T`.
3. Cycle Detection:
   - Perform Tarjan's or Kosaraju's Strongly Connected Components (SCC) algorithm.
   - If an import cycle is detected in Phase 2 dependencies, produce a diagnostic trace:
     `Cycle detected: a.prism -> b.prism -> c.prism -> a.prism`.

---

## 3. Data Structures & API Design for `prism-resolver`

### 3.1. Crate Architecture
A dedicated crate `crates/prism-resolver` will be added to the Rust workspace:
```
crates/prism-resolver/
├── Cargo.toml
├── src/
│   ├── lib.rs          # Public facade
│   ├── root.rs         # Pure path root discovery
│   ├── import.rs       # Sibling and root-anchored import resolver
│   ├── package.rs      # Package directory scanner
│   ├── graph.rs        # Multi-file dependency graph & cycle detector
│   └── error.rs        # Structured resolution errors & diagnostics
└── tests/
    └── resolver_tests.rs
```

### 3.2. Logical Model & Entities

The resolver operates on the following conceptual entities:

- **Resolved Target**: Represents the resolved filesystem entity of an import, which is either a concrete `.prism` source file or a package directory.
- **Source Unit**: Represents a parsed `.prism` source file, its discovered workspace root, its associated package name, and its direct outgoing import dependencies.
- **Dependency Graph**: A directed graph where nodes are source units and edges represent import dependencies, providing topological compilation ordering and cycle detection.

### 3.3. Engine Responsibilities

The resolver engine provides four core operations:

1. **Root Discovery**: Given an entry file path and its optional package declaration, discovers the workspace root strictly by path-suffix comparison with zero marker files and zero configuration files.
2. **Import Resolution**: Given a source file path, an import declaration, and the workspace root, resolves the import into a concrete filesystem path on disk.
3. **Package Scanning**: Given a package directory, scans all direct `.prism` files belonging to the package and validates that all files share identical package declarations.
4. **Dependency Graph Construction**: Given a set of entry paths, traverses import statements transitively to construct the complete dependency DAG, detecting circular references and producing the topological compilation order.

---

## 4. Diagnostics & Error Catalog

| Error Code | Error Variant | User-Facing Message | Resolution Suggestion |
| :--- | :--- | :--- | :--- |
| `E1001` | `PackagePathMismatch` | `Package path 'foo.bar' does not match directory '/workspace/foo/baz'` | Rename directory to match package or update package statement. |
| `E1002` | `FileNotFound` | `Cannot resolve import '/core/fmt': file '<root>/core/fmt.prism' not found` | Check spelling or verify target file exists at root. |
| `E1003` | `CircularDependency` | `Circular import dependency detected: [a.prism -> b.prism -> a.prism]` | Refactor shared declarations into a sibling module. |
| `E1004` | `PackageHeterogeneity` | `File 'b.prism' in package directory 'foo/bar' declares conflicting package 'foo.other'` | Ensure all files in a folder share the same package. |

---

## 5. Verification Fixtures & Acceptance Criteria

A goldfish implementing `prism-resolver` must verify against the following test fixtures:

1. **Standard Nested Package**:
   - `/demo/app/ui/views.prism` with `package app.ui` ==> Root is `/demo`.
2. **Root File**:
   - `/demo/main.prism` with no `package` statement ==> Root is `/demo`.
3. **Leading Slash Root Import**:
   - From `/demo/app/ui/views.prism`: `import /core/fmt` ==> `/demo/core/fmt.prism`.
4. **Sibling Import**:
   - From `/demo/app/ui/views.prism`: `import widgets` ==> `/demo/app/ui/widgets.prism`.
5. **Directory Mismatch Rejection**:
   - `/demo/app/ui/views.prism` with `package app.models` ==> Returns `PackagePathMismatch`.
6. **Cycle Detection**:
   - `a.prism` imports `b`, `b.prism` imports `a` ==> Returns `CircularDependency`.
7. **Package Boundary Invariant**:
   - `scan_package("/demo/app/ui")` finds `views.prism` and `widgets.prism`, but ignores `/demo/app/ui/components/sub.prism`.
