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

Implemented today (`codegen-core`, `codegen-aggregates`, `runtime-strings-and-drops` and `heap-boxes`, [`code-generation`](../openspec/specs/code-generation/spec.md) and [`execution`](../openspec/specs/execution/spec.md)): the Tier 1 Cranelift JIT behind `prism run`, for programs over `Int`, `Double`, `Bool`, `Char`, `String`, structs, enums, nullables, `Heap<T>` boxes and references to them, reaching `main` well under 15 ms, with drop and clone glue freeing owned memory deterministically and `prism run --check-allocations` verifying it; every call goes through a patchable function table and nothing is inlined, which hot reload needs. Generic bodies, closures and composable functions are rejected with `E6001` until their changes land. Hot reload and Tier 2 are planned below.

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

Implemented today: rejecting borrowed fields in structs ([`syntax`](../openspec/specs/syntax/spec.md)); basic borrow type compatibility, including rejecting `&mut` of a `val` and requiring `&mut` on an argument passed to a `&mut T` parameter (`E4004`, [`type-system`](../openspec/specs/type-system/spec.md)); and, from `value-semantics`, the `: Copy` and `: Clone` markers, the built-in `Copy` and `Clone` tables, explicit `.clone()`, whole-value moves (`E4017`) and immutable top-level declarations ([`ownership`](../openspec/specs/ownership/spec.md)); and, from `reference-access`, implicit shared borrows at argument sites, copy-out of `Copy` values from borrows, moving out of a borrow (`E4017`), prefix `*` for writes, re-pointing `var` references, reborrowing `&mut` arguments, the `&(T?)` to `(&T)?` view and invariant references ([`references`](../openspec/specs/references/spec.md)); and, from `extension-methods`, methods as extension functions whose receivers are borrowed shared, borrowed mutably or moved by their `fun` / `mut fun` / `consuming fun` mode with nothing written at the call site ([`extension-functions`](../openspec/specs/extension-functions/spec.md), [`references`](../openspec/specs/references/spec.md), [`ownership`](../openspec/specs/ownership/spec.md)); and, from `borrowck-moves`, single-owner values checked by the borrow checker: use after move (`E5001`) through branches, loops, consuming receivers and closure captures, reinitialization by assignment, uninitialized reads (`E5002`), and drops elaborated so that each value is dropped exactly once where its owner's scope ends ([`ownership`](../openspec/specs/ownership/spec.md), [`mir`](../openspec/specs/mir/spec.md)); and, from `borrowck-exclusivity`, stack-scoped borrows checked by the borrow checker: any number of shared borrows or one mutable borrow, each lasting until the last use of the references holding it, with conflicting borrows and reads (`E5003`), assignments and moves of borrowed values (`E5004`, `E5005`) and values that end while borrowed (`E5006`) rejected, disjoint field borrows, two-phase receiver borrows, call results that keep their input borrows (the caller's half of the elision rules), closures that capture by inferred borrow and never move a capture (`E4017`), and capturing closures and function-typed parameters that cannot escape (`E5007`) ([`references`](../openspec/specs/references/spec.md), [`ownership`](../openspec/specs/ownership/spec.md), [`mir`](../openspec/specs/mir/spec.md)); and, from `borrowck-elision`, the four lifetime elision rules from signatures alone, for callers and callees, with returned borrows of locals, temporaries and by-value parameters (`E5008` `DanglingBorrowReturn`) and returned borrows derived from untied inputs (`E5009`) rejected, function types eliding by the same rules, defaults borrowing only through shared references, and borrows of top-level `val`s valid everywhere ([`references`](../openspec/specs/references/spec.md)); and, from `runtime-strings-and-drops`, deterministic cleanup at run time: each drop the elaborated MIR keeps runs the value's drop glue, freeing its owned memory where its owner's scope ends, where it is overwritten (`*r = v` drops the old referent) or in the code it was moved to, `clone()` builds values sharing no owned memory, top-level values are never dropped, and `prism run --check-allocations` checks that every allocation is freed exactly once ([`execution`](../openspec/specs/execution/spec.md), [`code-generation`](../openspec/specs/code-generation/spec.md)); and, from `heap-boxes`, `Heap<T>` as the single-owner heap allocation of recursive data: `Heap(value)`, boxes read like references to their contents, `*h = v` writes them, `h.unbox()` and consuming receivers move them out, patterns look through boxes, boxes are invariant, and dropping a box drops its contents then frees it, cloning allocates a new box holding a clone, with no reference counting ([`type-system`](../openspec/specs/type-system/spec.md), [`references`](../openspec/specs/references/spec.md), [`ownership`](../openspec/specs/ownership/spec.md), [`patterns`](../openspec/specs/patterns/spec.md), [`execution`](../openspec/specs/execution/spec.md), [`code-generation`](../openspec/specs/code-generation/spec.md)). Everything below is planned.

The "road to the borrow checker" in [`ROADMAP.md`](../ROADMAP.md), agreed with the user on 2026-10-17, is complete; escaping, move-capturing closures are left to `closures`. Flow-insensitive rules are type errors (`E4xxx`, in the type checker); flow-sensitive ones are borrow-checker errors (`E5xxx`, see section 1 for the pipeline).

### Requirement: Data-Oriented Typed Handles for Graphs
Graph topologies and cyclic data structures MUST use typed integer/handle IDs instead of raw pointers or reference-counting pointer cycles.

#### Scenario: Peer node reference via typed ID
- **WHEN** a struct references another struct via a typed identifier (`val parentId: NodeId?`)
- **THEN** no cyclic pointer graphs are formed and memory is managed by the owning collection

