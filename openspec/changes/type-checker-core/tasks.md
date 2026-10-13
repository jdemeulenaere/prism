# Tasks

## 1. Scaffolding & Core Type Representation

- [ ] 1.1 Scaffold `crates/prism-typechecker` in workspace `Cargo.toml`, implement complete `Type` definitions (`Int`, `Float`, `Bool`, `Char`, `String`, `Unit`, `Nothing`, `Reference { mutable, inner }`, `Nullable`, `Heap`, `Struct`, `Enum`, `Fn { is_compose, .. }`, `TypeParam`, `Error`), subtyping lattice rules (`Nothing <= T`, `T <= T?`, `&mut T <= &T`), and verify with unit tests
- [ ] 1.2 Implement lexical scoping and symbol environments (`Scope`, `Symbol`, `Mutability`) supporting shadowing, local bindings, duplicate declarations (`E4010`), and unknown identifier errors (`E4001`), and verify with unit tests
- [ ] 1.3 Implement Pass 1 file-level symbol registration supporting forward references of top-level functions, structs, and enums, and inferred return types for private expression-body functions, and verify with unit tests

## 2. Bidirectional Type Checking & Control Flow

- [ ] 2.1 Implement bidirectional expression inference and checking for literals (`Char`, `Int`, `Float`, `String`, `Bool`), strict numeric operations (no implicit widening), relational/logical operators, indexing (`target[index]`), immutability (`E4003`), `Type::Error` cascade suppression, and block statement `Unit` typing, and verify with unit tests
- [ ] 2.2 Implement function invocation and closure type checking (arity, argument types, default parameters, `E4004`), closure checking against expected `Type::Fn`, scoped ambient `compose fun` context validation across closures (`E4008`), and `return` compatibility (`E4005`), and verify with unit tests
- [ ] 2.3 Implement `when` expression type checking: subject pattern matching, subjectless boolean branches, branch unification with `Nothing` bottom type, and exhaustiveness verification (`E4009`), and verify with unit tests

## 3. Nominal Types, References & Cross-Module ABI Integration

- [ ] 3.1 Implement struct instantiation, field access validation (`E4006`), field mutability validation on `val` instances (`E4003`), and enum variant constructor typing, and verify with unit tests
- [ ] 3.2 Implement reference & borrow typing (`&T`, `&mut T`), nullability (`T?`, safe nav `?.`, Elvis `?:`), and `Heap<T>` indirection typing, and verify with unit tests
- [ ] 3.3 Ingest Phase 1 `FileAbi` imports into `TypeChecker` using structured type accessors in `crates/prism-abi` without redundant string re-parsing, enforce package visibility boundaries (`E4007`), and verify with multi-file unit tests

## 4. CLI Workspace Integration & End-to-End Verification

- [ ] 4.1 Decouple Phase 1 ABI extraction and Phase 2 body checking in `crates/prism-cli/src/workspace.rs`, integrating `prism-typechecker` across topological SCC orders, and verify with integration tests in `cli_workspace_tests.rs`
- [ ] 4.2 Render formatted type error diagnostics (`E4001` through `E4010`) with source code snippets and carets in `crates/prism-cli/src/diagnostics.rs`, and verify with CLI integration tests
- [ ] 4.3 Run full workspace `cargo test` across all crates to verify clean execution and zero regressions
