## MODIFIED Requirements

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