### Requirement: Reactive Tree Copy State Handles
Slot-table handles (`State<T>`, `MutableState<T>`, `NodeId`) MUST be `Copy` for every `T` and hold a 64-bit generational index (a 32-bit index and a 32-bit generation) into an arena the runtime owns, never a heap pointer; the arenas own the values, which section 5 gives single owners ("State Cells Belong to Their Creator").

#### Scenario: Capturing State<T> handle in event closure
- **WHEN** an escaping UI event closure captures a `MutableState<Int>` handle (`Button(onClick = { count.update { it + 1 } })`)
- **THEN** the handle is copied by value as a 64-bit ID into the closure's environment, and the closure holds no borrow (needs `escaping-closures`)

### Design notes (2026-10-17 exploration)
Why the requirements above were chosen; each change's `design.md` repeats the decisions it implements.

- **Closures are conservative until escaping closures are designed.** Inferred-borrow captures plus "capturing closures cannot escape" are sound with no new syntax and are implemented (`references`); move-capturing closures, which compose's `onClick` needs, come in `closures`.
- **`&String?` parses as `&(String?)`.** Postfix binds tighter than prefix, as in expressions (`&x.y` borrows `x.y`), and `&mut String?` is the natural type for a mutable nullable slot. An optional borrow is written `(&String)?`.

---

## 4. Runtime Semantics

Implemented: the runtime semantics of every construct the type checker accepts and the Tier 1 backend runs, over scalars, strings, structs, enums, nullables, boxes and references to them, are in [`execution`](../openspec/specs/execution/spec.md) and [`loops`](../openspec/specs/loops/spec.md) (`codegen-core`, `codegen-aggregates`, `runtime-strings-and-drops`, `payload-patterns`, `heap-boxes`), and their layouts, drop glue and clone glue in [`code-generation`](../openspec/specs/code-generation/spec.md). The order of evaluation, including when receivers are evaluated and borrowed, and where each temporary is dropped are fixed by the MIR ([`mir`](../openspec/specs/mir/spec.md)), which the backend follows. Nothing remains planned here: the runtime semantics of closures and composable functions come with `closures` and the compose changes (section 5).

---

## 5. Compose UI Engine

Implemented today: parsing `compose fun` and composable function types ([`syntax`](../openspec/specs/syntax/spec.md)), rejecting composable calls outside composable contexts (`E4008`, [`type-system`](../openspec/specs/type-system/spec.md)), marking composable calls in the typed tree and MIR ([`typed-tree`](../openspec/specs/typed-tree/spec.md), [`mir`](../openspec/specs/mir/spec.md)), and rejecting composable functions in the backend (`E6001`, [`code-generation`](../openspec/specs/code-generation/spec.md)). Everything below is planned: the plan of the Milestone 2 changes, decided on 2026-10-18 (`compose-architecture`, logged in [`docs/decisions-for-review.md`](decisions-for-review.md)) so that each change is proposed against one design. The state-handle invariant is in section 3 above.

The engine has four layers, each owned by one or two changes of the road to Milestone 2 in [`ROADMAP.md`](../ROADMAP.md):

| Layer | What it is | Changes |
|-------|------------|---------|
| Bundled library | `/prism/compose` (a prelude) and `/prism/ui`, Prism source shipped with the compiler, with `external` and `intrinsic` declarations | `standard-library` |
| Composition | the `prism-compose` crate (composer, slot table, restart scopes, state cells) and the MIR transform of composables | `compose-runtime`, `compose-transform`, `state-handles` |
| UI | the `prism-ui` crate (node tree, layout, display list, headless renderer, input) and the `prism-desktop` crate (window) | `ui-toolkit`, `desktop-window` |
| Development loop | hot reload of composables keeping slot-table state, and the verification apps | `compose-hot-reload`, `reactive-app-verification` |

### Requirement: Compose Runtime Crates Behind C Entry Points
The composition runtime MUST be Rust code in a `prism-compose` crate that knows nothing about UI, depending on no crate but `prism-runtime`. The UI node tree, layout, drawing and the headless renderer MUST be in a `prism-ui` crate, and the desktop window in a `prism-desktop` crate that only the CLI links. Generated code MUST reach them only through `extern "C"` entry points (`prism_compose_*`, `prism_ui_*`) called by symbol, as it reaches `prism-runtime` today. The runtime MUST run user code only through function values (descriptor, slot, environment) and drop entries, so a hot reload reaches every callback.

#### Scenario: Headless programs link no window code
- **WHEN** a program using `/prism/ui` runs with `prism run --headless`
- **THEN** it runs without opening a window, and the fixture tests of `prism-ui` build without `prism-desktop`

### Requirement: Bundled Standard Library and Compose Prelude
The compiler MUST ship Prism source packages under the reserved root package `prism`: `import /prism/ui` MUST resolve to the bundled library rather than the workspace, and a workspace package whose path starts with `prism` MUST be rejected. Their files MUST be checked like any file, and their ABI MUST be covered by the compiler version. Two declaration forms MUST be accepted only in bundled files: `external fun`, a body-less declaration of a runtime entry point called directly by symbol with the C calling convention and no function-table slot, and `intrinsic`, a declaration whose calls the compiler lowers itself. The public declarations of `/prism/compose` MUST be in scope in every file without an import, shadowed by any declaration or import of the same name.

