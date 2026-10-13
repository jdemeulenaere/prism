# Design

## Context

Phase 1 provides lexing, parsing, zero-config package resolution, and surface ABI extraction (`FileAbi`). Invariant 2 requires all `public` and `package` declarations to have explicit parameter and return types, while Invariant 6 dictates that each file compiles as an independent unit where Phase 2 body checking depends strictly on imported ABIs. Currently, function bodies are unvalidated. See `proposal.md` for motivation.

## Goals / Non-Goals

**Goals:**
- Implement `crates/prism-typechecker` with clean separation between type representation, symbol environments, bidirectional type inference, and diagnostic generation.
- Support comprehensive type representations: primitives (`Int`, `Float`, `Bool`, `Char`, `String`, `Unit`, `Nothing`), stack references (`&T`, `&mut T`), nullability (`T?`), recursive heap indirection (`Heap<T>`), nominal types (`Struct`, `Enum`), generic parameters (`TypeParam`), and composable functions (`is_compose`).
- Support bidirectional typing: `infer(expr) -> Result<Type>` and `check(expr, expected: &Type) -> Result<()>`.
- Support lexical scoping for locals, parameters, and module declarations with immutability enforcement (`val` vs `var`), struct field mutability, and duplicate symbol detection (`E4010`).
- Enforce composable calling context constraints across functions and closures: `compose fun` can only be invoked within other composable functions or composable closures (`E4008`).
- Enforce package visibility boundaries across modules: `package`-visible declarations cannot be accessed outside their defining package (`E4007`).
- Cross-module type resolution driven strictly by Phase 1 `FileAbi` imports with structured types to avoid redundant parsing.
- Decouple Phase 1 ABI extraction and Phase 2 body verification in CLI `prism check` so mutual recursion cycles don't deadlock body checking.

**Non-Goals:**
- Full borrow-checker graph analysis (deferred to ownership/borrowck pass; this pass validates types and basic reference mutability).
- Generic parameter monomorphization (deferred to generic compilation pass).
- Trait/interface polymorphism or dynamic dispatch.

## Decisions

### 1. Standalone `crates/prism-typechecker` Crate
- **Decision**: Introduce a dedicated crate `prism-typechecker` that depends on `prism-ast` and `prism-abi`.
- **Alternative**: Abbreviated naming like `prism-typeck` or embedding type checking inside `prism-resolver`/`prism-parser`.
- **Rationale**: Keeps naming consistent with existing workspace crates (`prism-lexer`, `prism-parser`, `prism-resolver`). Decouples parser grammar and module graph resolution strictly from semantic analysis. `prism-typechecker` can be tested independently with in-memory AST fixtures.

### 2. Bidirectional Type Checking
- **Decision**: Use bidirectional typing (`infer` mode for unannotated literals/expressions, `check` mode against expected types from annotations or return types).
- **Alternative**: Global Hindley-Milner type inference with unification variables.
- **Rationale**: Prism mandates explicit types on all exported signatures and function parameters (Invariant 2). Local bidirectional typing is fast, highly deterministic, and generates pinpointed error diagnostics without complex unification solver traces.

### 3. Comprehensive Type Representation, Subtyping Lattice & `Nothing`
- **Decision**: Define the core `Type` enum:
  ```rust
  pub enum Type {
      Int,
      Float,
      Bool,
      Char,
      String,
      Unit,
      Nothing,
      Reference { mutable: bool, inner: Box<Type> },
      Nullable(Box<Type>),
      Heap(Box<Type>),
      Struct { name: String, fields: Vec<(String, bool, Type)> }, // (name, is_var, type)
      Enum { name: String, variants: Vec<(String, Vec<Type>)> },
      Fn { is_compose: bool, params: Vec<Type>, ret: Box<Type> },
      TypeParam(String),
      Error,
  }
  ```
- **Subtyping Lattice**:
  - `Nothing <= T` for all types `T` (diverging branch unifies with any expected type).
  - `T <= T?` for all types `T` (non-null value is assignable to nullable).
  - `&mut T <= &T` (mutable borrow coerces to shared borrow at call sites).
  - `Type::Error` unifies with all types (sentinel suppresses cascading secondary errors).
  - All other types are strictly invariant (e.g. `Int` does not coerce to `Float`).

### 4. Cross-Module Resolution via Structured `FileAbi` and Visibility Tracking
- **Decision**: Supply imported module interfaces to `TypeChecker` using `FileAbi` from Phase 1. In `FileAbi`, expose structured `prism_ast::types::Type` accessors for exported signatures so `TypeChecker` converts directly to `Type` without string re-parsing. Tag exported symbols with their declaring package path, allowing `TypeChecker` to enforce that `Visibility::Package` symbols are rejected when accessed from outside the defining package (`E4007`).
- **Alternative**: Passing full ASTs of all dependency modules.
- **Rationale**: Strictly preserves Invariant 6 (File as Compilation Unit). Phase 2 body typechecking of file `A` never inspects the implementation bodies of file `B`, only its exported signatures.

### 5. Ambient Composable Context Tracking Across Closures
- **Decision**: `TypeChecker` maintains a scoped stack of `in_compose: bool` contexts.
  - When entering a `compose fun`, `in_compose = true` is pushed.
  - When checking a closure `{ ... }`, the closure pushes `true` if its expected function type has `is_compose: true`, or `false` if it is a standard non-composable callback (e.g. `onClick: () -> Unit`).
  - When checking a call expression to a function with `is_compose: true`, if the current top of the stack is `false`, emit `E4008`.
- **Alternative**: Deferring compose checks to a UI runtime pass.
- **Rationale**: Invariant 5 defines `compose` as a first-class language keyword with statically enforced semantics. Rejecting non-composable calls early prevents runtime slot-table panics.

### 6. Decoupled Two-Phase Workspace Pipeline & Pass 1 Symbol Registration
- **Decision**:
  - **Within a file**: Pass 1 registers all top-level symbols (functions, structs, enums) into module scope. Private functions with expression bodies (`fun helper(x: Int) = x * 2`) register with an unresolved placeholder return type, which is inferred and back-propagated during Pass 2 body evaluation.
  - **Across files in `workspace.rs`**: Phase 1 extracts `FileAbi` across the entire workspace dependency graph before Phase 2 body checking begins in topological SCC order.
- **Alternative**: Interleaving ABI extraction and body checking in a single loop.
- **Rationale**: Mutually recursive sibling imports form valid SCC clusters in Phase 1. If body checking is interleaved, file `A` in a cycle cannot resolve against file `B` before `B`'s ABI is extracted. Decoupling completely eliminates mutual recursion deadlocks.

## Risks / Trade-offs

- **[Risk] Mutually recursive declarations inside a file**:
  - *Mitigation*: The type checker runs in two passes per file: Pass 1 collects all top-level symbols (functions, structs, enums) into the module scope before Pass 2 typechecks function bodies and expressions.
- **[Risk] Error cascade from invalid subexpressions**:
  - *Mitigation*: Expressions returning `Type::Error` unify with any expected type in `check()` to prevent downstream noise.
- **[Risk] Performance overhead during `prism check`**:
  - *Mitigation*: Symbol environments use scoped hash maps and cloned type descriptors without heavy allocations.
