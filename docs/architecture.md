# Prism Planned Architecture

This document holds the **planned** design of Prism subsystems that are **not implemented yet**. It is the counterpart of [`openspec/specs/`](../openspec/specs), which describes only behavior the compiler implements today.

- **Nothing in this document is implemented.** Treat it as design intent, not as a description of the code.
- Requirements keep the OpenSpec `Requirement` / `WHEN` / `THEN` form so they can be copied into a change's delta specs.
- When an OpenSpec change implements a requirement, its delta spec adds it to `openspec/specs/` and the change removes it from this document.
- Progress is tracked in [`ROADMAP.md`](../ROADMAP.md). The non-negotiable invariants these designs must respect live in [`openspec/specs/system-invariants`](../openspec/specs/system-invariants/spec.md).

---

## 1. Compilation Pipeline & Compilation Avoidance

Implemented today (see [`compilation-and-abi`](../openspec/specs/compilation-and-abi/spec.md)): Phase 1 ABI extraction, a deterministic 128-bit ABI hash, recording a hash of each generic function's body, and the first three stages of the Phase 2 semantic pipeline: the typed tree ([`typed-tree`](../openspec/specs/typed-tree/spec.md)), MIR ([`mir`](../openspec/specs/mir/spec.md)), and borrow checking each file's MIR alone for uses of moved and uninitialized values, followed by drop elaboration (`borrowck-moves`, [`compilation-and-abi`](../openspec/specs/compilation-and-abi/spec.md) and [`ownership`](../openspec/specs/ownership/spec.md)), and then for borrow conflicts and escaping closures on the elaborated MIR (`borrowck-exclusivity`, [`references`](../openspec/specs/references/spec.md)). Generic declarations are checked once, against their bounds, into polymorphic typed trees and MIR, with their signatures and marker parameters in the ABI ([`generics`](../openspec/specs/generics/spec.md)); instantiating them is planned for `monomorphization`. The callee's side of returned borrows is planned in section 3. Everything below is planned.

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

### Open decision: where generic instances live (`monomorphization`)
`generics` checks each generic declaration once into elaborated polymorphic MIR, but ships no generic body to importers (D15 of [`openspec/changes/archive/2026-10-18-generics/design.md`](../openspec/changes/archive/2026-10-18-generics/design.md)). The recommended direction: the exporter ships each generic function's elaborated polymorphic MIR, which is self-describing because calls record callee paths, declared parameter types and type arguments. Instances live in separate instance artifacts keyed by the generic's path, the type arguments and the generic's MIR hash, rather than in the importer's artifact. That keeps a file's artifact a function of its content and its direct imports' ABI hashes even when an instance calls generics of modules the importer does not import, and with import cycles, and regenerates only the instances whose generic body changed. It may make the Phase 1 body hash in the ABI hash unnecessary. Still to decide: whether exported generic bodies may reference private declarations (Rust ships what they reach; Swift's `@inlinable` allows only exported ones), how polymorphic recursion is rejected, and the MIR rewrites for `T?` instantiated with a nullable `T`, which flattens.

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

Implemented today: rejecting borrowed fields in structs ([`syntax`](../openspec/specs/syntax/spec.md)); basic borrow type compatibility, including rejecting `&mut` of a `val` and requiring `&mut` on an argument passed to a `&mut T` parameter (`E4004`, [`type-system`](../openspec/specs/type-system/spec.md)); and, from `value-semantics`, the `: Copy` and `: Clone` markers, the built-in `Copy` and `Clone` tables, explicit `.clone()`, whole-value moves (`E4017`) and immutable top-level declarations ([`ownership`](../openspec/specs/ownership/spec.md)); and, from `reference-access`, implicit shared borrows at argument sites, copy-out of `Copy` values from borrows, moving out of a borrow (`E4017`), prefix `*` for writes, re-pointing `var` references, reborrowing `&mut` arguments, the `&(T?)` to `(&T)?` view and invariant references ([`references`](../openspec/specs/references/spec.md)); and, from `extension-methods`, methods as extension functions whose receivers are borrowed shared, borrowed mutably or moved by their `fun` / `mut fun` / `consuming fun` mode with nothing written at the call site ([`extension-functions`](../openspec/specs/extension-functions/spec.md), [`references`](../openspec/specs/references/spec.md), [`ownership`](../openspec/specs/ownership/spec.md)); and, from `borrowck-moves`, single-owner values checked by the borrow checker: use after move (`E5001`) through branches, loops, consuming receivers and closure captures, reinitialization by assignment, uninitialized reads (`E5002`), and drops elaborated so that each value is dropped exactly once where its owner's scope ends ([`ownership`](../openspec/specs/ownership/spec.md), [`mir`](../openspec/specs/mir/spec.md)); and, from `borrowck-exclusivity`, stack-scoped borrows checked by the borrow checker: any number of shared borrows or one mutable borrow, each lasting until the last use of the references holding it, with conflicting borrows and reads (`E5003`), assignments and moves of borrowed values (`E5004`, `E5005`) and values that end while borrowed (`E5006`) rejected, disjoint field borrows, two-phase receiver borrows, call results that keep their input borrows (the caller's half of the elision rules), closures that capture by inferred borrow and never move a capture (`E4017`), and capturing closures and function-typed parameters that cannot escape (`E5007`) ([`references`](../openspec/specs/references/spec.md), [`ownership`](../openspec/specs/ownership/spec.md), [`mir`](../openspec/specs/mir/spec.md)); and, from `borrowck-elision`, the four lifetime elision rules from signatures alone, for callers and callees, with returned borrows of locals, temporaries and by-value parameters (`E5008` `DanglingBorrowReturn`) and returned borrows derived from untied inputs (`E5009`) rejected, function types eliding by the same rules, defaults borrowing only through shared references, and borrows of top-level `val`s valid everywhere ([`references`](../openspec/specs/references/spec.md)). Everything below is planned.