#### Scenario: Compose primitives need no import
- **WHEN** `compose fun Counter() { val count = remember { mutableStateOf(0) } }` is checked in a file without imports
- **THEN** `remember` and `mutableStateOf` resolve to `/prism/compose`, and the ABI writes the handle type as `prism/compose.MutableState<Int>`

#### Scenario: UI toolkit is imported
- **WHEN** a file calls `Column { }` without `import /prism/ui`
- **THEN** the type checker reports an unresolved symbol (`E4001`)

#### Scenario: External declarations outside the bundled library rejected
- **WHEN** a workspace file declares `external fun f(): Int`
- **THEN** the compiler rejects it with a new error code taken by `standard-library`

### Requirement: Composable Calling Convention
The `compose fun` transform MUST give every composable function, composable closure and composable function type a machine signature of its declared parameters (the receiver first) followed by the address of the current composer and its change words: one 64-bit word per 32 parameters, two bits per parameter (uncertain, same, different, static) and a force bit. Default values MUST stay caller-evaluated generators, so no default mask is passed. The transform MUST be a pass over each body's elaborated MIR, run once per body before instantiation: a generic composable is transformed once, in its generic record, and its instances substitute the composed MIR. It MUST produce composed MIR that `prism dump-mir --composed` prints.

#### Scenario: Composer passed to composable calls
- **WHEN** `compose fun Label(n: Int) {}` and `compose fun Screen() { Label(1) }` are compiled
- **THEN** `prism dump-mir --composed` shows the call of `Label` passing `Screen`'s composer and a change word marking its argument static

### Requirement: Groups Keyed by Declaration Path and Lexical Position
Every composable body, composable call, branch of an `if` or `when` containing composable calls, loop body containing them and `key(...)` block MUST run inside a group whose key is a 64-bit hash of the enclosing top-level declaration's path (`app/main::Counter`) and the lexical path from it to the group: each enclosing construct (call, branch, loop, closure argument), with a call's callee declaration path and its ordinal among the siblings with the same construct and callee. A closure body MUST be reached through the call it is an argument of, not through its index (`#closure0`). Keys MUST NOT depend on byte offsets, lines or columns. The runtime MUST NOT rely on keys for memory safety: every slot records a fingerprint of its value's type and layout, and a slot whose fingerprint does not match is discarded and recreated.

#### Scenario: Unrelated edits keep keys
- **WHEN** a comment is added above `Counter`, its `Text` call's argument changes, or a `Button` call is inserted before its `Text` call
- **THEN** the group key of the `Text` call does not change

#### Scenario: Branches get their own groups
- **WHEN** `compose fun V(on: Bool) { if (on) { A() } else { B() } }` recomposes with `on` changed from `true` to `false`
- **THEN** the group of the `true` branch, with its slots, is removed and a group for the `false` branch is created

### Requirement: Slot Table of Groups and Owned Slots
The slot table MUST be a tree of groups in a generational arena, each group holding its key, kind (restart, replaceable, movable, node), parent, ordered children, slots, an optional node and an optional restart scope. A slot MUST hold a type-erased value with its size, alignment, drop entry and type fingerprint. Starting a group MUST reuse the first unvisited old sibling with the same key (so reordered siblings keep their state, and equal keys match in order) or create one; at a group's end, its unvisited old children MUST be removed. Removing a group, and forgetting a remembered value, MUST be deferred to the end of the pass, then drop children before parents and later slots before earlier ones.

#### Scenario: Removed groups drop their slots after the pass
- **WHEN** a composable that remembered a state stops being called, and another composable read that state earlier in the same pass
- **THEN** the borrow taken by the earlier read stays valid to the end of the pass, and the state's cell is freed afterwards

### Requirement: Positional Memoization with remember
`remember<T : Copy>(calc: () -> T): T` MUST be an intrinsic composable that runs `calc` the first time its group is composed, stores the result in a slot and returns a copy of it on every later pass. `remember(key, calc)` and `remember(key1, key2, calc)`, with `Copy` keys, MUST rerun `calc` and forget the old value when a key differs by `==`. Non-`Copy` data MUST be remembered in a state cell (`remember { mutableStateOf(list) }`).

#### Scenario: Remembered value survives recomposition
- **WHEN** `compose fun C() { val id = remember { println("computed"); 7 } }` is composed and then recomposed twice
- **THEN** `computed` is printed once

#### Scenario: Remembering a non-Copy value rejected
- **WHEN** `compose fun C() { val s = remember { "a" + "b" } }` is checked
- **THEN** the type checker reports an unsatisfied `Copy` bound (`E4021`)

### Requirement: Keyed Movable Groups
`key(value) { ... }` MUST be an intrinsic composable running its content in a movable group identified by the `Copy` value as well as the call's key, so that items reordered in a loop keep their groups, slots and nodes.

#### Scenario: Reordered keyed items keep state
- **WHEN** a loop emits `key(todo.id) { TodoRow(todo.id) }` for ids `1, 2, 3` and then for `3, 1, 2`
- **THEN** each row keeps its remembered state and its nodes, which are moved rather than recreated

### Requirement: Skipping Composables with Stable Parameters
A parameter MUST be stable when its type is `Copy` and holds no borrow, or when it is a function type whose value, at run time, owns an environment of `Copy` captures only. Stability MUST be read from declared markers alone, never inferred from a type's fields or from other files: a type is `Copy` by its declaration, and a type parameter only by a `Copy` bound, whatever its arguments. A composable returning `Unit` whose parameters are all stable MUST be skippable: it keeps its parameter values in its group (`Copy` values by copy, function values as their descriptor and a copy of their environment) and, when every argument is the same (by `==` for `Copy` values, through the type's equality glue when it is an aggregate, by descriptor and captures for function values, or by the caller's change bits) and its scope is not invalid, MUST skip its body, dropping the arguments it owns. A parameter of any other type (`String`, `&T`, a borrowing closure, a composable content lambda) MUST count as changed on every call. The runtime MUST NOT clone a value to compare it.

