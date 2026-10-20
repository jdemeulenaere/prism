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

#### Scenario: Relative entry path resolved before root discovery
- **WHEN** a file at `/workspace/app/ui/main.prism` declares `package app.ui` and is checked by a relative path (`main.prism`, `./main.prism` or `../ui/main.prism`) from a current directory other than the workspace root, or its directory is checked as `.`
- **THEN** the path is first made absolute against the current directory, so the workspace root resolves to `/workspace` exactly as for the absolute path, and diagnostics show file paths relative to the current directory when they lie inside it

### Requirement: Import Resolution and Workspace Anchoring
The resolver MUST resolve imports into concrete, verified `.prism` files on disk, strictly distinguishing between sibling and root-anchored imports. A root-anchored import of two or more segments whose first segment is `prism` MUST instead resolve to the bundled library module of that path, whatever the workspace root holds (see "The Reserved Root Package"). Import paths MUST consist of identifier segments only; the parser rejects `.` and `..` segments before resolution.

#### Scenario: Root-anchored import with leading slash
- **WHEN** a source file declares `import /core/fmt`
- **THEN** the target resolves to `<workspace_root>/core/fmt.prism`

#### Scenario: Sibling import without leading slash
- **WHEN** a file in `<workspace_root>/app/ui/` declares `import widgets`
- **THEN** the target resolves to `<workspace_root>/app/ui/widgets.prism`

#### Scenario: Import of a library module
- **WHEN** a source file declares `import /prism/system`
- **THEN** the target is the bundled module `prism/system`, and an import of a module the library does not have (`import /prism/nothing`) fails with `FileNotFound` (`E1002`), naming the bundled library

#### Scenario: Target file does not exist
- **WHEN** an import targets a non-existent file on disk
- **THEN** resolution MUST fail with diagnostic `FileNotFound` (`E1002`)

#### Scenario: Import attempting directory traversal
- **WHEN** an import path contains `.` or `..` path segments (`import ../x`, `import ./x`, `import /a/../x`)
- **THEN** parsing MUST fail with a syntax error (`E2001`) whose message says import paths cannot contain `.` or `..` and suggests a root-anchored import

#### Scenario: Invalid import path rejected by resolution
- **WHEN** a file imports itself, or a sibling import has more than one segment (`import doo.util`)
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

### Requirement: The Reserved Root Package
The root package `prism` MUST name the bundled library (`standard-library`), never the workspace. A workspace file whose `package` declaration's first segment is `prism` (`package prism`, `package prism.ui`) MUST be rejected with `ReservedPackage` (`E1008`), whose help suggests renaming the package.

#### Scenario: A workspace package named prism rejected
- **WHEN** `/workspace/prism/util.prism` declares `package prism`, or `/workspace/prism/ui/view.prism` declares `package prism.ui`, and either is checked
- **THEN** resolution fails with `ReservedPackage` (`E1008`)

#### Scenario: A workspace directory named prism is not imported
- **WHEN** `/workspace/prism/system.prism` exists and `/workspace/app/main.prism` declares `import /prism/system`
- **THEN** the import resolves to the bundled module `prism/system`, and the workspace file is not read

#### Scenario: A root file named prism is an ordinary module
- **WHEN** `/workspace/prism.prism` declares no package and `/workspace/main.prism` declares `import /prism`
- **THEN** the import resolves to `/workspace/prism.prism`, whose module path is `prism`

#### Scenario: A copy of the library's sources is not the library
- **WHEN** the compiler repository's `library/` directory is copied to `/copy/library/` and `prism check /copy/library/prism/core.prism` runs
- **THEN** resolution fails with `ReservedPackage` (`E1008`): the sources are compiled only as the bundled library, embedded in the executable

### Requirement: Library Files Resolve Only Library Imports
A library file MUST resolve its imports only to library modules: a sibling import to a module of its own package, a root-anchored import to a module under `/prism`. Any other import MUST be rejected with `FileNotFound` (`E1002`). Library files MUST NOT take part in workspace root discovery, the root consistency check (`E1004`) or boundary checks (`E1005`).

#### Scenario: A library file imports a workspace module
- **WHEN** a library test module declares `import /app/util`
- **THEN** resolution fails with `FileNotFound` (`E1002`), saying that library files import only library modules

### Requirement: Library Files Are Shown by Module Path
Diagnostics, `--explain-cache` and runtime error locations MUST show a library file by its module path followed by `.prism` (`prism/collections.prism`), which no workspace file can be shown by.

#### Scenario: A diagnostic in a library file
- **WHEN** a library test module `prism/test` has a type error on line 3
- **THEN** the diagnostic's location reads `prism/test.prism:3:<column>`

## Design Rationale
Decisions from `cli-workspace-driver` and `fix-cli-workspace-audit`:

- **No manifest file.** A `Prism.toml` was rejected because the workspace root must come from the entry file's path and `package` declaration alone (zero-marker invariant).
- **File targets follow imports; directory targets scan.** `prism check <file>` loads only what the file transitively imports, so unrelated or half-written siblings never block it. `prism check <dir>` scans recursively and enforces package homogeneity per directory.
- **Import cycles are allowed.** Files are processed in topological order of SCC clusters. Because exported declarations carry explicit types, Phase 1 can extract the ABI of every file in a cycle without reading any body, so a cycle is not an error (the old `E1007` was retired).
- **Every input is boundary-checked.** Entry files and scanned files, not just imports, are canonicalized and must stay under the canonical workspace root, so a symlink cannot pull in files from outside (`E1005`).
- **Case is verified against the disk.** On case-insensitive filesystems (APFS, NTFS) a plain existence check ignores case, so the resolver reads directory entries to compare each segment exactly. A project then behaves the same on every OS.
- **An empty relative root means `.`.** A workspace root of `""` is normalized to `.` before existence checks so canonicalization and boundary checks are never skipped.
- **`prism` is a reserved root package, resolved to the bundled library** (D2 of `standard-library`). A root-anchored import of two or more segments starting with `prism` names a library module whatever the workspace holds, and a workspace `package prism...` is `E1008`, so the same import means the same code in every workspace and no workspace file can be shown by a library module's path (`prism/collections.prism`). Library nodes are keyed by those virtual paths, never read from disk, root-discovered or boundary-checked, and resolve their imports only within the library. The zero-marker invariant holds: nothing on disk marks the library. `import /prism`, a single segment, stays a workspace import, so a root file `prism.prism` keeps working. Kotlin's `kotlin.*`, Java's `java.*` and Rust's `std` are reserved the same way. Rejected: a separate import syntax for the library (`import prism:ui`), letting a workspace `prism/` directory shadow it, and reserving every path starting with `prism`. Since `explicit-library` the sources are ordinary files in the repository's `library/`, still embedded in the executable, and the reservation is what keeps them from being replaceable: `prism check library/prism/core.prism` is `E1008`, since a copy of the sources is not the library (`explicit-library` D6); checking `library/` in place is a follow-up for library authors.
