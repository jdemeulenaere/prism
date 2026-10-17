# Diagnostics Specification Delta

## ADDED Requirements

### Requirement: Diagnostics Show Related Locations
When a diagnostic has a label for its primary location or related locations, such as where a used value was moved, the CLI MUST render the primary location's snippet with its label, then each related location in source order with its own location line, snippet and label, then the diagnostic's notes and help lines.

#### Scenario: Rendered related location
- **WHEN** `prism check` reports an `E5001` for `fun take(s: String) {}` and `fun f(s: String) { take(s); take(s) }`
- **THEN** after the header, the location and snippet of the second `s` labelled "value used here after move", it prints the location and snippet of the first `s` labelled "value moved here", then a `note` line and a `help` line

## MODIFIED Requirements

### Requirement: Stable Diagnostic Codes
Every error the compiler reports MUST carry a code from the catalog below. A code MUST keep its meaning once assigned, and a retired code MUST NOT be reused.

#### Scenario: Rendered diagnostic header
- **WHEN** `prism check` reports an error
- **THEN** the diagnostic starts with `error[<code>]: <title>`, followed by the source location when one is known

#### Scenario: Syntax error location and lexer message
- **WHEN** `prism check` reports an `E2xxx` error, including one in an imported file
- **THEN** the diagnostic points at the error's own line and column, and a lexer error shows the lexer's message (`Unknown escape sequence '\q'`) rather than the parser's view of an error token

#### Scenario: Code range identifies the stage
- **WHEN** a diagnostic is reported
- **THEN** its code falls in the range of the stage that produced it: `E1xxx` resolution and I/O, `E2xxx` lexing and parsing, `E3xxx` Phase 1 ABI extraction, `E4xxx` Phase 2 type checking, `E5xxx` Phase 2 borrow checking

#### Scenario: New error kind gets a new code
- **WHEN** a change introduces a new kind of error
- **THEN** it takes the next unused code in its stage's range, adds it to this catalog, and does not reuse a retired code