#### Scenario: Equal arguments skip the body
- **WHEN** `compose fun Badge(n: Int) { println("badge") }` is called as `Badge(1)` from a root content recomposed with `setContent`
- **THEN** `badge` is printed only on the first composition

#### Scenario: Handler capturing a state handle is stable
- **WHEN** `compose fun Plus(onClick: escaping () -> Unit) { println("plus") }` is called as `Plus(onClick = { count.value += 1 })` by a composable recomposed with the same `count` handle
- **THEN** `plus` is printed only on the first composition: the new closure has the same descriptor and equal captures

#### Scenario: String parameters are always changed
- **WHEN** `compose fun Title(s: String) { println("title") }` is recomposed by its caller with an equal string
- **THEN** `title` is printed again

### Requirement: Restart Scopes and Invalidation
A skippable composable MUST be a restart scope: when only its scope is invalid, the runtime recomposes it alone from its kept parameters with the force bit set. A read of a state during composition MUST register the innermost restart scope on the composer's stack as a reader, including reads in other composables' content lambdas and in plain functions called from composition. Invalid scopes MUST be recomposed in tree order, parents first, each at most once per pass, and a scope removed by its parent's recomposition MUST be dropped from the queue. The root content of a composition MUST be a restart scope.

#### Scenario: A read inside a content lambda invalidates the lambda's author
- **WHEN** `compose fun Counter() { val c = remember { mutableStateOf(0) }; Column { Text("Count: ${c.value}") } }` is composed and `c` is written outside composition
- **THEN** the next pass recomposes `Counter`'s scope, which calls `Column` and its content again

### Requirement: Compositions Are Owned Values
`composition(content: escaping compose () -> Unit): Composition` MUST compose its content once and return an owned `Composition`, neither `Copy` nor `Clone`. `c.setContent(content)` MUST replace the root content and recompose the root, `c.recompose(): Bool` MUST recompose invalid scopes and report whether any ran, and dropping a `Composition` MUST remove every group, dropping its slots, state cells, nodes and the closures they hold. `runApp` MUST be built on a composition.

#### Scenario: Dropping a composition frees everything
- **WHEN** `fun main() { val c = composition { Counter() } }` runs under `prism run --check-allocations`
- **THEN** the program exits with status 0: no allocation, state cell, group or node outlives `c`

### Requirement: State Handles Are Generational Copy Handles
`State<T>` and `MutableState<T>` MUST be intrinsic generic types of `/prism/compose`, `Copy` for every `T`, invariant in `T`, represented as one 64-bit word: a 32-bit cell index and a 32-bit generation. `MutableState<T>` MUST convert to `State<T>` as a subtype step that keeps the representation. `mutableStateOf<T>(value: T): MutableState<T>` MUST create a cell in the runtime's cell arena holding the value with its drop entry and type fingerprint. Using a handle whose cell was freed (a generation mismatch) MUST end the program with the runtime error "stale state handle" (status 101). An index whose generation reaches its maximum MUST be retired, never reused.

#### Scenario: Stale handle detected
- **WHEN** an event handler kept a handle remembered by a composable that has since left the composition and reads it
- **THEN** the program ends with the runtime error "stale state handle" and status 101

### Requirement: Reading and Writing State
`s.value = v` MUST move `v` into the cell and drop the old value, for every `T`, anywhere outside composition. For a `Copy` `T`, `s.value` MUST read a copy anywhere, and compound assignments (`s.value += 1`) MUST read and write it. For a non-`Copy` `T`, `s.value` MUST be a place that, in a composable context only, can be borrowed shared, as a top-level `val` can, its borrows valid until the body ends; reading it in any other context MUST be rejected with a new type error, with help naming `read`. `s.read { v -> ... }` (`(&T) -> R`), `s.update { v -> ... }` (`(T) -> T`) and `s.edit { v -> ... }` (`(&mut T) -> Unit`) MUST work anywhere; while one runs, the cell is borrowed, and a conflicting access to the same cell MUST end the program with the runtime error "state accessed while borrowed". A write MUST invalidate the cell's readers, except a `value =` write of a type with structural equality that is equal to the current value.

#### Scenario: Iterating a list state in a composable
- **WHEN** `compose fun Todos(todos: State<List<Todo>>) { for (todo in todos.value) { TodoRow(todo) } }` is checked
- **THEN** it is accepted: `todos.value` is borrowed for the loop

#### Scenario: Reading a non-Copy state in a handler rejected
- **WHEN** an `onClick` closure inside a composable reads `println(todos.value.size)`
- **THEN** the type checker reports the new state-access error, with help suggesting `todos.read { it.size }`

### Requirement: State Is Read-Only During Composition
A write to any state during composition MUST end the program with the runtime error "state written during composition", and a write in a composable context MUST be rejected statically by a new type error. `mutableStateOf` called in a composable context MUST be rejected statically, with help wrapping it in `remember { }`, and called during composition outside a `remember` calculation MUST end the program with a runtime error.

#### Scenario: Unremembered state rejected
- **WHEN** `compose fun C() { val s = mutableStateOf(0) }` is checked
- **THEN** the type checker reports the new state-creation error with help `remember { mutableStateOf(0) }`

