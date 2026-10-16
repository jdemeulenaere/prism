# Prism Planned Architecture

This document holds the **planned** design of Prism subsystems that are **not implemented yet**. It is the counterpart of [`openspec/specs/`](../openspec/specs), which describes only behavior the compiler implements today.

- **Nothing in this document is implemented.** Treat it as design intent, not as a description of the code.
- Requirements keep the OpenSpec `Requirement` / `WHEN` / `THEN` form so they can be copied into a change's delta specs.
- When an OpenSpec change implements a requirement, its delta spec adds it to `openspec/specs/` and the change removes it from this document.
- Progress is tracked in [`ROADMAP.md`](../ROADMAP.md). The non-negotiable invariants these designs must respect live in [`openspec/specs/system-invariants`](../openspec/specs/system-invariants/spec.md).

---

## 1. Compilation Pipeline & Compilation Avoidance

Implemented today (see [`compilation-and-abi`](../openspec/specs/compilation-and-abi/spec.md)): Phase 1 ABI extraction, a deterministic 128-bit ABI hash, and recording a hash of each generic function's body. Everything below is planned.

### Requirement: Single File as Compilation Unit
Every `.prism` source file MUST be compiled into native machine code independently, with interface boundaries defined strictly by exported declarations.

#### Scenario: Compiling independent source files
- **WHEN** compiling multiple source files in a package
- **THEN** each file is lowered and code-generated as a standalone compilation unit

### Requirement: Parallel Phase 2 Body Compilation
After Phase 1 completes, function bodies MUST be lowered and compiled in parallel against the imported ABI interface hashes.

#### Scenario: Phase 2 body lowering
- **WHEN** Phase 1 completes
- **THEN** function bodies are typechecked and compiled in parallel against the imported ABI interface hashes

### Requirement: Compilation Avoidance via ABI Hashes
The compiler MUST cache compilation artifacts and recompile a file only when its own content or the ABI hash of a direct import changes.

#### Scenario: Modifying non-generic function body
- **WHEN** a developer modifies the internal implementation of a public function without changing its signature
- **THEN** zero downstream files that import it are recompiled

#### Scenario: Modifying public function signature
- **WHEN** a developer changes the parameter types or return type of a public function
- **THEN** direct downstream importers are recompiled

#### Scenario: Modifying private helper function
- **WHEN** a private helper function is added, removed, or modified in a source file
- **THEN** downstream files remain cached

### Requirement: Re-monomorphization on Generic Body Change
Downstream call sites that instantiate an exported generic function MUST be re-monomorphized when its Monomorphization Hash changes, and only those call sites.

#### Scenario: Changing generic function body
- **WHEN** the body of an exported generic function changes
- **THEN** re-monomorphization is triggered only in downstream call sites that instantiate that generic

### Open decision: ABI hash algorithm
The original design called for a BLAKE3 cryptographic hash of exported declarations. `crates/prism-abi` currently uses a deterministic 128-bit FNV-1a hash. Decide which algorithm is authoritative before building the `.abi` cache.

---

## 2. Two-Tier Native Execution

### Requirement: Two-Tier Native Execution Engine
The compiler MUST support two distinct native execution backends: Tier 1 (Cranelift JIT) for development and Tier 2 (LLVM AOT) for release.

#### Scenario: Tier 1 instant script execution
- **WHEN** the user runs `prism run <file.prism>`
- **THEN** the compiler compiles the file into memory using Cranelift JIT in under 15ms without invoking an external toolchain

#### Scenario: Tier 1 hot reload with disabled cross-file inlining
- **WHEN** code is running in development mode under Tier 1
- **THEN** cross-file inlining across compilation units is disabled, allowing function-pointer tables to be dynamically patched during hot reload

#### Scenario: Tier 2 release optimization
- **WHEN** the user builds with `prism build --release`
- **THEN** the compiler invokes the LLVM backend with whole-program optimization and Link-Time Optimization (LTO)

---

## 3. Memory Model & Resource Management

Compile-time memory safety: zero runtime garbage collection, zero hidden automatic reference counting (ARC), single-owner value semantics, stack-confined borrows with automatic lifetime elision, and data-oriented handles.

Implemented today: rejecting borrowed fields in structs ([`syntax`](../openspec/specs/syntax/spec.md)), and basic borrow type compatibility, including rejecting `&mut` of a `val` ([`type-system`](../openspec/specs/type-system/spec.md)). Everything below is planned.

### Requirement: Single-Owner Value Semantics and RAII
Every non-primitive resource MUST have a single owning binding, and resources MUST be dropped deterministically when their owner leaves lexical scope.

#### Scenario: Value move on assignment
- **WHEN** an owned non-primitive value is passed to a function by value or assigned to a new variable
- **THEN** ownership moves to the target, and subsequent access through the original binding is rejected at compile time

#### Scenario: Deterministic scope exit cleanup
- **WHEN** an owning variable leaves its enclosing block scope
- **THEN** its destructor is executed immediately and deterministically, dropping owned fields and heap allocations without garbage collector intervention

### Requirement: Recursive Heap Indirection via Heap<T>
Self-referential or recursive structures MUST use the standard `Heap<T>` primitive to provide single-owner heap allocation.

