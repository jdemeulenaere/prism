# Spec Delta

## ADDED Requirements

### Requirement: CLI Workspace Analysis and Dependency-Ordered Validation
The compiler CLI `prism check` command MUST accept either an individual source file or a directory path, executing workspace root discovery, import resolution, cycle detection, and Phase 1 ABI verification in topological dependency order.

#### Scenario: Single entry file with transitive dependency resolution
- **WHEN** user executes `prism check <file.prism>` on an entry source file that imports sibling or root modules
- **THEN** the CLI discovers the workspace root, parses the entry file, recursively loads all transitively imported modules under import-directed isolation, validates the dependency graph, and verifies ABI extraction for all reachable files in topological order

#### Scenario: Directory check with package homogeneity
- **WHEN** user executes `prism check <dir>` targeting a package directory
- **THEN** the CLI scans the directory, asserts that all `.prism` files in the directory share an identical package declaration, resolves internal and external dependencies, and validates all contained files

#### Scenario: Circular dependency diagnostic error reporting
- **WHEN** user executes `prism check` on source files containing mutually recursive or cyclical imports
- **THEN** the CLI detects the cycle, formats a user-friendly diagnostic listing the cyclic import path or SCC cluster, and exits with a non-zero exit code

#### Scenario: Workspace root multi-package analysis
- **WHEN** user executes `prism check <workspace_root>` on a workspace directory containing multiple package subdirectories
- **THEN** the CLI recursively scans all packages, builds the global dependency graph without requiring marker files, and validates all source units in topological order
