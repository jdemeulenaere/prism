# Prism Planned Architecture

This document holds the **planned** design of Prism subsystems that are **not implemented yet**. It is the counterpart of [`openspec/specs/`](../openspec/specs), which describes only behavior the compiler implements today.

- **Nothing in this document is implemented.** Treat it as design intent, not as a description of the code.
- Requirements keep the OpenSpec `Requirement` / `WHEN` / `THEN` form so they can be copied into a change's delta specs.
- When an OpenSpec change implements a requirement, its delta spec adds it to `openspec/specs/` and the change removes it from this document.
- Progress is tracked in [`ROADMAP.md`](../ROADMAP.md). The non-negotiable invariants these designs must respect live in [`openspec/specs/system-invariants`](../openspec/specs/system-invariants/spec.md).

---

## 1. Compilation Pipeline & Compilation Avoidance

Implemented today (see [`compilation-and-abi`](../openspec/specs/compilation-and-abi/spec.md)): Phase 1 ABI extraction, deterministic 256-bit BLAKE3 ABI and interface hashes, recording a hash of each generic function's body, and the first three stages of the Phase 2 semantic pipeline: the typed tree ([`typed-tree`](../openspec/specs/typed-tree/spec.md)), MIR ([`mir`](../openspec/specs/mir/spec.md)), and borrow checking each file's MIR alone for uses of moved and uninitialized values, followed by drop elaboration (`borrowck-moves`, [`compilation-and-abi`](../openspec/specs/compilation-and-abi/spec.md) and [`ownership`](../openspec/specs/ownership/spec.md)), and then for borrow conflicts and escaping closures on the elaborated MIR (`borrowck-exclusivity`, [`references`](../openspec/specs/references/spec.md)). Generic declarations are checked once, against their bounds, into polymorphic typed trees and MIR, with their signatures and marker parameters in the ABI ([`generics`](../openspec/specs/generics/spec.md)); instantiating them is planned for `monomorphization`. `prism check` avoids work through a per-user build cache ([`build-cache`](../openspec/specs/build-cache/spec.md), `abi-cache`): a file's Phase 2 is skipped when the cache holds a passing verdict for its content, module path and direct imports' interface hashes, and instance records stand in for the instances a generic body change invalidates. Phase 3 generates each file's code from its own elaborated MIR, as one code unit per file (`codegen-core`, [`code-generation`](../openspec/specs/code-generation/spec.md)); caching that code, in object-code entries keyed like check entries plus the target triple, is planned. The callee's side of returned borrows is planned in section 3. Everything below is planned.

### Requirement: Parallel Phase 2 Body Compilation
After Phase 1 completes, function bodies MUST be lowered and compiled in parallel against the imported ABI interface hashes.

#### Scenario: Phase 2 body lowering
- **WHEN** Phase 1 completes
- **THEN** function bodies are typechecked and compiled in parallel against the imported ABI interface hashes

### Requirement: Re-monomorphization on Generic Body Change
The instances of an exported generic function MUST be generated again when what they depend on changes, and only those instances: no importer's own artifact is rebuilt for a generic body change. `prism check` already records, per instance, the generic's signature and Monomorphization Hash and its type arguments, and reports which instances it would generate again (the instance records of [`compilation-and-abi`](../openspec/specs/compilation-and-abi/spec.md)); `monomorphization` fills those records with code.

#### Scenario: Changing generic function body
- **WHEN** the body of an exported generic function changes
- **THEN** only its instances are generated again, and the object code of the files that call it is reused

