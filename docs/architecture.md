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

### Requirement: Semantic Pipeline After Type Checking
Phase 2 MUST lower each file through a typed, resolved tree and a control-flow graph before ownership checking and code generation, each produced from the previous stage and the direct imports' ABIs only.

#### Scenario: Typed tree from the type checker
- **WHEN** the type checker checks a file
- **THEN** it produces a typed tree (`prism-typed-tree` crate) in which every node carries its type and span, names are resolved to bindings or declaration identities, calls are normalized to positional order with explicit default slots, and every implicit copy, move, borrow, copy-out and reborrow is an explicit node; the tree is produced even when the file has type errors

#### Scenario: Lowering to MIR
- **WHEN** a file has no type errors
- **THEN** each function and closure body is lowered (`prism-mir` crate) to a place-based control-flow graph of basic blocks: statements assign rvalues to places (a local plus field and deref projections) and mark storage, terminators branch, call, return and drop, and `&&`, `||`, `?.`, `?:`, `when` and `for` become explicit branches; a stable text dump (`prism dump-mir <file.prism>`) drives snapshot tests

#### Scenario: Borrow checking on MIR
- **WHEN** a file's MIR has been built
- **THEN** the borrow checker (`prism-borrowck` crate) checks it using only the MIR and imported ABIs, reporting `E5xxx` diagnostics, and the same MIR later feeds the Cranelift backend

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

Implemented today: rejecting borrowed fields in structs ([`syntax`](../openspec/specs/syntax/spec.md)); basic borrow type compatibility, including rejecting `&mut` of a `val` and requiring `&mut` on an argument passed to a `&mut T` parameter (`E4004`, [`type-system`](../openspec/specs/type-system/spec.md)); and, from `value-semantics`, the `: Copy` and `: Clone` markers, the built-in `Copy` and `Clone` tables, explicit `.clone()`, whole-value moves (`E4017`) and immutable top-level declarations ([`ownership`](../openspec/specs/ownership/spec.md)); and, from `reference-access`, implicit shared borrows at argument sites, copy-out of `Copy` values from borrows, moving out of a borrow (`E4017`), prefix `*` for writes, re-pointing `var` references, reborrowing `&mut` arguments, the `&(T?)` to `(&T)?` view and invariant references ([`references`](../openspec/specs/references/spec.md)); and, from `extension-methods`, methods as extension functions whose receivers are borrowed shared, borrowed mutably or moved by their `fun` / `mut fun` / `consuming fun` mode with nothing written at the call site ([`extension-functions`](../openspec/specs/extension-functions/spec.md), [`references`](../openspec/specs/references/spec.md), [`ownership`](../openspec/specs/ownership/spec.md)). Everything below is planned.

The requirements below were decided with the user on 2026-10-17 and are delivered by the rest of the "road to the borrow checker" sequence in [`ROADMAP.md`](../ROADMAP.md): `borrowck-moves`, `borrowck-exclusivity` and `borrowck-elision`. Flow-insensitive rules are type errors (`E4xxx`, in the type checker); flow-sensitive ones are borrow-checker errors in a new `E5xxx` range (see section 1 for the pipeline).

### Requirement: Single-Owner Value Semantics and RAII
Every non-`Copy` value MUST have a single owning binding, and resources MUST be dropped deterministically when their owner leaves lexical scope.

#### Scenario: Value move on assignment
- **WHEN** an owned non-`Copy` value is passed to a function by value, returned, or assigned to a new variable
- **THEN** ownership moves to the target, and subsequent access through the original binding is rejected at compile time (use after move, `E5xxx`)