The "road to the borrow checker" in [`ROADMAP.md`](../ROADMAP.md), agreed with the user on 2026-10-17, is complete; escaping, move-capturing closures are left to `closures`. Flow-insensitive rules are type errors (`E4xxx`, in the type checker); flow-sensitive ones are borrow-checker errors (`E5xxx`, see section 1 for the pipeline).

### Requirement: Recursive Heap Indirection via Heap<T>
Self-referential or recursive structures MUST use the standard `Heap<T>` primitive to provide single-owner heap allocation. Constructing a `Heap<T>` and moving its contents out are not designed yet (see `ROADMAP.md`).

#### Scenario: Dropping Heap<T> allocation
- **WHEN** an instance of `Heap<T>` leaves scope
- **THEN** the allocated heap buffer and the inner value `T` are freed deterministically with zero reference-counter traffic

### Requirement: Data-Oriented Typed Handles for Graphs
Graph topologies and cyclic data structures MUST use typed integer/handle IDs instead of raw pointers or reference-counting pointer cycles.

#### Scenario: Peer node reference via typed ID
- **WHEN** a struct references another struct via a typed identifier (`val parentId: NodeId?`)
- **THEN** no cyclic pointer graphs are formed and memory is managed by the owning collection

### Requirement: Reactive Tree Copy State Handles
Slot-table state handles (`State<T>`, `NodeId`) used in UI components MUST be declared `: Copy` and store generational indices rather than raw heap pointers.

#### Scenario: Capturing State<T> handle in event closure
- **WHEN** an escaping UI event closure captures a `State<T>` handle (`Button(onClick = { count.update { it + 1 } })`)
- **THEN** the handle is copied by value as a 64-bit ID, avoiding heap-allocation lifetime issues (needs the escaping-closures follow-up)

### Design notes (2026-10-17 exploration)
Why the requirements above were chosen; each change's `design.md` repeats the decisions it implements.

- **Closures are conservative until escaping closures are designed.** Inferred-borrow captures plus "capturing closures cannot escape" are sound with no new syntax and are implemented (`references`); move-capturing closures, which compose's `onClick` needs, come in `closures`.
- **`&String?` parses as `&(String?)`.** Postfix binds tighter than prefix, as in expressions (`&x.y` borrows `x.y`), and `&mut String?` is the natural type for a mutable nullable slot. An optional borrow is written `(&String)?`.