### Requirement: State Cells Belong to Their Creator
A cell MUST be owned by the `remember` slot whose calculation created it; otherwise by the composition whose event handler or root content created it; otherwise by the program. A cell MUST be freed, dropping its value, when its owner is forgotten (at the end of the pass), when its composition is dropped, or, for program-owned cells, when `main` returns, before the allocation check.

#### Scenario: Forgotten state frees its value
- **WHEN** `remember(n) { mutableStateOf("item " + n.toString()) }` recomposes with a different `n` under `--check-allocations`
- **THEN** the old cell and its string are freed at the end of that pass

### Requirement: UI Node Tree Owned by the Runtime
UI nodes MUST live in an arena of `prism-ui`, addressed by `NodeId`, a `Copy` 64-bit generational handle of `/prism/ui`. The intrinsic composable `Node(kind, update, content)` of `/prism/ui` MUST create its group's node on the first pass and reuse it after, run the lent `update` closure with the node's `NodeId` (property setters are `external` functions that compare and mark layout or drawing dirty), and compose `content` as the node's children. Node insertions, moves and removals MUST be applied during composition, removals at the end of the pass with the group's other slots. A node MUST own the values it is given (strings, handler closures), dropping each when replaced or when the node is removed.

#### Scenario: Recomposition updates the node in place
- **WHEN** `Text("Count: ${c.value}")` recomposes with a different count
- **THEN** the same node's text is replaced, the old string is dropped, and the node's layout is marked dirty

### Requirement: Built-in Composables and Modifiers
`/prism/ui` MUST provide, as Prism composables over `Node`, `Column`, `Row`, `Box`, `Text`, `Button`, `Spacer`, `Checkbox` and a single-line `TextField`, and `runApp`. Each layout composable MUST take a `modifier: Modifier` and its own named parameters (`spacing`, `alignment`, `size`, `color`); event parameters MUST be `escaping` function types (`onClick: escaping () -> Unit`). `Modifier` MUST be a `Copy` struct of order-insensitive properties (padding, fixed or filling width and height, background, border), built by chained extension functions, so passing one never makes a composable unstable.

#### Scenario: Counter screen
- **WHEN** `import /prism/ui` and `compose fun Counter() { val c = remember { mutableStateOf(0) }; Column(modifier = Modifier().padding(16), spacing = 8) { Text("Count: ${c.value}"); Button(onClick = { c.value += 1 }) { Text("+") } } }` is checked
- **THEN** it is accepted, and its `Button` closure captures `c` by copy

### Requirement: Constraint Layout in Logical Pixels
Layout MUST be a single measure-and-place pass over the node tree: a parent passes minimum and maximum width and height to each child, the child returns its size, and the parent places it. Sizes and positions MUST be `Int` logical pixels; text MUST be measured with the bundled font's metrics in every renderer, so headless layouts equal windowed ones at scale 1. A clean node measured with the same constraints MUST reuse its last result.

#### Scenario: Column stacks children
- **WHEN** a `Column(spacing = 8)` holds two `Text` nodes 20 pixels high
- **THEN** the second is placed 28 pixels below the first

### Requirement: Input Events Dispatch to Node Handlers
Pointer and key events MUST be dispatched outside composition, one at a time. A press MUST hit-test the laid-out tree topmost first (reverse drawing order) for a node with a handler, and a release over the same node MUST call its handler through its function value; key events MUST go to the focused `TextField`. After the queued events, the frame loop MUST run one pass if any scope is invalid.

#### Scenario: Click increments a counter
- **WHEN** the counter screen receives a press and release over its `+` button
- **THEN** the handler writes `c`, the next pass recomposes `Counter`, and the text reads `Count: 1`

### Requirement: Rendering Through a Display List
After layout, the toolkit MUST build a display list of filled and rounded rectangles, borders and text runs. The desktop renderer MUST rasterize it on the CPU with `tiny-skia` and glyphs from the bundled font through `fontdue` into a pixel buffer presented with `softbuffer` in a `winit` window. The window and its event loop MUST run on the process's main thread, as macOS requires, while the program, its compositions and its handlers keep running on the program thread that `prism run` starts; input events and rendered frames MUST cross between them over channels.

#### Scenario: Window app runs and exits cleanly
- **WHEN** `fun main() { runApp(title = "Counter") { Counter() } }` runs under `prism run --check-allocations` and the window is closed
- **THEN** `runApp` disposes its composition and returns, and the program exits with status 0

### Requirement: Frame Loop of runApp
`runApp(title: String = "Prism", width: Int = 800, height: Int = 600, content: escaping compose () -> Unit)` MUST create a composition of `content` and loop on the program thread: wait for events, dispatch them, apply a pending hot reload, recompose invalid scopes, lay out dirty nodes, build the display list and render it, sleeping while nothing is invalid. When the window closes or the event script ends, it MUST drop the composition and return.

#### Scenario: Idle apps do no work
- **WHEN** a counter app runs with no input for one second
- **THEN** no pass, layout or rendering runs after the first frame

### Requirement: Headless Rendering and Scripted Input
`prism run --headless[=<width>x<height>] --events <script>` MUST run `runApp` without a window, feeding it the script's events in order: clicks on a node found by its text or at a position, typed text, keys, window resizes, `frame` and `dump`, and, with `compose-hot-reload`, `edit <file> <replacement>`. `dump` MUST print the laid-out node tree (kind, bounds, properties, one node per line) to stdout, and `runApp` MUST return when the script ends. `--screenshot <file.png>` MAY write the rasterized frame, which fixtures do not compare.

