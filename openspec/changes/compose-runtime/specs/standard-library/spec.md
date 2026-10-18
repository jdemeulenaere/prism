# Standard Library Delta

## MODIFIED Requirements

### Requirement: The Compiler Ships the Library
The `prism` executable MUST contain the source of every library module and MUST NOT read library files from disk. The modules MUST be `/prism/core` and `/prism/collections`, the prelude modules, and `/prism/system` and `/prism/composer` (compose, "The Composer Protocol Module"), which are imported explicitly. Each MUST be a file of the package `prism`, or of a package below it, whose module path is `prism/<name>`.

#### Scenario: A library module is imported
- **WHEN** `app/main.prism` declares `import /prism/system` and `fun main() { println(1); exitProcess(3) }`, in a workspace with no `prism` directory
- **THEN** `prism run app/main.prism` prints `1` and exits with status 3

#### Scenario: The composer module is not in the prelude
- **WHEN** `prism check --explain-cache main.prism` runs on a file without imports, and again after the file declares `import /prism/composer`
- **THEN** the first explanation names no `prism/composer.prism`, and the second names it