---

## 4. Runtime Semantics Awaiting Code Generation

The parser and type checker already accept these constructs (see [`syntax`](../openspec/specs/syntax/spec.md) and [`type-system`](../openspec/specs/type-system/spec.md)). Their runtime behavior needs a backend. The MIR ([`mir`](../openspec/specs/mir/spec.md)) already fixes the order of evaluation and where each temporary is dropped (argument and receiver temporaries at the end of their statement, a `val r = &make()` temporary with its binding, a borrowed default's temporary in the caller's statement), and its elaborated form which drops run, some under a drop flag; the scenarios below are what the backend must make observable at run time.

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

### Requirement: Loop and Range Evaluation Semantics
A `while` loop MUST evaluate its condition before each iteration and a `do`/`while` loop after each iteration. A `for` loop MUST evaluate its range once and bind the loop variable to each `Int` in it in ascending order. `break` MUST leave the innermost enclosing loop and `continue` MUST go to its next condition check or element. `a..b` MUST include `b`, and `a..<b` MUST exclude it.

#### Scenario: Half-open range iteration
- **WHEN** `for (i in 0..<3)` runs, and `for (i in 3..<3)` runs
- **THEN** the first loop's body runs with `i` equal to 0, 1 and 2, and the second loop's body never runs

#### Scenario: Closed range iteration
- **WHEN** `for (i in 1..3)` runs
- **THEN** its body runs with `i` equal to 1, 2 and 3

#### Scenario: Do-while body runs once
- **WHEN** `do { n = n + 1 } while (false)` runs with `n` equal to 0
- **THEN** `n` is 1

#### Scenario: Continue skips to the next element
- **WHEN** `for (i in 0..<5) { if (i == 2) continue; sum = sum + i }` runs with `sum` equal to 0
- **THEN** `sum` is 8

#### Scenario: Break leaves only the innermost loop
- **WHEN** a `break` runs inside a `while` loop nested in a `for` loop
- **THEN** the `while` loop ends and the `for` loop continues with its next element

#### Scenario: Range bounds evaluated once
- **WHEN** `for (i in 0..<next())` runs, where `next()` returns 3
- **THEN** `next()` is called once, before the first iteration

### Requirement: Clone Evaluation Semantics
`x.clone()` MUST produce a value that owns no storage shared with `x`, and MUST run no user code. A `Copy` value is copied bitwise. A `String` clone allocates a new buffer with the same contents. A `Heap<T>` clone allocates a new box holding a clone of its contents. A `T?` clone is `null` when `x` is, and otherwise a clone of its contents. A user struct or enum clones each field or payload in declaration order.

#### Scenario: String clone is independent
- **WHEN** `var a = "hi"` is cloned into `b`, and `a` is then reassigned
- **THEN** `b` still holds `"hi"`, and dropping `b` frees only `b`'s buffer

#### Scenario: Struct clone clones each field
- **WHEN** `struct User : Clone { val name: String, val tags: Heap<Int> }` is cloned
- **THEN** the clone holds a new `String` buffer and a new `Heap` box with equal contents

#### Scenario: Copy value clone is a copy
- **WHEN** `p.clone()` runs for `p: Point` where `struct Point : Copy { val x: Int }`
- **THEN** the result is a bitwise copy of `p`, with no allocation

### Requirement: Reference Evaluation Semantics
The type checker decides every implicit operation on a reference ([`references`](../openspec/specs/references/spec.md), from `reference-access`). At runtime, a borrow MUST point at the borrowed place itself, never at a copy of it. A temporary created for a borrow MUST be dropped at the end of the statement that created it. A copy-out MUST be a bitwise copy that runs no user code.

#### Scenario: Implicitly borrowed place is not copied
- **WHEN** `greet(u.name)` runs for `fun greet(n: &String)`
- **THEN** `greet` receives a pointer to `u.name`'s storage, and nothing is copied or allocated

