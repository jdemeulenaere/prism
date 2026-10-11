# Package Resolution & Root Discovery

## Purpose
Deterministic, zero-marker workspace root discovery and import resolution based on filesystem paths matching package declarations.

## Requirements

### Requirement: Pure Path Root Discovery
The compiler must discover the workspace root by comparing the source file's filesystem path against its package declaration segments.

#### Scenario: Matching package declaration
- **WHEN** a file at `/workspace/foo/bar/main.prism` declares `package foo.bar`
- **THEN** the workspace root resolves to `/workspace`

#### Scenario: Empty package declaration
- **WHEN** a file at `/workspace/main.prism` declares an empty package
- **THEN** the workspace root resolves to `/workspace`

#### Scenario: Mismatched package declaration
- **WHEN** a file at `/workspace/foo/baz/main.prism` declares `package foo.bar`
- **THEN** package resolution aborts with an error diagnostic indicating `PackagePathMismatch`
