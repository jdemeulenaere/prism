## MODIFIED Requirements

### Requirement: Import Resolution and Workspace Anchoring
The resolver MUST resolve imports into concrete, verified `.prism` files on disk, strictly distinguishing between sibling and root-anchored imports. Import paths MUST consist of identifier segments only; the parser rejects `.` and `..` segments before resolution.

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
- **WHEN** an import path contains `.` or `..` path segments (`import ../x`, `import ./x`, `import /a/../x`)
- **THEN** parsing MUST fail with a syntax error (`E2001`) whose message says import paths cannot contain `.` or `..` and suggests a root-anchored import

#### Scenario: Invalid import path rejected by resolution
- **WHEN** a file imports itself, or a sibling import has more than one segment (`import doo.util`)
- **THEN** resolution MUST reject the import with `InvalidImportPath` (`E1006`)

#### Scenario: Import target escaping workspace boundary
- **WHEN** an import resolves to a path outside the discovered workspace root (such as through a symlink)
- **THEN** resolution MUST reject the target with `WorkspaceBoundaryViolation` (`E1005`)