#### Scenario: Scripted counter session
- **WHEN** the counter program runs headless with the script `click "+"`, `click "+"`, `dump`
- **THEN** stdout holds a tree whose text node reads `Count: 2`, and the run passes the allocation check

### Requirement: Hot Reload of Composables Keeps State
Under `prism run --watch`, a reload MUST be applied between frames, never during a pass or a handler. After `hot-reload` repoints the function table, every restart scope of a composable defined in a reloaded file MUST be recomposed with its force bit set; groups whose keys still match MUST keep their slots, slots whose type fingerprint changed MUST be discarded and recreated (resetting that state and freeing its cells), and groups no longer emitted MUST be removed. A function value MUST never run code compiled for a different environment layout: a closure whose captures changed MUST get a new slot, and old values keep calling the old code until they are replaced. Old code MUST stay mapped while any drop entry or descriptor may point into it.

#### Scenario: Editing a composable body keeps its state
- **WHEN** the counter app has been clicked three times and `Text("Count: ${c.value}")` is edited to `Text("Clicks: ${c.value}")`
- **THEN** after the reload the window shows `Clicks: 3`

#### Scenario: Changing a remembered type resets that state only
- **WHEN** `remember { mutableStateOf(0) }` is edited to `remember { mutableStateOf("") }` in one composable
- **THEN** that composable's state restarts from `""`, and every other composable keeps its state

### Requirement: Reactive App Verification
`examples/apps/counter` and `examples/apps/todo` (add, toggle and remove items, with a `TextField`, `List<Todo>` state and `key`) MUST run headless as executable fixtures under `--check-allocations`, which MUST also count live state cells, groups and nodes as leaks when the program ends. `prism run --frame-stats` MUST report the median and maximum time of recomposition, layout and rasterization per frame. On a release build, a counter click MUST be recomposed, laid out and rendered headless within 1 ms (median), and toggling one item of a 1,000-item todo list within 4 ms of recomposition and 16 ms of frame time at 800×600.

#### Scenario: Todo app session leaks nothing
- **WHEN** the todo app runs a script adding three items, toggling one and removing another
- **THEN** the final dump shows two items, one checked, and the run ends with status 0

### Design notes (2026-10-18 compose architecture)
Why the requirements above were chosen, with the alternatives rejected; each is a `compose-architecture` row in [`docs/decisions-for-review.md`](decisions-for-review.md), and each change's `design.md` repeats the decisions it implements.