### Open decision: where generic instances live (`monomorphization`)
`generics` checks each generic declaration once into elaborated polymorphic MIR, but ships no generic body to importers (D15 of [`openspec/changes/archive/2026-10-18-generics/design.md`](../openspec/changes/archive/2026-10-18-generics/design.md)). The recommended direction: the exporter ships each generic function's elaborated polymorphic MIR, which is self-describing because calls record callee paths, declared parameter types and type arguments. Instances live in separate instance artifacts keyed by the generic's path, the type arguments and the generic's MIR hash, rather than in the importer's artifact. That keeps a file's artifact a function of its content and its direct imports' ABI hashes even when an instance calls generics of modules the importer does not import, and with import cycles, and regenerates only the instances whose generic body changed. It may make the Phase 1 body hash in the ABI hash unnecessary. The instance records of `abi-cache` are keyed by the generic's identity, signature hash and Monomorphization Hash and its module-qualified type arguments; an instance's code also depends on the layouts and markers of its type arguments and on the private helpers its body calls, which that key does not cover yet. Still to decide: whether exported generic bodies may reference private declarations (Rust ships what they reach; Swift's `@inlinable` allows only exported ones), how polymorphic recursion is rejected, and the MIR rewrites for `T?` instantiated with a nullable `T`, which flattens.

---

## 2. Two-Tier Native Execution

Implemented today (`codegen-core`, `codegen-aggregates` and `runtime-strings-and-drops`, [`code-generation`](../openspec/specs/code-generation/spec.md) and [`execution`](../openspec/specs/execution/spec.md)): the Tier 1 Cranelift JIT behind `prism run`, for programs over `Int`, `Double`, `Bool`, `Char`, `String`, structs, enums, nullables and references to them, reaching `main` well under 15 ms, with drop and clone glue freeing owned memory deterministically and `prism run --check-allocations` verifying it; every call goes through a patchable function table and nothing is inlined, which hot reload needs. `Heap<T>` values, generic bodies, closures and composable functions are rejected with `E6001` until their changes land. Hot reload and Tier 2 are planned below.

### Requirement: Two-Tier Native Execution Engine
The compiler MUST support two distinct native execution backends: Tier 1 (Cranelift JIT) for development and Tier 2 (LLVM AOT) for release.

#### Scenario: Tier 1 hot reload with disabled cross-file inlining
- **WHEN** code is running in development mode under Tier 1
- **THEN** cross-file inlining across compilation units is disabled, allowing function-pointer tables to be dynamically patched during hot reload

#### Scenario: Tier 2 release optimization
- **WHEN** the user builds with `prism build --release`
- **THEN** the compiler invokes the LLVM backend with whole-program optimization and Link-Time Optimization (LTO)

---

## 3. Memory Model & Resource Management

Compile-time memory safety: zero runtime garbage collection, zero hidden automatic reference counting (ARC), single-owner value semantics, stack-confined borrows with automatic lifetime elision, and data-oriented handles.

Implemented today: rejecting borrowed fields in structs ([`syntax`](../openspec/specs/syntax/spec.md)); basic borrow type compatibility, including rejecting `&mut` of a `val` and requiring `&mut` on an argument passed to a `&mut T` parameter (`E4004`, [`type-system`](../openspec/specs/type-system/spec.md)); and, from `value-semantics`, the `: Copy` and `: Clone` markers, the built-in `Copy` and `Clone` tables, explicit `.clone()`, whole-value moves (`E4017`) and immutable top-level declarations ([`ownership`](../openspec/specs/ownership/spec.md)); and, from `reference-access`, implicit shared borrows at argument sites, copy-out of `Copy` values from borrows, moving out of a borrow (`E4017`), prefix `*` for writes, re-pointing `var` references, reborrowing `&mut` arguments, the `&(T?)` to `(&T)?` view and invariant references ([`references`](../openspec/specs/references/spec.md)); and, from `extension-methods`, methods as extension functions whose receivers are borrowed shared, borrowed mutably or moved by their `fun` / `mut fun` / `consuming fun` mode with nothing written at the call site ([`extension-functions`](../openspec/specs/extension-functions/spec.md), [`references`](../openspec/specs/references/spec.md), [`ownership`](../openspec/specs/ownership/spec.md)); and, from `borrowck-moves`, single-owner values checked by the borrow checker: use after move (`E5001`) through branches, loops, consuming receivers and closure captures, reinitialization by assignment, uninitialized reads (`E5002`), and drops elaborated so that each value is dropped exactly once where its owner's scope ends ([`ownership`](../openspec/specs/ownership/spec.md), [`mir`](../openspec/specs/mir/spec.md)); and, from `borrowck-exclusivity`, stack-scoped borrows checked by the borrow checker: any number of shared borrows or one mutable borrow, each lasting until the last use of the references holding it, with conflicting borrows and reads (`E5003`), assignments and moves of borrowed values (`E5004`, `E5005`) and values that end while borrowed (`E5006`) rejected, disjoint field borrows, two-phase receiver borrows, call results that keep their input borrows (the caller's half of the elision rules), closures that capture by inferred borrow and never move a capture (`E4017`), and capturing closures and function-typed parameters that cannot escape (`E5007`) ([`references`](../openspec/specs/references/spec.md), [`ownership`](../openspec/specs/ownership/spec.md), [`mir`](../openspec/specs/mir/spec.md)); and, from `borrowck-elision`, the four lifetime elision rules from signatures alone, for callers and callees, with returned borrows of locals, temporaries and by-value parameters (`E5008` `DanglingBorrowReturn`) and returned borrows derived from untied inputs (`E5009`) rejected, function types eliding by the same rules, defaults borrowing only through shared references, and borrows of top-level `val`s valid everywhere ([`references`](../openspec/specs/references/spec.md)); and, from `runtime-strings-and-drops`, deterministic cleanup at run time: each drop the elaborated MIR keeps runs the value's drop glue, freeing its owned memory where its owner's scope ends, where it is overwritten (`*r = v` drops the old referent) or in the code it was moved to, `clone()` builds values sharing no owned memory, top-level values are never dropped, and `prism run --check-allocations` checks that every allocation is freed exactly once ([`execution`](../openspec/specs/execution/spec.md), [`code-generation`](../openspec/specs/code-generation/spec.md)). Everything below is planned.

