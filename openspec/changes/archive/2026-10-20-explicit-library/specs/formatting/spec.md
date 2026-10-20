## ADDED Requirements

### Requirement: Imports Are Sorted
The formatter MUST print a file's imports sorted by path, a root-anchored path (`/a/b`) comparing as its text with the leading `/`, so root-anchored imports come before sibling ones, and imports of the same path keeping their order. When a comment lies between the first and the last import, the formatter MUST keep the imports in their written order.

#### Scenario: Imports sorted by path
- **WHEN** formatting `package app\nimport widgets\nimport /prism/core\nimport /app/model\nimport /prism/collections\nfun main() {}`
- **THEN** the result is `package app\n\nimport /app/model\nimport /prism/collections\nimport /prism/core\nimport widgets\n\nfun main() {}\n`

#### Scenario: Imports with comments among them are kept in order
- **WHEN** formatting `import widgets\n// The core.\nimport /prism/core\nfun main() {}`
- **THEN** the result is `import widgets\n// The core.\nimport /prism/core\n\nfun main() {}\n`

## MODIFIED Requirements

### Requirement: Formatting Preserves Meaning
The formatted source MUST parse to the same syntax tree as the input, ignoring source positions and the order of imports ("Imports Are Sorted"), and MUST contain the same comments, with the same text and in the same order. The formatter MUST verify both on every run by parsing its own output; on any difference it MUST leave the file untouched, report an internal error (a formatter bug, which carries no catalog code), and exit with a non-zero status.

#### Scenario: Every fixture and example keeps its meaning
- **WHEN** formatting every `.prism` file under `crates/prism-cli/tests/fixtures/` and `examples/` that parses
- **THEN** each output parses to the input's syntax tree, ignoring positions and the order of imports, and holds the input's comments in order

#### Scenario: Line breaks are only added where they keep the parse
- **WHEN** formatting a statement whose expression is too long for one line, inside a block where a line break ends an expression
- **THEN** the formatter breaks only inside parentheses or brackets, after a binary operator, `=` or `->`, or before `.`, `?.` or `?:`, so the statement parses as before