- **D1. Rust runtime crates, split by concern.** The composer, slot table and cell arena are data-structure code that needs raw memory and type-erased values, which Prism cannot express before traits, collections and an unsafe layer exist; a Prism runtime would also wait for self-hosting. `prism-compose` is UI-agnostic (its `Node` groups hold opaque node handles from an applier that `prism-ui` provides, and a test applier in unit tests), so it can later drive other trees. The window is its own crate so that headless tests and Tier 2 apps without windows build without `winit`. Rejected: a compose runtime written in Prism, one crate for everything.
- **D2. A bundled standard library with a compose prelude.** Declaring the toolkit in Prism source gives named parameters, defaults, the ABI, `prism fmt` and later the LSP for free, and composables written in Prism exercise the transform and hot reload; hand-written type-checker tables (as for `print` and `Range`) do not scale to dozens of declarations with defaults. `external` is the runtime entry point form an FFI and a self-hosted standard library will need anyway, and `intrinsic` keeps a declared signature for what the compiler lowers itself (state members need special typing, `remember` and `key` need the composer). The compose primitives are a prelude because compose is part of the language (the first-class compose invariant), while the toolkit is a library imported like any package. The bundled files are covered by the compiler version, which every artifact key holds, so the deterministic hermetic builds invariant is kept. Rejected: global built-ins in the type checker, auto-importing the toolkit, an `import /prism/compose` in every composable file.
- **D3. The transform is a MIR pass after borrow checking, before instantiation (revised for compile speed 2026-10-19).** The borrow checker should not see composer plumbing (the composer is runtime memory reached by address), and skipping needs the parameter drops elaboration placed (a skipped body still drops its owned arguments), so the pass runs on each body's elaborated MIR. It runs once per body, before `monomorphization`: a generic composable is transformed once and its record holds the composed MIR, which each instance only substitutes, and `instance-cache` keys and stores that. Stability is read from declared markers (D9), so it needs no concrete type: a type-parameter parameter is stable exactly when it is bounded `Copy`, and the `==` that compares it is lowered per instance by the backend like any `==` on a substituted type. `prism dump-mir --composed` makes it snapshot-testable. The first version ran the pass on each unit's concrete MIR after instantiation, so that stability was decided per instance: that repeats the transform for every instance of a generic composable and makes an instance's code depend on a stability decision taken per type argument; the revision costs a `T : Copy` bound on a generic composable that should skip (Jetpack instead infers stability per type argument at run time). Rejected: rewriting the typed tree as Kotlin's IR plugin rewrites IR (the borrow checker would check the plumbing), generating it only in Cranelift (untestable), transforming after instantiation (above).
- **D4. An explicit composer parameter.** Passing the composer and change words keeps several compositions (windows, tests) independent and costs no thread-local lookup per call. Default values are generated by the caller (`mir-lowering`), so Jetpack's default mask is not needed. Rejected: a thread-local current composer, which would keep composable signatures unchanged but hide which composition a call belongs to.
- **D5. Keys from declaration paths and lexical paths.** Jetpack hashes source offsets, which every edit above a call changes, losing state on hot reload. A key made of the body symbol (the identity MIR gives bodies, `mir-lowering`), the construct path and the ordinal among same-callee siblings survives formatting, comments, argument edits and inserted calls of other callees. Keys only decide which state is reused; type fingerprints make a wrong match safe. Rejected: source offsets, a per-body call index (an inserted call shifts every later key).
- **D6. A generational arena tree as the slot table.** Groups with parent and child links make moves, keyed lookup and restart anchors simple, and generational group ids serve as restart scope handles; Jetpack itself is moving from its gap buffer to a linked table. The representation is behind the `prism_compose_*` entry points, so it can change without touching the transform. Removals are deferred to the end of the pass so that borrows taken during the pass stay valid (D13). Rejected: Jetpack's gap buffer of groups and slots (anchors and gap moves for little benefit at this scale).
- **D7. Node changes applied during composition.** Jetpack records a change list and applies it through an `Applier` afterwards so that a composition can be abandoned or run concurrently; Prism composes on one thread and never abandons a pass, so applying directly is simpler. Rejected: a recorded change list.
- **D8. `remember` keeps `Copy` values only.** Returning a remembered non-`Copy` value would need a borrow tied to the composer, which conflicts with every later composable call, or a hidden clone. Handles are `Copy`, so `remember { mutableStateOf(list) }` covers non-`Copy` data, and type arguments are already borrow-free (`E4022`). Rejected: `remember` returning `&T`, remembering non-`Copy` values returned by move once.
- **D9. Stability without clones.** Jetpack compares parameters by reference or `equals`, which a GC makes free; in Prism keeping a non-`Copy` argument to compare it later would mean a hidden clone, against the explicit `.clone()` rule of `ownership`. `Copy` values are kept by copy, and closures whose environments hold only `Copy` captures (the common `{ count.value += 1 }`) by descriptor and environment copy, compared by a per-closure equality entry that `compose-transform` adds to closure descriptors (D3 of `closures`). Two change bits per parameter let callers pass "static" for literals and top-level values and "same" for forwarded unchanged parameters. Stability is read from the declared `Copy` marker, which Prism already requires for copying, so it costs one lookup per parameter type and no analysis: Jetpack's compiler instead infers the stability of every class from its fields, across modules, and needs `@Stable` and `@Immutable` annotations and stability configuration files where inference fails; the declared marker plays their role here. Rejected: keeping clones of `Clone` parameters (a later opt-in), comparing hashes (unsound), never skipping, inferring stability from fields (a whole-program analysis that makes a composable's skipping depend on other files' private fields).
- **D10. Restart scopes only where parameters are kept.** Restarting a body needs its arguments; only stable ones are kept, so a composable with other parameters is not a restart scope and its reads invalidate the nearest one above it, which reruns it with fresh arguments. Content lambdas borrow their author's frame, so they are not restart scopes either (Jetpack's restartable lambdas rely on the GC), and the author recomposes instead. Rejected: restarting only at the root (correct but every change recomposes the whole tree), restartable lambdas.
- **D11. Compositions are owned values.** An owned `Composition` gives disposal a place (its drop), lets tests drive passes from `main` before the toolkit exists (`compose-transform` and `state-handles` test through `composition`, `setContent` and `recompose`), and allows several compositions. Rejected: one global composition started by `runApp` only.
- **D12. Two handle types, one word each.** `MutableState<T>` and `State<T>` follow Kotlin, and a read-only parameter says a composable does not write; both are the same 64-bit word, so the conversion is a subtype step like `&mut T` to `&T`. Generations catch handles that outlive their cell at the cost of a compare per access, where Rust's `slotmap` has the same check. Rejected: one mutable type, owning (non-`Copy`) state values, which the state-handle invariant excludes.
- **D13. Borrowing state contents during composition.** During composition no state can be written (D14) and no cell is freed before the pass ends (D6), and a borrow cannot leave the body it was taken in (escaping closures hold no borrows, `E5007`; a returned borrow needs a borrowed input, `E5009`; struct fields cannot be borrows), so a shared borrow of `s.value` in a composable context is valid for the whole body, which allows `for (todo in todos.value)` as in Kotlin. Outside composition handlers can write any cell through any copy of a handle, which no static check can see, so access goes through `read`, `update` and `edit` closures, whose runtime borrow flag catches re-entrant access the way `RefCell` does. Rejected: run-time-checked place borrows released at their last use (needs the borrow checker to emit releases), a closure-only API (no `for` over a list state in composables).
- **D14. State is read-only during composition.** Jetpack allows writes during composition through its snapshot system and warns about backward writes; without snapshots a write during the pass would invalidate borrows (D13) and produce order-dependent results. `mutableStateOf` in a composable context is always the unremembered-state bug that Jetpack's lint reports, so it is an error. Side effects belong in handlers (and later in effect APIs). Rejected: allowing writes, deferring writes to the end of the pass.
- **D15. Cells belong to their creator.** Ownership by the creating `remember` slot gives every cell a single owner with no counting, like Solid's ownership tree. Cells made in handlers belong to their composition, so an app that creates states in handlers keeps them until it ends; an explicit release or state values owned by data structures is a later option if apps need it. Rejected: reference counting (against the zero hidden ARC invariant), rejecting `mutableStateOf` outside `remember` (handlers legitimately create state).
- **D16. No snapshot system.** One program thread runs compositions and handlers, so there is nothing to isolate; reads register the innermost restart scope, writes invalidate readers, and an equal `value =` write invalidates nothing (Jetpack's structural equality policy). The comparison is a call of the value type's equality glue at the write site, inline only for scalars (revised for compile speed 2026-10-19: the first version expanded it inline at every write site, which repeats code per site and, for large structs, is super-linear in the type's size, the cost the `codegen-aggregates` follow-up on per-type equality measured). Every equality the compose runtime needs, for skipping (D9), `remember` keys and handler captures, goes through the same glue. `update` and `edit` always invalidate. Rejected: Jetpack's MVCC snapshots, until threads exist.
- **D17. Toolkit composables in Prism over one node primitive.** `Node(kind, update, content)` plus `external` property setters keep the Rust surface small and let `Column` and `Button` be ordinary, hot-reloadable composables. `Checkbox` and a single-line `TextField` (printable characters, backspace, enter; no selection or IME) are included because the todo app needs them. Rejected: composables implemented in Rust, an `Applier` interface in Prism.
- **D18. Modifiers as a `Copy` property struct.** Jetpack's ordered modifier chain (`padding` then `background` differs from the reverse) needs an allocated list per call and makes every composable taking one unstable here (D9); a `Copy` struct of order-insensitive properties allocates nothing, compares by `==` and covers padding, size, fill, background and border. The demo's `Column(padding = 16)` becomes `Column(modifier = Modifier().padding(16))` (the empty value's spelling is `ui-toolkit`'s). Rejected: an ordered chain (a later extension), named layout parameters on every composable.
- **D19. Single-pass constraint layout.** Parent constraints and child sizes, as in Jetpack and Flutter, cover columns, rows, boxes, padding, fixed and filling sizes and wrapped text, with a measure cache; integer logical pixels keep dumps exact. Rejected: a flexbox engine such as `taffy` (more than the toolkit needs), intrinsic measurements (later, for equal-width rows).
- **D20. CPU rendering in a `winit` window on the main thread.** `winit` 0.30 (Apache-2.0), `softbuffer` 0.4 (MIT or Apache-2.0), `tiny-skia` 0.11 (BSD-3-Clause) and `fontdue` 0.9 (MIT, Apache-2.0 or Zlib) are pure Rust apart from platform bindings and were built together on macOS arm64 on 2026-10-18. The bundled font is Inter (SIL Open Font License 1.1, shipped with its licence). Software rasterization at 800×600 fits a frame budget and keeps build times and determinism; the display list lets a GPU backend replace it later. `prism run` already runs `main` on a thread of its own (for its stack size), and macOS requires the window's event loop on the main thread, so the CLI's main thread serves windows while the program thread composes. Rejected: `wgpu` with `vello` or `glyphon` (large dependency tree, GPU-dependent output), running programs on the main thread.
- **D21. Headless mode with text dumps.** A text dump of the laid-out tree is exact on every platform and reviewable in a diff, and the existing run-fixture runner already compares stdout under the allocation check; scripts target nodes by text so they survive layout changes. PNG screenshots are for people, since anti-aliasing differs across CPUs. Rejected: PNG golden images, a Prism-level UI test API now (it needs `prism test`; a later item).
- **D22. Handlers are closures owned by nodes.** `escaping` handler parameters move the closure into the node, which drops it when replaced; the runtime calls it through its descriptor's slot, so reloaded code runs on the next click. Click on release over the pressed node, as desktop toolkits do. Rejected: a global handler table keyed by `NodeId`, press-time clicks.
- **D23. Hot reload recomposes the reloaded files' scopes.** Recomposing every restart scope of a reloaded file is simple and catches every changed body for the price of one frame; keys (D5) and fingerprints decide what state survives, and resetting a changed-type slot is the answer `codegen-aggregates` (D10, D21) left open for slot-table state. A closure whose captures changed gets a new slot because a stored function value with the old environment would otherwise run new code over the wrong layout, which `hot-reload` must also respect for function values outside compositions. Rejected: recomposing only bodies whose MIR hash changed (a later refinement), restarting the app, migrating state between layouts.
- **D24. Verification by scripted apps and measured frames.** The counter and todo apps exercise state, lists, keys, text input, skipping and disposal; counting live cells, groups and nodes extends "no allocation outlives its owner" to the runtime's arenas. The timing targets follow the `codegen-core` benchmark's approach (a release build, the median of repeated runs). Rejected: verifying by hand in a window only.
- **D25. Out of scope for Milestone 2.** Effects (`LaunchedEffect`, `DisposableEffect`, `SideEffect`), timers and animation, `derivedStateOf`, subcomposition and lazy lists, IME and text selection, accessibility, multiple threads, mobile targets and a GPU renderer. Each can be added without changing the decisions above.
- **Compile-time cost (audited 2026-10-19, when the user made fast compilation a primary criterion).** The transform is one pass over each body's elaborated MIR, once per body rather than per instance (D3, revised). Group keys are one hash per group from paths the pass already walks, with a per-construct counter for same-callee ordinals (D5). Stability is a marker lookup per parameter type (D9), never an inference over fields or other files. A closure descriptor's equality entry is one small function per closure that owns only `Copy` captures, a word compare for inline environments (`escaping-closures` D9), and equalities call per-type glue (D16, revised). The static state rules (D13, D14) are checks of the enclosing context's composable flag at each state access, linear in the body. The bundled library is checked like any file and cached under the compiler version (D2), so a build pays for it once. Nothing here is whole-program except instance collection, which `monomorphization` keeps linear in the instances.