#### Scenario: Dropping Heap<T> allocation
- **WHEN** an instance of `Heap<T>` leaves scope
- **THEN** the allocated heap buffer and the inner value `T` are freed deterministically with zero reference-counter traffic

### Requirement: Stack-Scoped Borrowing and Mutation Rules
The type checker MUST enforce borrowing rules: arbitrary concurrent shared borrows (`&T`), or exactly one exclusive mutable borrow (`&mut T`), strictly confined to function arguments and local execution scopes.

#### Scenario: Concurrent shared borrows
- **WHEN** multiple functions inspect the same resource concurrently via `&T`
- **THEN** the compiler allows the shared reads

#### Scenario: Exclusive mutable borrow prevents concurrent access
- **WHEN** an exclusive borrow `&mut T` is active on a value
- **THEN** any other borrow or access through the owner is rejected at compile time

#### Scenario: Explicit mutation at call sites
- **WHEN** calling a function that accepts `&mut T`
- **THEN** the caller MUST pass the argument with explicit `&mut` syntax (`updateUser(&mut user)`)

### Requirement: The Four Lifetime Elision Rules
When a function returns a borrowed reference, its lifetime MUST be inferred deterministically according to the four lifetime elision rules without explicit lifetime annotations.

#### Scenario: Rule 1 - Single input borrow parameter
- **WHEN** a function signature has exactly one input borrow (`fun first(items: &Vector<String>): &String?`)
- **THEN** the returned borrow's lifetime is automatically tied to the input parameter's lifetime at the call site

#### Scenario: Rule 2 - Method receiver borrow
- **WHEN** a method has a receiver borrow `this: &T` and returns a reference
- **THEN** the returned reference's lifetime is automatically tied to the receiver's lifetime

#### Scenario: Rule 3 - Multiple input borrow parameters
- **WHEN** a function accepts multiple input borrows and returns a borrow
- **THEN** the returned reference's lifetime is tied to the intersection of all input borrow scopes

#### Scenario: Rule 4 - Zero input borrows returning a borrow
- **WHEN** a function has zero input borrow parameters and attempts to return a borrow to a local value
- **THEN** compilation MUST fail with diagnostic `DanglingBorrowReturn` (no error code assigned yet)

### Requirement: Data-Oriented Typed Handles for Graphs
Graph topologies and cyclic data structures MUST use typed integer/handle IDs instead of raw pointers or reference-counting pointer cycles.

#### Scenario: Peer node reference via typed ID
- **WHEN** a struct references another struct via a typed identifier (`val parentId: NodeId?`)
- **THEN** no cyclic pointer graphs are formed and memory is managed by the owning collection

### Requirement: Reactive Tree Copy State Handles
Slot-table state handles (`State<T>`, `NodeId`) used in UI components MUST implement `Copy` and store generational indices rather than raw heap pointers.

#### Scenario: Capturing State<T> handle in event closure
- **WHEN** an escaping UI event closure captures a `State<T>` handle (`Button(onClick = { count.update { it + 1 } })`)
- **THEN** the handle is copied by value as a 64-bit ID, avoiding heap-allocation lifetime issues

---

## 4. Runtime Semantics Awaiting Code Generation

The parser and type checker already accept these constructs (see [`syntax`](../openspec/specs/syntax/spec.md) and [`type-system`](../openspec/specs/type-system/spec.md)). Their runtime behavior needs a backend.

### Requirement: Inline Value Struct Layout
Struct instances MUST be stored inline by value without heap indirection, and recursive enums MUST have a finite layout through `Heap<T>`.

#### Scenario: Value struct definition
- **WHEN** a struct is declared with fields (`struct Point { val x: Double, val y: Double }`)
- **THEN** instances of `Point` are stored inline by value without heap indirection

#### Scenario: Recursive enum with Heap indirection
- **WHEN** an enum variant contains recursive instances wrapped in `Heap<T>` (`enum Expr { Literal(Int), Add(Heap<Expr>, Heap<Expr>) }`)
- **THEN** the enum layout is finite

### Requirement: Expression Evaluation Semantics
`when` branches MUST evaluate in order, and null-safe operators MUST short-circuit at runtime.

#### Scenario: When pattern matching
- **WHEN** a `when` expression matches over values or ranges (`when (x) { 0 -> "zero", in 1..10 -> "small", else -> "large" }`)
- **THEN** the branches are evaluated in order and the matching branch expression is returned

#### Scenario: Safe navigation and Elvis operator
- **WHEN** code evaluates `val len = name?.length ?: 0` and `name` is null
- **THEN** the expression evaluates to 0

---

## 5. Compose UI Engine

Implemented today: parsing `compose fun` ([`syntax`](../openspec/specs/syntax/spec.md)) and rejecting composable calls outside composable contexts (`E4008`, [`type-system`](../openspec/specs/type-system/spec.md)). The slot table, `remember` / `mutableStateOf`, the `compose fun` transform and hot-reload rendering are planned; see Phase 4 in [`ROADMAP.md`](../ROADMAP.md). The state-handle requirement is in section 3 above.
