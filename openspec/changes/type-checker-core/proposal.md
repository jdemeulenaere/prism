# Proposal

## Why

Prism's Phase 1 front-end extracts declaration interfaces, handles import resolution, and computes ABI hashes, but function bodies and variable declarations currently remain unvalidated semantically. To transition into Phase 2 of the roadmap and guarantee type safety before code generation (Cranelift JIT/LLVM), Prism requires a core static type checker (`crates/prism-typechecker`) that performs symbol table resolution, bidirectional type checking, statement/expression verification, and compiler invariant enforcement.

## What Changes

- Create a new crate `crates/prism-typechecker` in the compiler workspace.
- Implement comprehensive `Type` representation accommodating:
  - Primitive types: `Int`, `Float`, `Bool`, `Char`, `String`, `Unit`, and bottom type `Nothing` (matching Kotlin's bottom type semantics for expressions that diverge or return).
  - Stack borrows and references: `&T` and `&mut T` with call-site mutability verification and `&mut T <= &T` coercion (Invariant 4).
  - Nullable types: `T?` with null-coalescing (`?:`) and safe navigation (`?.`) expressions (Invariant 1).
  - Recursive indirection: `Heap<T>` single-owner value indirection.
  - Nominal types: user-defined `struct` and `enum` definitions/variants.
  - Composable functions: tracking `is_compose` on function types and closures (Invariant 5).
  - Generic parameters: `TypeParam(String)`.
  - Sentinel: `Error` for downstream error cascade suppression.
- Implement lexical scoping and symbol environments for local variables, function parameters, file declarations (with two-pass forward reference support and expression-body private function return type inference), and imported ABI symbols.
- Implement bidirectional type checking and inference for:
  - Strict numeric typing (no implicit type widening or coercions).
  - Variable and struct field bindings (`val` immutability vs `var` mutability on base instances and fields).
  - Arithmetic, relational, logical, equality, indexing, and range expressions.
  - Safe navigation (`?.`), Elvis operator (`?:`), and closures / trailing lambdas.
  - Block evaluation and statement `Unit` typing rules.
  - Control flow structures: `if`/`else` branch unification, exhaustive `when` expressions, and `return` expressions matching function return types.
  - Function invocations: arity check, argument type compatibility, default parameter handling, and `compose fun` ambient calling context validation across closure boundaries.
  - Cross-package visibility: enforcing that `package`-visibility declarations cannot be accessed outside their defining package (Invariant 1).
- Introduce structured semantic diagnostic codes (`E4001` through `E4010`) with source spans and descriptive error messages:
  - `E4001`: Unresolved symbol / unknown identifier
  - `E4002`: Type mismatch / assignment incompatibility
  - `E4003`: Immutability violation (reassigning `val` or mutating a field on a `val` instance)
  - `E4004`: Call signature mismatch (arity / parameter types / invoking non-callable)
  - `E4005`: Return type mismatch
  - `E4006`: Unknown struct field access (or field access on non-struct)
  - `E4007`: Visibility access violation (cross-package access to `package` declarations)
  - `E4008`: Invalid compose call context (calling `compose fun` from non-composable function or closure)
  - `E4009`: Non-exhaustive `when` expression pattern match
  - `E4010`: Duplicate symbol declaration in lexical scope
- Integrate Phase 2 body type checking into the CLI driver (`prism check`) with decoupled two-phase execution: all Phase 1 ABIs are extracted across the dependency graph before Phase 2 body checking begins in topological SCC order.

## Capabilities

### New Capabilities
- `type-system`: Semantic analysis, lexical symbol resolution, nominal type definitions, bidirectional type inference, and static type checking for Prism modules and function bodies.

### Modified Capabilities
<!-- No existing spec requirements need modification; this introduces the type-system capability -->

## Impact

- **New Crates**: `crates/prism-typechecker` added to `Cargo.toml` workspace members.
- **ABI Crate**: `crates/prism-abi` adapted to provide structured type conversion / accessors for exported signatures without redundant string re-parsing.
- **Compiler CLI**: `crates/prism-cli` invokes `prism-typechecker` in Phase 2 during `prism check`.
- **Diagnostics**: `prism-cli` renders formatted Phase 2 type errors (`E4001`-`E4010`) with source code snippets and carets.
- **Downstream**: Establishes the typed AST / semantic foundation necessary for Phase 2 borrow checking and Phase 3 Cranelift JIT compilation.
