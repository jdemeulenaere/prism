# Prism Package Resolution & Zero-Marker Root Discovery Specification

**Status**: Living Architecture Specification (The Elephant)  
**Applies To**: Milestone 1, Phase 1 (`prism-resolver`, `prism-cli`, and runtime loaders)  
**Governing Invariants**: [Invariant 1 (`docs/INVARIANTS.md`)](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md#L7), [Invariant 2 (`docs/INVARIANTS.md`)](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md#L15), [Invariant 3 (`docs/INVARIANTS.md`)](file:///Users/jdemeulenaere/Development/lang/docs/INVARIANTS.md#L22), [ADR-0003 (`docs/adr/ADR-0003`)](file:///Users/jdemeulenaere/Development/lang/docs/adr/ADR-0003-module-resolution-and-abi-invalidation.md)

---

## 1. Overview & Architectural Principles

Prism adopts a strict **convention-over-configuration** filesystem contract designed to eliminate build configuration files (`Cargo.toml`, `build.gradle.kts`, `CMakeLists.txt`) and **eliminate marker files** (`.git`, `.prism-root`, `package.json`).

The filesystem hierarchy and source code declarations form a bidirectional, self-authenticating contract:
`Directory Hierarchy` <===> `Package Hierarchy`

Prism enforces a **strictly import-directed dependency model**:
- Every compilation dependency is explicit. Compiling an entry file (e.g. `main.prism` or `app.prism`) resolves only the files transitively imported by that file.
- Sibling files in the same directory do not implicitly share scope or pollute compilation. To access a sibling file's declarations, it must be explicitly imported (`import doo`).
- Unimported files in the same directory (e.g. unfinished drafts or test fixtures) are never inspected during single-file compilation, do not slow down incremental builds, and produce zero recompilations.
- Directory scanning is purely an optional CLI convenience when the user explicitly requests a batch check on an entire folder (`prism check <dir>`).

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
   - Normalize `P` to eliminate redundant separators, resolve `.` components, and standardize path separators across POSIX and Windows.
   - Obtain absolute directory path `dir = parent(P)`.

2. **Strict Case-Sensitivity Enforcement**:
   - On all operating systems (including case-insensitive filesystems such as macOS APFS and Windows NTFS), directory segment names on disk must match package segments with strict case sensitivity.
   - If the canonical on-disk directory casing differs from the declared package identifier casing, resolution aborts with `Error: PackagePathMismatch`.

3. **Empty Package (Root Files)**:
   - If `K` is empty (the file omits `package` or declares empty package):
     `WorkspaceRoot(P) = dir`
   - The file resides directly in the workspace root. It may use sibling relative imports (`import helper`) or root-anchored imports (`import /core/fmt`).

4. **Non-Empty Package**:
   - If `n > m` (the package path has more segments than the filesystem path):
     ==> Emits diagnostic `Error: PackagePathMismatch`.
   - Compare the trailing `n` directory segments of `dir` against `K`:
     Every segment `dir[m - n + i]` must exactly equal `k_i` (case-sensitive).
   - If all segments match:
     `WorkspaceRoot(P)` is obtained by stripping the trailing `n` segments from `dir`.
   - If any segment does not match:
     ==> Emits diagnostic `Error: PackagePathMismatch`.

5. **Root Consistency Across Multi-File Graphs**:
   - In any compilation graph, all resolved files must share the identical authoritative `WorkspaceRoot`.
   - If a transitively imported file resolves to a different workspace root, the resolver emits `Error: InconsistentWorkspaceRoot`.

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

### 2.2. Import Syntax & Resolution Rules

Imports fall into exactly two mutually exclusive categories:

| Import Syntax | Classification | Root Anchor | Resolved Filesystem Target |
| :--- | :--- | :---: | :--- |
| `import doo` | **Sibling Import** | Current Package Dir | `<current_dir>/doo.prism` |
| `import /core/fmt` | **Root-Anchored** | `<workspace_root>` | `<workspace_root>/core/fmt.prism` |
| `import /app/ui/views` | **Root-Anchored** | `<workspace_root>` | `<workspace_root>/app/ui/views.prism` |

#### Formal Resolution Rules
Given:
- Source file `P` located in directory `dir = parent(P)`.
- Discovered workspace root `root = WorkspaceRoot(P)`.
- Parsed `ImportDecl { is_root, path }`.

1. **Root-Anchored Import (`is_root == true`)**:
   - Begins with a leading slash: `import /core/fmt`.
   - Path components: `["core", "fmt"]`.
   - Target candidate: `T = root/core/fmt.prism`.
   - Candidate must exist as a regular file.
   - If `T` exists, verify case sensitivity against actual disk casing and resolve to `T`.
   - Else emit `Error: FileNotFound`.

2. **Sibling Import (`is_root == false`)**:
   - Does NOT begin with a leading slash: `import doo`.
   - Path components: exactly one identifier `["doo"]`.
   - Target candidate: `T = dir/doo.prism`.
   - Candidate must exist as a regular file.
   - If `T` exists, verify case sensitivity against actual disk casing and resolve to `T`.
   - Else emit `Error: FileNotFound`.

3. **No Directory Fallbacks or Marker Files**:
   - Prism has no `package.prism`, `mod.rs`, or `index.js`.
   - Every import maps strictly and directly to an explicit `.prism` file.

4. **Security & Workspace Jail**:
   - Import paths must never contain relative navigation tokens (`.` or `..`). Path traversal attempts trigger `Error: InvalidImportPath`.
   - Resolved targets must strictly reside within `root`. If a symbolic link points outside `root`, the resolver rejects it with `Error: WorkspaceBoundaryViolation`.

---

### 2.3. Import-Directed Resolution vs CLI Directory Scanning

Prism strictly decouples **file compilation dependencies** from **filesystem directory contents**.

#### 1. Import-Directed Compilation (Default)
When compiling an entry file `entry.prism`:
- The compiler parses `entry.prism`, extracts its imports, and resolves them recursively.
- Only files reachable via transitive `import` statements are loaded and compiled.
- Unimported sibling files in the same directory are completely ignored.
- **Package Visibility Access Rule**:
  When file `A` imports file `B` and accesses a declaration marked `package`:
  - Access is granted if and only if `A.package == B.package`.
  - If `A.package != B.package`, access is denied (`Error: PrivatePackageSymbol`).

#### 2. Optional CLI Directory Discovery (`prism check <dir>`)
When the user explicitly invokes batch analysis on a folder:
- The resolver inspects direct entries in `<dir>`.
- Filter criteria:
  - Exclude subdirectories (subdirectories are distinct packages).
  - Exclude hidden files (e.g. `.DS_Store`).
  - Include files ending with canonical extension `.prism`.
- **Package Homogeneity Validation**:
  Every discovered `.prism` file in `<dir>` must declare the exact same `package` statement matching `<dir>`.
  If any file declares a conflicting package, the scanner emits `Error: PackageHeterogeneity`.

---

### 2.4. Dependency Graph, Strongly Connected Components (SCCs), and Cycles

Prism decouples declarations (Phase 1 ABI) from implementations (Phase 2 Body Typechecking):

1. **Import Dependency Graph**:
   - Constructed as a directed graph where nodes are `.prism` source files and edges represent direct `import` references.
   - Transitively traversed starting from one or more entry files.

2. **Cyclic Dependencies & Strongly Connected Components (SCCs)**:
   - Sibling or cross-package files may import each other (e.g. `user.prism` imports `account.prism`, while `account.prism` imports `user.prism`).
   - Using Tarjan's or Kosaraju's algorithm, the dependency graph is partitioned into a DAG of **Strongly Connected Components (SCCs)**.
   - **Phase 1 (Declaration ABI Extraction)**:
     All files within an SCC are analyzed together. Their public and package declarations are extracted simultaneously. Mutual type references and mutual function calls across files are fully supported.
   - **Phase 2 (Body Compilation & Topological Order)**:
     The SCC DAG is compiled in topological order. Files outside the SCC depend only on the exported ABI hashes of the SCC.
   - **Illegal Value Recursion**:
     Circular import references are valid in Phase 1 as long as struct types do not contain infinite-size inline cycles without indirection (`Heap<T>` or ID handles). Infinite value-type layout cycles are caught during type layout verification (`Error: RecursiveTypeWithoutIndirection`).

---

## 3. Data Structures & Architectural Layout for `prism-resolver`

### 3.1. Crate Architecture
A dedicated crate `crates/prism-resolver` in the Rust workspace:
```
crates/prism-resolver/
├── Cargo.toml
├── src/
│   ├── lib.rs          # Public resolver facade
│   ├── root.rs         # Pure path workspace root discovery
│   ├── import.rs       # Sibling and root-anchored import resolver
│   ├── graph.rs        # Dependency graph, SCC decomposition, and topological sort
│   ├── scanner.rs      # Optional CLI directory scanner & homogeneity checker
│   └── error.rs        # Resolution diagnostics and error types
└── tests/
    └── resolver_tests.rs
```

### 3.2. Logical Model & Entities

The resolver operates on the following conceptual entities:

- **Workspace Root**: An absolute, canonical directory path discovered from an entry file path and its package declaration.
- **Import Target**: A concrete `.prism` file path on disk, verified to exist within the workspace root.
- **Source Node**: A compilation unit representing a `.prism` file, its discovered root, its declared package, and its direct outgoing imports.
- **SCC Cluster**: A group of one or more source nodes forming a strongly connected component in the import graph.
- **Dependency Graph**: A directed acyclic graph of SCC clusters providing deterministic compilation ordering.

### 3.3. Engine Responsibilities

The resolver engine provides four core operations:

1. **Root Discovery**: Given an entry file path and its optional package declaration, discovers the workspace root strictly by path-suffix comparison with zero marker files and zero configuration files.
2. **Import Resolution**: Given a source file path, an import declaration, and the workspace root, resolves the import into an absolute, verified `.prism` file path on disk.
3. **Dependency Graph Builder**: Given an entry file, recursively resolves imports, detects cycles, groups mutually recursive files into SCCs, and produces a topologically sorted schedule.
4. **CLI Directory Scanner**: Scans all direct `.prism` files in a folder for batch CLI verification, ensuring package homogeneity.

---

## 4. Diagnostics & Error Catalog

| Error Code | Error Variant | User-Facing Message | Resolution Suggestion |
| :--- | :--- | :--- | :--- |
| `E1001` | `PackagePathMismatch` | `Package path 'foo.bar' does not match directory '/workspace/foo/baz'` | Rename directory to match package or update package statement. |
| `E1002` | `FileNotFound` | `Cannot resolve import '/core/fmt': file '<root>/core/fmt.prism' not found` | Check spelling or verify target file exists at root. |
| `E1003` | `PackageHeterogeneity` | `File 'b.prism' in package directory 'foo/bar' declares conflicting package 'foo.other'` | Ensure all files in a folder share the same package. |
| `E1004` | `InconsistentWorkspaceRoot` | `File 'dep.prism' resolves to root '/a', conflicting with workspace root '/b'` | Ensure all project dependencies reside within the same workspace root. |
| `E1005` | `WorkspaceBoundaryViolation` | `Import resolves to path outside workspace root: '/etc/passwd'` | Remove symlink or relative path escaping the workspace. |
| `E1006` | `InvalidImportPath` | `Invalid import path '..': relative parent traversal is forbidden` | Use sibling import 'name' or root-anchored import '/path'. |

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
   - `/demo/app/ui/views.prism` with `package app.models` ==> Returns `PackagePathMismatch` (`E1001`).
6. **Import-Directed Isolation**:
   - Compiling `/demo/app/main.prism` which only imports `widgets.prism` ignores unimported `/demo/app/draft.prism`.
7. **Mutual Recursion in SCC**:
   - `a.prism` imports `b`, `b.prism` imports `a` ==> Grouped into single SCC cluster of size 2 without fatal graph cycle error.
8. **CLI Directory Scanning & Homogeneity**:
   - `scan_directory("/demo/app/ui")` finds `views.prism` and `widgets.prism`, but rejects if any file in that folder declares a different package (`E1003`).
9. **Workspace Jail Enforcement**:
   - Import target escaping root (e.g. symlink to `/tmp/outside.prism`) ==> Returns `WorkspaceBoundaryViolation` (`E1005`).
10. **Strict Case-Sensitivity Enforcement**:
    - Directory `Foo/Bar` with `package foo.bar` on macOS/Windows ==> Returns `PackagePathMismatch` (`E1001`).
