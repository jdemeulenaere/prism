# Prism Diagnostic Codes

## Purpose
The diagnostics specification is the single catalog of compiler error codes. Each code identifies one kind of error across all stages (resolution, parsing, ABI extraction and type checking), so tools and tests can key on codes instead of message text. The other specs reference these codes in their scenarios; this spec lists them all in one place.

## Requirements

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
- **THEN** its code falls in the range of the stage that produced it: `E1xxx` resolution and I/O, `E2xxx` lexing and parsing, `E3xxx` Phase 1 ABI extraction, `E4xxx` Phase 2 type checking

#### Scenario: New error kind gets a new code
- **WHEN** a change introduces a new kind of error
- **THEN** it takes the next unused code in its stage's range, adds it to this catalog, and does not reuse a retired code

## Catalog

### E1xxx: Resolution and I/O (`crates/prism-resolver`, `crates/prism-cli`)

| Code | Name | Meaning |
|---|---|---|
| `E1000` | `Io` | A file or directory could not be read, or a `prism check` target does not exist. |
| `E1001` | `PackagePathMismatch` | The declared `package` does not match the file's directory path, including a case-only mismatch or more package segments than directories. |
| `E1002` | `FileNotFound` | An import targets a file that does not exist with the exact casing on disk. |
| `E1003` | `PackageHeterogeneity` | Files directly inside one directory declare different packages. |
| `E1004` | `InconsistentWorkspaceRoot` | A dependency resolves to a different workspace root than the entry file. |
| `E1005` | `WorkspaceBoundaryViolation` | An import, entry file or scanned file resolves outside the workspace root, for example through a symlink. |
| `E1006` | `InvalidImportPath` | A file imports itself, or a sibling import has more than one segment. `.` and `..` segments are rejected earlier by the parser (`E2001`). |
| `E1007` | (retired) | Cyclic dependency. Retired by `fix-cli-workspace-audit`, which allowed mutually recursive imports. Do not reuse. |

### E2xxx: Lexing and parsing (`crates/prism-lexer`, `crates/prism-parser`)

| Code | Name | Meaning |
|---|---|---|
| `E2001` | syntax error | The lexer produced an error token or the parser rejected the source. The parser stops at the first error. |
| `E2002` | `NestingTooDeep` | Expressions or types nest deeper than 256 levels, counting each operator of a left-associative chain such as `a + a + ... + a` as one level. |

### E3xxx: Phase 1 ABI extraction (`crates/prism-abi`)

| Code | Name | Meaning |
|---|---|---|
| `E3001` | `MissingExplicitReturnType`, `MissingExplicitVarType` | An exported declaration lacks a type required for ABI extraction: an expression-bodied `public`/`package` function without a return type, or an exported top-level variable without a type. |

### E4xxx: Phase 2 type checking (`crates/prism-typechecker`)

| Code | Name | Meaning |
|---|---|---|
| `E4001` | `UnresolvedSymbol` | An identifier is not found in any enclosing scope or imported ABI. |
| `E4002` | `TypeMismatch`, `TypeNameAsValue` | An expression's type is not assignable to the type expected by an annotation, assignment, operator operand or default value (return and argument sites have their own codes), including implicit numeric widening and `null` for a non-nullable type; or a struct or enum name is used as a value. |
| `E4003` | `ImmutabilityViolation` | Reassigning a `val`, assigning a `val` field, mutating a field of a `val` instance, or taking `&mut` of a `val`. |
| `E4004` | `CallSignatureMismatch`, `ArgumentTypeMismatch` | Wrong argument count, an argument whose type is not assignable to its parameter (positional, named or trailing-lambda argument, struct constructor field, enum payload), a missing, duplicate or twice-assigned named argument, or a missing or duplicate struct field in a constructor call. |
| `E4005` | `ReturnTypeMismatch` | A returned value (a `return` operand, an expression body, the final expression of a non-`Unit` block body, or a closure body) is incompatible with the declared or expected return type, including a value returned from a `Unit` function and a `return` outside a function. |
| `E4006` | `UnknownField` | A field does not exist on the struct, or member access on a non-struct or opaque type. |
| `E4007` | `VisibilityViolation` | A `package`-visible symbol is accessed from another package. |
| `E4008` | `InvalidComposeContext` | A `compose fun` is called outside a composable function or closure. |
| `E4009` | `NonExhaustiveWhen` | A `when` in value position covers neither every enum variant nor has an `else`. |
| `E4010` | `DuplicateSymbol` | Two declarations in the same scope share a name, including a top-level declaration with the same name as an imported symbol. |
| `E4011` | `MissingReturnType` | A `public` or `package` function omits its return type. |
| `E4012` | `AmbiguousImport` | A name the file uses (as a value, callee or type) is exported by more than one of its imports. Importing both without using the name is accepted. |
| `E4013` | `CannotInferType` | A type cannot be inferred and must be written: a closure parameter with no expected function type, or an unannotated private declaration whose inferred type depends on itself (such as a recursive function without a return type). |
| `E4014` | `MemberOfUnimportedType` | A member of a struct or enum is used (a field read or written, its variants matched, the type constructed or a variant named) but the module defining the type is not a direct import; the message names the import to add. Receiving and passing along such a value is accepted. |

A `public` or `package` function without a return type is reported as `E3001` when it has an expression body (Phase 1 rejects it first) and as `E4011` when it has a block body (Phase 1 accepts it as returning `Unit`). Unifying these is tracked in `ROADMAP.md`.

## Design Rationale
- **One code per error kind, separate from message text.** Tooling and tests match on codes, so messages can improve without breaking them.
- **`E4011` instead of reusing `E4004`** (`resolve-nominal-type-annotations`): a missing return type is not a call error, and the type checker also runs without the ABI stage (unit tests, a future LSP), so it needs its own diagnostic rather than relying on `E3001`.
- **`E1007` retired, not reused** (`fix-cli-workspace-audit`): mutually recursive imports are valid because exported declarations carry explicit types, so Phase 1 can extract every file in a cycle without reading any body.
- **Site-specific mismatch codes** (`typechecker-soundness`): `E4002`, `E4004` and `E4005` all report a value whose type is not assignable, distinguished by where the expected type came from (an annotation or operand, a parameter, a return type), so tools can suggest the right fix.
- **`E4012`, `E4013` and `E4014` are new kinds, not reuses** (`typechecker-soundness`): an ambiguous import is not an unresolved symbol (`E4001`), a type that cannot be inferred is not a mismatch, and a member of a type whose module is not imported is not an unknown field (`E4006`), so each needs its own code and its own help line.