#### Scenario: Conditional and loop moves
- **WHEN** a value is moved in only one branch of an `if` or `when`, or inside a loop body without being reassigned before the next iteration
- **THEN** a later use (or the next iteration's use) is rejected as a use after move

#### Scenario: Reinitialization by assignment
- **WHEN** a moved `var` is assigned a new value (`var s = a; take(s); s = b; take(s)`)
- **THEN** it is usable again after the assignment

#### Scenario: Consuming method moves the receiver
- **WHEN** `consuming fun User.toAccount(): Account` is called as `u.toAccount()` and `u` is used afterwards
- **THEN** the later use is rejected as a use after move (the type checker already treats the receiver as a consuming position; see `ownership`)

#### Scenario: Plain enum is move-only without the marker
- **WHEN** `enum Color { Red, Green }` is declared without a marker and a value is passed by value twice
- **THEN** the second use is a use after move whose help suggests adding `: Copy` to `Color`

#### Scenario: String is move-only
- **WHEN** `val b = a` is declared for `a: String` and `a` is used afterwards
- **THEN** the use is rejected as a use after move, with help suggesting `a.clone()`; a string literal evaluates to a fresh owned `String`

#### Scenario: Deterministic scope exit cleanup
- **WHEN** an owning variable leaves its enclosing block scope
- **THEN** its destructor is executed immediately and deterministically, dropping owned fields and heap allocations without garbage collector intervention

### Requirement: Recursive Heap Indirection via Heap<T>
Self-referential or recursive structures MUST use the standard `Heap<T>` primitive to provide single-owner heap allocation. Constructing a `Heap<T>` and moving its contents out are not designed yet (see `ROADMAP.md`).

#### Scenario: Dropping Heap<T> allocation
- **WHEN** an instance of `Heap<T>` leaves scope
- **THEN** the allocated heap buffer and the inner value `T` are freed deterministically with zero reference-counter traffic

### Requirement: Stack-Scoped Borrowing and Mutation Rules
The borrow checker MUST enforce borrowing rules: arbitrary concurrent shared borrows (`&T`), or exactly one exclusive mutable borrow (`&mut T`), strictly confined to function arguments and local execution scopes. A borrow MUST last until the last use of the reference (non-lexical), not until the end of its block.

#### Scenario: Concurrent shared borrows
- **WHEN** multiple functions inspect the same resource concurrently via `&T`
- **THEN** the compiler allows the shared reads

#### Scenario: Exclusive mutable borrow prevents concurrent access
- **WHEN** an exclusive borrow `&mut T` is live on a value
- **THEN** any other borrow or access through the owner is rejected at compile time (conflicting borrow, `E5xxx`), and assigning or moving a value while it is borrowed is rejected (`E5xxx`)

#### Scenario: Borrow ends at its last use
- **WHEN** a body runs `var x = 1; val r = &x; print(r); x = 2`
- **THEN** the assignment is accepted, because `r` is not used after `print(r)`

#### Scenario: Disjoint field borrows
- **WHEN** `&mut o.user` and `&mut o.email` are live at the same time
- **THEN** the borrows are accepted, while a borrow of `o` itself conflicts with either

#### Scenario: Two-phase receiver borrows
- **WHEN** a `mut fun` method is called with an argument that reads the receiver (`v.push(v.size())`)
- **THEN** the call is accepted, because the receiver's `&mut` borrow starts when the call happens, after the arguments are evaluated

#### Scenario: Closures capture by inferred borrow
- **WHEN** a closure reads a captured variable, or writes one (`val f = { u.name = "z" }`)
- **THEN** the closure holds a shared or mutable borrow of that variable for as long as the closure value is live, and conflicting uses of `u` meanwhile are rejected

#### Scenario: Capturing closures do not escape
- **WHEN** a closure that captures any variable, or a function-typed parameter, is returned, stored in a struct field, or assigned to a top-level declaration
- **THEN** the borrow checker rejects it (`E5xxx`) until escaping, move-capturing closures are designed; a closure with no captures may escape

### Requirement: The Four Lifetime Elision Rules
When a function returns a borrowed reference, its lifetime MUST be inferred deterministically from the signature alone, and borrowing a top-level `val` MUST always be valid, by the four lifetime elision rules, without explicit lifetime annotations. The rules need nothing beyond the exported signature (parameter types and receiver mode), so callers in other files apply them to the imported ABI. Input borrows are parameters whose type contains a reference (`&T`, `&mut T`, `(&T)?`) and the receiver of a non-`consuming` method; function-typed parameters and `consuming` receivers are not input borrows. Function types (`(&User) -> &String`) elide by the same rules.

#### Scenario: Borrow of a top-level val
- **WHEN** `val DEFAULT = User("guest")` is declared at top level and a function returns `&DEFAULT`
- **THEN** the borrow is accepted, including from a function with no input borrows, and the caller's borrow is unconstrained

#### Scenario: Rule 1 - Single input borrow parameter
- **WHEN** a function signature has exactly one input borrow (`fun firstName(team: &Team): (&String)?`)
- **THEN** the returned borrow's lifetime is tied to that argument at the call site, and the body must return a borrow derived from that parameter

#### Scenario: Rule 2 - Method receiver borrow
- **WHEN** a method with a borrowed receiver (`fun Team.captain(): &User` or a `mut fun`) returns a reference, whatever its other parameters
- **THEN** the returned reference's lifetime is tied to the receiver only, and returning a borrow derived from another parameter is rejected with its own `E5xxx` code

#### Scenario: Rule 3 - Multiple input borrow parameters
- **WHEN** a function accepts multiple input borrows and returns a borrow
- **THEN** the returned reference borrows from every input borrow at the call site, each in its own mode (a `&mut` source stays frozen while the result is live), and the body may return a borrow derived from any of them

#### Scenario: Rule 4 - Zero input borrows returning a borrow
- **WHEN** a function has zero input borrow parameters and attempts to return a borrow to a local value
- **THEN** compilation MUST fail with `DanglingBorrowReturn` (an `E5xxx` code)

#### Scenario: Returning a borrow of a local under any rule
- **WHEN** a function with input borrows returns a borrow of one of its locals or of a temporary
- **THEN** compilation fails with `DanglingBorrowReturn`

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

- **Non-lexical borrows.** Liveness on the MIR makes them natural; lexical borrows reject code such as `val r = &x; print(r); x = 2`.
- **Closures are conservative until escaping closures are designed.** Inferred-borrow captures plus "capturing closures cannot escape" are sound with no new syntax; move-capturing closures, which compose's `onClick` needs, come in their own change.
- **Elision needs nothing new in the ABI.** A returned borrow's sources follow from parameter types and the receiver mode, both already exported, so files still compile independently and builds stay hermetic.
- **`&String?` parses as `&(String?)`.** Postfix binds tighter than prefix, as in expressions (`&x.y` borrows `x.y`), and `&mut String?` is the natural type for a mutable nullable slot. An optional borrow is written `(&String)?`.

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

### Requirement: Method Call Evaluation Semantics
The type checker decides how each extension call passes its receiver (the `extension-functions` spec, from `extension-methods`). At runtime, a call `recv.name(args)` MUST evaluate `recv` first, then the arguments left to right, then borrow or move the receiver by the extension's mode and make the call. A temporary receiver MUST be dropped at the end of the statement. A `?.` call MUST evaluate its receiver once, and MUST evaluate no argument and give `null` when it is `null`.

#### Scenario: Receiver evaluated before the arguments
- **WHEN** `make().greet(next())` runs for `fun make(): User`, `fun next(): Int` and `fun User.greet(n: Int)`
- **THEN** `make()` is called before `next()`, and the `User` it returned lives until the statement ends, when it is dropped

#### Scenario: A mutable receiver is borrowed after the arguments
- **WHEN** `v.push(v.size())` runs for `mut fun Stack.push(n: Int)` and `fun Stack.size(): Int`
- **THEN** `v.size()` runs before `v` is borrowed mutably for `push`, which is what lets two-phase borrows accept it

#### Scenario: A safe call on null evaluates no argument
- **WHEN** `u?.greet(next())` runs with `u` equal to `null`
- **THEN** `next()` is not called and the expression is `null`

---

## 5. Compose UI Engine

Implemented today: parsing `compose fun` ([`syntax`](../openspec/specs/syntax/spec.md)) and rejecting composable calls outside composable contexts (`E4008`, [`type-system`](../openspec/specs/type-system/spec.md)). The slot table, `remember` / `mutableStateOf`, the `compose fun` transform and hot-reload rendering are planned; see Phase 4 in [`ROADMAP.md`](../ROADMAP.md). The state-handle requirement is in section 3 above.
