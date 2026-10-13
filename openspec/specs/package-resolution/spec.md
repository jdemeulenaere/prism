# Package Resolution & Zero-Marker Root Discovery

## Purpose
The package resolution subsystem discovers the workspace root without configuration or marker files and resolves file-level imports into a deterministic dependency graph.

## Requirements

### Requirement: Pure Path Workspace Root Discovery
The resolver MUST deterministically discover the workspace root by comparing the entry source file's filesystem path against its declared package statement.

#### Scenario: Matching nested package declaration
- **WHEN** a file located at `/workspace/app/ui/views.prism` declares `package app.ui`
- **THEN** the workspace root resolves to `/workspace`

#### Scenario: Root-level file with empty package
- **WHEN** a file located at `/workspace/main.prism` declares an empty package or omits the `package` declaration
- **THEN** the workspace root resolves to `/workspace`

#### Scenario: Package declaration exceeds directory depth
- **WHEN** a file located at `/app/main.prism` declares `package foo.bar.baz`
- **THEN** resolution MUST fail with diagnostic `PackagePathMismatch` (`E1001`)

#### Scenario: Directory path does not match declared package
- **WHEN** a file located at `/workspace/app/ui/views.prism` declares `package app.models`
- **THEN** resolution MUST fail with diagnostic `PackagePathMismatch` (`E1001`)

#### Scenario: Strict case-sensitivity enforcement
- **WHEN** a file is located in directory `/workspace/App/UI` on a case-insensitive filesystem but declares `package app.ui`
- **THEN** resolution MUST reject the path and emit `PackagePathMismatch` (`E1001`)

### Requirement: Import Resolution and Workspace Anchoring
The resolver MUST resolve imports into concrete, verified `.prism` files on disk, strictly distinguishing between sibling and root-anchored imports.

#### Scenario: Root-anchored import with leading slash
- **WHEN** a source file declares `import /core/fmt`
- **THEN** the target resolves to `<workspace_root>/core/fmt.prism`

#### Scenario: Sibling import without leading slash
- **WHEN** a file in `<workspace_root>/app/ui/` declares `import widgets`
- **THEN** the target resolves to `<workspace_root>/app/ui/widgets.prism`

#### Scenario: Target file does not exist
- **WHEN** an import targets a non-existent file on disk
- **THEN** resolution MUST fail with diagnostic `FileNotFound` (`E1002`)

#### Scenario: Import attempting directory traversal
- **WHEN** an import path contains `.` or `..` path segments
- **THEN** resolution MUST reject the import with `InvalidImportPath` (`E1006`)

#### Scenario: Import target escaping workspace boundary
- **WHEN** an import resolves to a path outside the discovered workspace root (such as through a symlink)
- **THEN** resolution MUST reject the target with `WorkspaceBoundaryViolation` (`E1005`)

### Requirement: Import-Directed Compilation Isolation
The compiler MUST load and compile only files that are transitively reachable via `import` statements from the entry file.

#### Scenario: Unimported sibling files ignored
- **WHEN** an entry file imports `widgets.prism` in the same directory where `draft.prism` also exists without being imported
- **THEN** `draft.prism` is not inspected, parsed, or compiled

#### Scenario: Package visibility across packages rejected
- **WHEN** a file in package `app.ui` imports a file in package `app.models` and accesses a declaration marked with `package` visibility
- **THEN** the compiler MUST emit a visibility access error

### Requirement: Dependency Graph and SCC Cycle Decomposition
The resolver MUST build an import dependency graph and decompose cycles into Strongly Connected Components (SCCs).

#### Scenario: Mutually recursive sibling imports
- **WHEN** file `a.prism` imports `b` and file `b.prism` imports `a`
- **THEN** both files are grouped into a single SCC cluster of size 2, and declaration extraction succeeds in Phase 1

### Requirement: CLI Directory Scanner Homogeneity
When running batch analysis on a folder (`prism check <dir>`), the scanner MUST verify that all direct `.prism` files declare the identical package.

#### Scenario: Homogeneous folder
- **WHEN** all `.prism` files directly inside `/workspace/app/ui/` declare `package app.ui`
- **THEN** the directory scan succeeds

#### Scenario: Conflicting package declaration in same folder
- **WHEN** a file inside `/workspace/app/ui/` declares `package app.other`
- **THEN** the scanner MUST reject the batch check with `PackageHeterogeneity` (`E1003`)

### Requirement: CLI Workspace Analysis and Dependency-Ordered Validation
The compiler CLI `prism check` command MUST accept either an individual source file or a directory path, executing workspace root discovery, import resolution, SCC cycle grouping, and Phase 1 ABI verification in topological dependency order.

#### Scenario: Single entry file with transitive dependency resolution
- **WHEN** user executes `prism check <file.prism>` on an entry source file that imports sibling or root modules
- **THEN** the CLI discovers the workspace root, parses the entry file, recursively loads all transitively imported modules under import-directed isolation, validates the dependency graph, and verifies ABI extraction for all reachable files in topological order

#### Scenario: Directory check with package homogeneity
- **WHEN** user executes `prism check <dir>` targeting a package directory
- **THEN** the CLI scans the directory, asserts that all `.prism` files in the directory share an identical package declaration, resolves internal and external dependencies, and validates all contained files

#### Scenario: Circular dependency diagnostic error reporting
- **WHEN** user executes `prism check` on source files containing mutually recursive or cyclical imports
- **THEN** the CLI groups the files into an SCC cluster and extracts Phase 1 ABIs for all cluster members in topological order without error

#### Scenario: Workspace root multi-package analysis
- **WHEN** user executes `prism check <workspace_root>` on a workspace directory containing multiple package subdirectories
- **THEN** the CLI recursively scans all packages, builds the global dependency graph without requiring marker files, and validates all source units in topological order

#### Scenario: Symlink escaping workspace boundary rejected
- **WHEN** an entry file or directory entry points to a symlink resolving outside the discovered workspace root
- **THEN** the CLI rejects the target with `WorkspaceBoundaryViolation` (`E1005`)
