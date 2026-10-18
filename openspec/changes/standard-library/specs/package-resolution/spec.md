# Package Resolution Delta

## ADDED Requirements

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

## MODIFIED Requirements

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