#### Scenario: Argument temporary dropped at the end of the statement
- **WHEN** `greet(make())` runs for `fun greet(n: &String)` and `fun make(): String`
- **THEN** the `String` returned by `make()` lives until `greet` returns and the statement ends, and is then dropped

#### Scenario: Viewing a borrowed nullable
- **WHEN** `&u.nick`, of type `&(String?)`, is read as a `(&String)?`
- **THEN** the tag of `u.nick` is read once, giving `null` when it is `null` and otherwise a pointer to its `String` payload, without copying or allocating

#### Scenario: Writing through a reference drops the old referent
- **WHEN** `*r = v` runs for `r: &mut String`
- **THEN** the `String` that `r` referred to is dropped and `v` is moved into its place, so the owner of that place sees `v`

### Requirement: Scope Exit Cleanup
At runtime, each drop that the elaborated MIR runs MUST execute the value's drop glue at that point, deterministically and without a garbage collector or reference counting: dropping a struct or an enum drops its owned fields or payload, and dropping a `String` or a `Heap<T>` frees its allocation. A drop guarded by a drop flag MUST run exactly when the flag is set. Which drops run is decided by the borrow checker's drop elaboration ([`mir`](../openspec/specs/mir/spec.md)); what a drop does is `runtime-strings-and-drops`'.

#### Scenario: Deterministic scope exit cleanup
- **WHEN** an owning variable that still holds a value leaves its enclosing block scope
- **THEN** its destructor runs immediately and deterministically, dropping owned fields and heap allocations without garbage collector intervention

#### Scenario: A conditionally moved value is dropped only if present
- **WHEN** `if (c) take(s)` runs for an owned `s: String` and the scope of `s` ends
- **THEN** the `String` is freed once: by `take` when `c` is true, and at the end of the scope otherwise

### Requirement: Method Call Evaluation Semantics
The type checker decides how each extension call passes its receiver (the `extension-functions` spec, from `extension-methods`). At runtime, a call `recv.name(args)` MUST evaluate `recv` first: a receiver the call takes by value (the receiver of a `consuming fun`, or a `Copy` receiver copied out of a reference) is moved or copied then, before the arguments, and a receiver the call borrows has its place evaluated then and is borrowed after the arguments. The arguments MUST be evaluated left to right, and then the call made. A temporary receiver MUST be dropped at the end of the statement. A `?.` call MUST evaluate its receiver once, and MUST evaluate no argument and give `null` when it is `null`.

#### Scenario: Receiver evaluated before the arguments
- **WHEN** `make().greet(next())` runs for `fun make(): User`, `fun next(): Int` and `fun User.greet(n: Int)`
- **THEN** `make()` is called before `next()`, and the `User` it returned lives until the statement ends, when it is dropped

#### Scenario: A mutable receiver is borrowed after the arguments
- **WHEN** `v.push(v.size())` runs for `mut fun Stack.push(n: Int)` and `fun Stack.size(): Int`
- **THEN** `v.size()` runs before `v` is borrowed mutably for `push`, which is what lets two-phase borrows accept it

#### Scenario: A consuming receiver is moved before the arguments
- **WHEN** `s.into(next())` runs for `consuming fun Stack.into(n: Int): Int` and `fun next(): Int`
- **THEN** `s` is moved before `next()` is called

#### Scenario: A safe call on null evaluates no argument
- **WHEN** `u?.greet(next())` runs with `u` equal to `null`
- **THEN** `next()` is not called and the expression is `null`

---

## 5. Compose UI Engine

Implemented today: parsing `compose fun` ([`syntax`](../openspec/specs/syntax/spec.md)) and rejecting composable calls outside composable contexts (`E4008`, [`type-system`](../openspec/specs/type-system/spec.md)). The slot table, `remember` / `mutableStateOf`, the `compose fun` transform and hot-reload rendering are planned; see Phase 4 in [`ROADMAP.md`](../ROADMAP.md). The state-handle requirement is in section 3 above.
