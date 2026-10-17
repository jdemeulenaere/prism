## MODIFIED Requirements

### Requirement: Lexical Scope and Symbol Resolution
The type checker MUST resolve identifiers across nested lexical scopes (local blocks, parameters, file declarations, and imported ABI symbols) and reject unresolved or duplicate symbols. Parameters form a scope level above the function body, so a local declared in the body MAY shadow a parameter.

#### Scenario: Local variable shadowing and resolution
- **WHEN** an expression references an identifier declared in an outer or enclosing block
- **THEN** the identifier resolves to the innermost matching declaration

#### Scenario: Local shadowing a parameter
- **WHEN** `fun f(a: Int): String { val a = "s"; return a }` is checked
- **THEN** the file typechecks successfully, and `a` after the local declaration refers to the `String` local

#### Scenario: Unknown identifier error reporting
- **WHEN** an expression references an undeclared identifier
- **THEN** the type checker emits an unresolved symbol error (`E4001`)

#### Scenario: Duplicate symbol declaration rejected
- **WHEN** a scope declares two variables or parameters with the identical identifier, including two closure parameters with the same name whether or not the closure has an expected type
- **THEN** the type checker rejects the duplicate declaration with `DuplicateSymbol` (`E4010`)

### Requirement: Ambiguous Imported Names Rejected at Use
When two imported modules export the same name and a file uses that name (as a value, callee or type), the type checker MUST reject the use with an ambiguity error (`E4012`) that names both modules. Importing both modules without using the name MUST be accepted. A top-level declaration that has the same name as an imported symbol MUST be accepted and MUST shadow the import: every use of the name in the file resolves to the local declaration, and no ambiguity is reported for it.

#### Scenario: Using a name exported by two imports
- **WHEN** `a/lib` exports `public fun helper(x: Int): Int`, `b/lib` exports `public fun helper(x: String): Int`, and a file importing both calls `helper("s")`
- **THEN** the type checker emits `E4012` naming `a/lib` and `b/lib`, and no other error for the call

#### Scenario: Unused colliding names accepted
- **WHEN** the same two modules are imported and `helper` is never referenced
- **THEN** the file typechecks successfully

#### Scenario: Ambiguous type name in an annotation
- **WHEN** both imported modules export `public struct User` and the file declares `fun f(u: User): Int`
- **THEN** the type checker emits `E4012` for the annotation

#### Scenario: Local declaration colliding with an import
- **WHEN** a file imports `a/lib`, which exports `public fun helper(x: Int): Int`, declares its own `fun helper(x: String): Int`, and calls `helper("s")`
- **THEN** the file typechecks successfully, and the call resolves to the local `helper`

#### Scenario: Local declaration resolving an ambiguity
- **WHEN** a file imports both `a/lib` and `b/lib`, which each export `helper`, declares its own `helper`, and uses it
- **THEN** the use resolves to the local declaration and no `E4012` is reported