The "road to the borrow checker" in [`ROADMAP.md`](../ROADMAP.md), agreed with the user on 2026-10-17, is complete; escaping, move-capturing closures are left to `closures`. Flow-insensitive rules are type errors (`E4xxx`, in the type checker); flow-sensitive ones are borrow-checker errors (`E5xxx`, see section 1 for the pipeline).

### Requirement: Recursive Heap Indirection via Heap<T>
Self-referential or recursive structures MUST use the standard `Heap<T>` primitive to provide single-owner heap allocation. Constructing a `Heap<T>` and moving its contents out are not designed yet (see `ROADMAP.md`). Dropping a `Heap<T>` MUST drop its contents and free its allocation, through the drop glue that `runtime-strings-and-drops` built for strings, structs, enums and nullables. A `Heap<T>` clone MUST allocate a new box holding a clone of its contents, running no user code.

#### Scenario: Dropping Heap<T> allocation
- **WHEN** an instance of `Heap<T>` leaves scope
- **THEN** the allocated heap buffer and the inner value `T` are freed deterministically with zero reference-counter traffic

#### Scenario: Struct clone clones each field
- **WHEN** `struct User : Clone { val name: String, val tags: Heap<Int> }` is cloned
- **THEN** the clone holds a new `String` buffer and a new `Heap` box with equal contents

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

## 4. Runtime Semantics

Implemented: the runtime semantics of every construct the type checker accepts and the Tier 1 backend runs, over scalars, strings, structs, enums, nullables and references to them, are in [`execution`](../openspec/specs/execution/spec.md) and [`loops`](../openspec/specs/loops/spec.md) (`codegen-core`, `codegen-aggregates`, `runtime-strings-and-drops`), and their layouts, drop glue and clone glue in [`code-generation`](../openspec/specs/code-generation/spec.md). The order of evaluation, including when receivers are evaluated and borrowed, and where each temporary is dropped are fixed by the MIR ([`mir`](../openspec/specs/mir/spec.md)), which the backend follows. Nothing remains planned here: the runtime semantics of `Heap<T>` are in section 3 (`heap-boxes`), and those of closures and composable functions come with `closures` and the compose changes (section 5).

---

## 5. Compose UI Engine

Implemented today: parsing `compose fun` ([`syntax`](../openspec/specs/syntax/spec.md)) and rejecting composable calls outside composable contexts (`E4008`, [`type-system`](../openspec/specs/type-system/spec.md)). The slot table, `remember` / `mutableStateOf`, the `compose fun` transform and hot-reload rendering are planned; see Phase 4 in [`ROADMAP.md`](../ROADMAP.md). The state-handle requirement is in section 3 above.
