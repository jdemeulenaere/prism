# Design: compose-runtime

## Context

See `proposal.md` for motivation. This is item 27 of "Then: the road to Milestone 2" in `ROADMAP.md`, the runtime layer of the compose plan in `docs/architecture.md` §5. It implements the runtime side of the `compose-architecture` decisions D1 (Rust crates behind C entry points), D6 (a generational arena tree as the slot table, removals deferred to the end of the pass), D7 (node changes applied during composition), D10 (restart scopes), D11 (compositions as owned values) and D23 (fingerprints make a wrong key match safe). Each is repeated below where this change refines it. The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Fast compilation is a primary criterion (user, 2026-10-19). This change adds no compiler stage, so each decision states its run-time cost, and its compile-time cost where it has one.

**Baseline.** This change was proposed while `closures` was being reviewed and archived, and `escaping-closures`, `collections`, `hot-reload`, `instance-cache` and `standard-library` were proposed but not implemented. It needs three of them:

- `closures`: a function value is 16 bytes, a descriptor address and an environment address. A descriptor's first word is the address of the closure code's function-table slot, and its second the environment's drop entry or null. Closure code takes `(result?, environment, parameters...)` with the host's C calling convention (its D2 to D6).
- `escaping-closures`: `escaping` parameters, closures that own their captures, heap environments freed through the drop entry, and inline one-word environments with a null drop entry (its D8 and D9). Restart functions are escaping closures over `Int` captures, so the common one allocates nothing.
- `standard-library`: bundled modules in `crates/prism-library/prism/`, `private external fun` declarations named by their C symbol and called directly with Prism's machine signature, the backend's check of externals against the runtime's table (`externals::check_externals`, `E6005`), and `library_tests.rs`. Its baseline brings `monomorphization`, `instance-cache`, `collections` and `hot-reload` too.

The Rust core (task groups 1 to 6) needs only `closures`' function value layout, which its tests build by hand, so it can be implemented before the others land. The Prism-level groups (7 and 8) need all three.

What exists today:

- **`prism-runtime`** has no dependencies. `functions.rs` lists every `extern "C"` function in `RUNTIME_FUNCTIONS` (symbol, address, `RuntimeType` parameters, result, never-returns flag), with a test that reads the crate's sources so that no exported function is missing from the table. `panic.rs` has `PanicKind` and `panic_without_location(kind, detail)`. `allocation.rs` has `finish_tracking`, which drops top-level values and builds the leak report. `string.rs` has `PrismString::owned`.
- **`prism-codegen`**: `link.rs::Linker::new` registers `RUNTIME_FUNCTIONS` with the JIT's symbol table, `unit.rs::runtime_function` imports an entry with the signature the table records, and `program.rs::Program::run` runs `main` on a thread of its own between `prism_runtime::start` and `prism_runtime::finish`.
- **Compose today** is syntax and checking only: `compose fun`, composable function types, `E4008`, composable calls marked in the typed tree and MIR, and `E6001` for composables in the backend.

**Baseline update (`standard-library`, applied 2026-10-18).** Names and mechanisms that differ from what this design assumed:
- The one lookup of runtime entry points is `prism_codegen::externals::entry_point(symbol)` (`crates/prism-codegen/src/externals.rs`), not `unit.rs::entry_point`: the external check and the lowering of `Callee::External` calls both use it. Extend it to search `COMPOSE_FUNCTIONS` after `RUNTIME_FUNCTIONS`; `UnitContext::runtime_function` still reads `RUNTIME_FUNCTIONS` alone and needs the same extension for runtime imports.
- `E6005` is checked by `prism_codegen::check_file` (`externals::check_externals`) over the externals a file declares, recorded in `MirFile::externals`, so an external is checked at its declaration even when nothing calls it; `UnitContext::external_function` does not exist.
- `prism_runtime::StopHandler` takes `(status, exited)`: `prism_rt_exit` reports `exited`, which a watch session treats as `main` returning.
- `MODULES` is a list of `(module path, source)` pairs, and every public library function writes its result type (`: Unit` too, `E4011`); `library_tests.rs` requires a `//` comment directly above every declaration line.

**Baseline update (`ui-toolkit` revision, user direction 2026-10-19).** `compose-transform`'s prelude module is now `/prism/compose/runtime`, one of the modules of `prism/compose/` that follow Jetpack Compose's artifacts (`ui-toolkit` D21). `/prism/composer` keeps its path and role: it is no Compose artifact, but the protocol module for fixtures written in lowered form.

**Reconciliation at apply time (2026-10-18).** Before implementing, the packet was compared with the main specs and the code that `standard-library` and the `ui-toolkit` revision left:
- `execution` "Runtime Errors End the Program" no longer holds the watch sentence and scenario: `hot-reload`'s archive moved them into "A Runtime Error Under Watch Ends Only the Run". The MODIFIED block now restates the current text, with only the misuse exception and its scenario added, and cites compose "Composition Misuse Is a Runtime Error".
- `standard-library` "The Compiler Ships the Library" is unchanged since the proposal, so its MODIFIED block stands. `/prism/composer` is a file of the package `prism` at the top of the library directory, so this change needs no subdirectory support. `/prism/compose/runtime`, the library's first module in a subdirectory (`package prism.compose`), is `compose-transform`'s, whose task 2.1 extends `MODULES`, `library_tests.rs`'s directory walk and sibling imports to `prism/compose/`. The naming does not collide: `prism/composer` and `prism/compose/runtime` are different module paths, and a file `prism/composer.prism` beside a directory `prism/compose/` is allowed.
- D2 and task 7.1 now name `externals::entry_point` and `externals::check_externals`, which `standard-library` built, in place of `unit.rs::entry_point` and `external_function`. `UnitContext::runtime_function` gets the same lookup.
- The `ui-toolkit` revision (no composed modifiers, the handler table in `prism-ui`) changes nothing here: it relies only on the `Applier` trait of D9, `prism_compose_start_node_group`, the per-thread state of D4 and `Handle`, which keep their packet names.
- Fixture lambdas with a parameter and an empty body, `{ k -> }`, already parse, type-check and run (checked with `prism run --check-allocations` before the change), so no parser work is needed.
- The handle generation starts at 1, so the word 0 is never a valid composition or scope, as `state-handles` D8 also assumes for its cells.

## Goals / Non-Goals

**Goals:**
- The composition runtime of §5, complete enough that `compose-transform`, `state-handles` and `ui-toolkit` only add callers, an applier and cells, and never change the slot table's behavior.
- A protocol fixed and tested before the transform depends on it, both from Rust and from real `.prism` source.
- Memory safety whatever a caller does: every misuse of the protocol is a runtime error, and no key match can give back a value of another type.
- O(1) amortized protocol operations and steady-state passes that allocate nothing.

**Non-Goals:**
- Computing group keys, change words, slot type records or restart functions: the transform does that (D1).
- State cells, the UI applier, the frame loop, and hot reload's recomposition policy (D1).
- Subcompositions, nested passes, threads, effects (`compose-architecture` D25).
- An owned Prism `Composition` type with drop glue (D11).

## Decisions

### D1. The boundary with the other compose changes (autonomous)

| Concern | `compose-runtime` (this change) | Later change |
|---|---|---|
| Slot table | groups, kinds, keyed matching, slots, cursors, deferred releases | — |
| Group keys | treated as opaque 64-bit words | `compose-transform` hashes declaration paths and lexical paths (§5 D5) and reduces `key(value)` to a data-key word |
| Slot types | the `SlotType` record, matching by fingerprint, drop and equality through it | `compose-transform` emits one record per type, from `hot-reload`'s type fingerprints |
| Change words, skipping decisions | none; `skipping`, `skipToGroupEnd`, `changed` are the primitives | `compose-transform` generates the comparisons and change bits |
| Restart functions | kept, replaced, called and released | `compose-transform` generates them, passing its force bit itself |
| `remember`, `key`, `/prism/compose/runtime`, the owned `Composition` | none | `compose-transform` |
| Reads, writes, cells | scope handles, `invalidate`, the current scope of the active pass (Rust API) | `state-handles` adds cells, readers and the C entries that read the active scope |
| Nodes | `Applier` trait, node groups, placement, the built-in node tree | `ui-toolkit` supplies the UI applier, `Node` and the frame loop |
| Hot reload | fingerprinted slots, kind-checked nodes, a Rust API to invalidate the scopes whose restart function reaches given slots | `compose-hot-reload` calls it at reload points |
| Leak counting | undisposed compositions | `reactive-app-verification` adds groups, cells and nodes |

The runtime knows nothing of change words because their layout (two bits per parameter and a force bit, §5 D4) belongs to the code that generates and reads them. The runtime only ever calls a restart function, and a restart function generated by the transform can pass its own force bit to the composable it reruns. This keeps the C interface free of a convention the transform may still revise.

Rejected: letting the runtime pass a force word to restart functions (it would fix the change-word layout here); including `remember` and `key` here (they are compiler intrinsics, which need the transform's lowering).

### D2. A `prism-compose` crate with a Rust API and a C entry table, linked by the backend (autonomous)

`crates/prism-compose` depends on `prism-runtime` only, as §5 D1 requires, and has no `unsafe` interface except its entry points and the slot type and function value records. Modules:

- `handle.rs`: `Arena<T>` with 32-bit indexes and generations, a free list, and retirement of an index whose generation reaches `u32::MAX`; `Handle` packs index and generation into a `u64`.
- `group.rs`: `Group` and `GroupKind`.
- `slot.rs`: `SlotType`, `Slot`, word slots.
- `composition.rs`: compositions, passes, the invalidation queue, releases.
- `composer.rs`: the protocol operations of a pass.
- `applier.rs`: the `Applier` trait and `NodeTree`.
- `thread.rs`: the per-thread state (D4).
- `dump.rs`, `entry.rs` (the `extern "C"` functions), `functions.rs` (`COMPOSE_FUNCTIONS`) and `test_support.rs`.

`COMPOSE_FUNCTIONS` reuses `prism_runtime::RuntimeFunction` and `RuntimeType`, with the same source-reading test as `RUNTIME_FUNCTIONS`. The Rust API (`Composition::start(applier, content)`, `recompose`, `set_content`, `dispose`, the composer operations) is what the unit tests and `prism-ui` call, and the entry points are thin wrappers over it.

`prism-codegen` depends on `prism-compose` and looks entry points up through one function, `externals::entry_point(symbol)`, which searches `RUNTIME_FUNCTIONS` then `COMPOSE_FUNCTIONS`. The external check (`externals::check_externals`), the lowering of external calls and the runtime imports (`UnitContext::runtime_function`) use it, and `Linker::new` registers both tables with the JIT. `ui-toolkit` adds `prism_ui::UI_FUNCTIONS` the same way. Neither crate depends on Cranelift, so linking them into the backend costs only their own code, and `prism-desktop`, which generated code never calls, stays out (§5 D1).

Calling Prism code from Rust needs the function value layout of `closures`, which `prism-ui`'s handlers need too. It goes into `prism-runtime/src/function_value.rs`: `FunctionValue { descriptor, environment }`, `Descriptor { slot, drop_environment }`, `unsafe fn call_with_word(&self, word: u64)` (load the slot's code with acquire ordering and call `code(environment, word)`), and `unsafe fn release(self)`. `escaping-closures` D9's inline environments need nothing special: the runtime passes the environment word as it is.

Run-time cost: one indirect call per entry point, as for every runtime function. Compile-time cost: an entry-point lookup over two small tables, once per external or runtime import a unit declares.

Rejected:
- Tables passed in by the CLI: every lowering entry point (dumps, the cache, tests) would need them.
- A registry filled at startup: a static order problem for a list known at build time.
- The compose runtime inside `prism-runtime`: §5 D1 keeps the base runtime minimal for Tier 2 programs without UI.

### D3. Handles: generational words for compositions, scopes and nodes; the composer is the pass's identifier (autonomous, revises `compose-architecture` D4)

- **Compositions and scopes** are `Handle`s: a 32-bit index and a 32-bit generation in one 64-bit word, the representation the state-handle invariant gives `State<T>` (§3). A scope's handle is its restart group's handle, so invalidating one is an arena lookup and a generation compare. A removed group's generation changes when it is released, so later uses find nothing.
- **Groups of every composition of a thread** live in one arena, so a scope handle needs no composition index. Each group records its composition.
- **The composer** is the 64-bit identifier of the pass in progress, taken from a per-thread counter that starts at 1. Since a thread runs one pass at a time (D4), an entry point validates a composer by one comparison with the active pass's identifier, and reaches the composition through the active pass. A composer kept past its pass, for instance by a restart function capturing the old one, is caught deterministically.
- **Nodes** are the applier's handles (`u64`). The node tree's are generational too.

§5 D4 planned the address of the composer as the composable's extra parameter. A word that is validated costs one load and one compare per entry point. In exchange, the protocol is memory-safe for any caller, which a public protocol module (D10) needs, and the runtime's composition storage can move. The transform passes the same word wherever D4 says "the composer's address", so the calling convention of §5 changes only in that word's meaning.

Rejected:
- Raw composer addresses: hand-written code could forge one.
- The composition's handle as the composer: a composer captured by a restart function would still be accepted in later passes.
- A thread-local current composer without a parameter: rejected by §5 D4 for the reasons given there.

### D4. One pass at a time per thread; invalidations during a pass go to the next one (autonomous)

The per-thread state (`thread.rs`) holds the composition arena, the group arena, the pass counter and the active pass, in a `thread_local!` of `UnsafeCell`s. Prism runs compositions and handlers on one program thread (§5 D16), and Rust tests run on many threads, each with its own state. Every runtime operation obtains the state, does its work and lets go before it calls program code, so no Rust reference is live while a content, restart, drop or equality entry runs, even if that code calls back into the runtime. The pass driver keeps raw indexes, never references, across those calls.

Starting, recomposing, setting the content of or disposing any composition during a pass is the misuse `composition operation during a pass`. Nested passes would only be needed for subcompositions (§5 D25). Releases at the end of a pass run after the pass has ended, so a drop entry that disposes another composition, as a captured owned `Composition` of `compose-transform` would do, is allowed. A scope invalidated during a pass, whoever invalidates it, is queued for the next pass, which makes a pass's set of scopes fixed when it starts. That keeps passes deterministic and cheap to order (D8). §5 D14 forbids state writes during composition anyway.

Run-time cost: one thread-local access per entry point.

Rejected:
- A global state behind a lock: a lock per entry point, and parallel tests would serialize.
- Allowing nested passes now: it needs a pass stack, and the composer could no longer be one compare.
- Recomposing scopes invalidated during the pass in the same pass, as Jetpack does: the order would depend on when the invalidation happened.

### D5. The group tree and keyed matching (implements `compose-architecture` D6)

A group record holds:
- its key, its data key for movable groups, and its kind;
- its composition and parent;
- `children: Vec<GroupId>` and `slots: Vec<Slot>`;
- `index_in_parent`, its last node (`Option<NodeHandle>`), its node for node groups, and its restart function for restart groups;
- flags: `invalid`, `removed`, and `visited` in the current pass.

The composer keeps a stack of open groups. Each entry holds the next old child to look at, the slot cursor, and the start of the group's new children in a scratch vector that the composition reuses from pass to pass.

Starting a group compares the key with the next unvisited old child. When they match, which is the steady state, it reuses it in O(1). On the first mismatch under a parent, it builds a map from (kind, key, data key) to the queue of that parent's unvisited old children, in order. The map is a `HashMap` the composer clears and reuses, and later starts under that parent look the key up there. Building it costs time linear in the remaining old children, once per parent per pass, so starts are amortized O(1). Equal keys match in order because the queues are FIFO. Ending a group copies its new children from the scratch vector into `children`, which reallocates only when the group grew, renumbers `index_in_parent`, and marks every unvisited old child removed. The removed children go on the pass's release list (D7).

Rejected:
- Jetpack's gap buffer (§5 D6).
- Children as an intrusive linked list: no `index_in_parent` for ordering (D8), and pointer chasing in the common case.
- Searching linearly on a mismatch: quadratic on reversed keyed lists.

### D6. Typed slots through static slot type records; word slots for hand-written code (autonomous)

A slot type is a static, read-only record that generated code points to:

```rust
#[repr(C)]
pub struct SlotType {
    pub fingerprint: [u64; 4],      // hot-reload D6: 256-bit structural type fingerprint
    pub size: usize,
    pub alignment: usize,
    pub drop: Option<unsafe extern "C" fn(*mut u8)>,
    pub equals: Option<unsafe extern "C" fn(*const u8, *const u8) -> u8>,
    pub name: *const u8,            // UTF-8 type name, for dumps
    pub name_length: usize,
}
```

A slot holds a pointer to its record and its value, inline in 16 bytes when the size is at most 16 and the alignment at most 8, or in a block the slot allocates otherwise. Matching compares record pointers first and fingerprints only when the pointers differ, as they do across reloads, since each module has its own records. Fingerprints are the full 256 bits of `hot-reload`'s type fingerprints, because a collision would give a value of one layout to code that reads another. The entry points for generated code are:

- `prism_compose_slot(composer, type) -> *const u8`: the address of the value at the cursor when it matches, moving the cursor, and null otherwise;
- `prism_compose_set_slot(composer, type, value)`: moves the value's bytes into the slot at the cursor, forgetting a mismatched one;
- `prism_compose_changed(composer, type, value) -> u8`: "Comparing a Value With a Slot". The new value is always moved in. When it is equal to the old one, the old value is kept and the new one released.

The transform emits the records (`compose-transform`), so no Prism code calls these entries in this change. Rust tests call them with records whose entries are Rust functions. The Prism-level protocol cannot name a record, so it has *word slots*: a built-in record for 8-byte values without drop, plus a 64-bit type tag kept in the slot and compared on reads. Their entries are `prism_compose_has_word`, `prism_compose_word`, `prism_compose_set_word` and `prism_compose_changed_word`, which `rememberInt` and `changedInt` wrap. The tag lets hand-written fixtures change a slot's type, as an edit does under hot reload. A slot's cursor is separate from the group's child cursor, so slots and child groups never shift each other.

Run-time cost: O(1) per operation, plus one allocation per new slot larger than 16 bytes. Compile-time cost: none here. `compose-transform` emits one record per slot type per unit, from fingerprints that `hot-reload` memoizes.

Rejected:
- Passing size, alignment, drop and fingerprint as separate arguments: five words per call instead of one.
- 64-bit fingerprints: a collision is a memory-safety bug.
- Typed entry points per scalar type for Prism code: one family per type, where a tagged word covers the fixtures.
- Interleaving slots and groups in one sequence, as Jetpack's slot table does: positional coupling with no benefit for a tree.

### D7. Deferred releases and their order (implements `compose-architecture` D6 and D13)

A pass keeps one release list per composition, reused between passes. Removing a group pushes it, forgetting a slot pushes the slot's record and value, and replacing a restart function or the root content pushes the old function value. All of them are released when the pass ends, after its last body has returned and the active pass is cleared. A removed group's subtree is released by an explicit post-order walk, not recursion, so deep trees do not overflow Rust's stack: its children last first, then its slots last first, then its restart function, then its node. The node is removed from its parent when it is the subtree's top node, and released after its descendants. Released groups go back to the arena with a new generation.

Releasing replaced restart functions at the end of the pass is required for memory safety. When a scope is recomposed, the restart function being replaced is the one whose code is running, and freeing its environment then would free memory its own frame may read. Deferring forgets is what lets a state borrowed earlier in the pass outlive the group that owned it (§5 D13). `state-handles` attaches cells to slots, and they are released after the slot's value.

Run-time cost: linear in what is released, and nothing when nothing is removed.

### D8. Restart functions, recomposition of one scope, and ordering (autonomous)

- **Restart functions** are owned `(Int) -> Unit` function values, passed by address and moved into the runtime by `prism_compose_end_restart_group`, which pushes the old one to the release list. The root group's restart function is the composition's content. A restart function capturing up to one word allocates nothing (`escaping-closures` D9). A larger one allocates its environment each time its scope ends. That is the closure's own cost, not the runtime's, and `compose-transform` can avoid it by reading kept parameters from slots.
- **Recomposing one scope** pushes a restart frame on the composer: the scope's parent group, with an old-children window of exactly the scope's group, and its node position (below). The frame then calls the restart function with the pass's composer. The window gives "Recomposing a Scope Alone" its meaning: a matching start reuses the group, a different first start removes it, later starts insert after it, and no start removes it. After the call, the frame checks that the composer's stack is back where it started (`group left open`).
- **Node position during a restart.** A node placed by a restarted scope goes after the last node, in group order, before the scope within the nearest enclosing node group. That node is found by walking from the scope towards that node group and looking at the previous siblings' `last_node`. This walk costs O(d · s) for depth d and s siblings, and it happens only when the restarted scope places or moves a node.
- **Ordering.** At the start of a pass, the invalid scopes are sorted in tree order. Each scope's path of `index_in_parent` values from the root is written into one reused buffer, and the paths are compared lexicographically, O(k log k · d) in all. Parents sort before their descendants. While the pass runs, a scope is skipped when it has been removed or is no longer invalid, because composing a group clears its `invalid` flag. Invalidations queued during the pass go to the next pass's queue (D4).
- **`skipping`** is true when the innermost open group is a restart group that was reused, not created, in this pass and is not invalid. `compose-hot-reload` adds a forced pass on top of this, through D15's API.

Rejected:
- Recomposing from the root on every invalidation (§5 D10).
- A queue ordered by depth only: unrelated scopes would run in an order that depends on depth, not position.
- Building the restart function only on the first composition: a reloaded body would keep old captures forever, which §5 D23 rules out.

### D9. Node groups, the `Applier` trait and the built-in node tree (autonomous, implements `compose-architecture` D7)

```rust
pub trait Applier {
    fn root(&self) -> NodeHandle;
    fn create(&mut self, kind: i64) -> NodeHandle;
    /// Puts `node` among `parent`'s children right after `previous` (first when `None`), inserting or moving it.
    fn place(&mut self, parent: NodeHandle, previous: Option<NodeHandle>, node: NodeHandle);
    fn remove(&mut self, parent: NodeHandle, node: NodeHandle);   // end of pass
    fn release(&mut self, node: NodeHandle);                       // after its descendants
    fn dump(&self, out: &mut String);
}
```

Placement is relative to the previous sibling rather than by index, because removals happen only at the end of the pass, so during a pass a parent's children still include nodes about to be removed and indexes would be ambiguous. Each open node group on the composer's stack keeps the last node placed into it in this pass. Starting a node group places its node after that node, and a `place` that changes nothing costs one comparison in the node tree, which keeps the steady state cheap. Skipping a group moves the "last node" mark to the skipped group's `last_node`. The node of a node group started with another kind is a new node, and the old one is released at the end of the pass, so a kind change after an edit never reuses a node of the wrong kind.

`NodeTree`, the built-in applier, is an arena of `{ kind, number, parent, first_child, last_child, previous, next }`. Its numbers count from 1 in creation order for dumps. Compositions started through the entry points use it, and the Rust tests use it or a recording applier. `prism-ui` passes its own applier to `Composition::start`. A Prism-level entry for that is `ui-toolkit`'s. The node tree costs one small arena per composition and lets this change test node groups, moves and removals from Prism.

Rejected:
- Index-based placement, as in Jetpack's `Applier`: ambiguous while removals are deferred.
- No node groups before `ui-toolkit`: the ROADMAP item includes them, and moving nodes with keyed groups is runtime logic.
- Compositions without an applier: every node group would then be an error to specify.

### D10. The Prism-level protocol module `/prism/composer` (autonomous)

`standard-library` makes externals `private` and library-only, and its executable fixtures cannot add library test modules (its task 7.5), so a Prism program can reach the runtime only through public functions of a bundled module. This change adds `crates/prism-library/prism/composer.prism` (`package prism`, module `/prism/composer`). It holds one `private external fun prism_compose_*` per entry point that hand-written code needs, with the parameters that keep a function value declared `escaping`, and the public wrappers of the spec. Composers, compositions and scopes are plain `Int` words, which is exactly the machine form generated code will pass (D3). A hand-written composable such as `fun Label(k: Int, n: Int)` is therefore what the transform will generate from `compose fun Label(n: Int)`, minus change words, and the fixtures exercise the same entry points the transform will call.

The module is outside the prelude, so its names never reach a program that does not import it. It is not `/prism/compose/runtime`: that module is `compose-transform`'s prelude module for application code (`standard-library` D14), and its `Composition` is an owned type, where the protocol's compositions are handles. After the transform, the module stays as the declaration of the externals its generated code calls, and as a hand-testing surface. Whether its wrappers stay public is `compose-transform`'s call. It is safe for any caller, since every handle is validated (D3) and every misuse is a runtime error (D12).

Compile-time cost: none for programs that do not import it. An importer parses and checks one small module, cached like any library module.

Rejected:
- Allowing `// library:` modules in executable fixtures: it needs the fixture library on the `prism` command line, which `standard-library` D13 rejected so that injection never reaches users.
- Testing only from Rust, as the ROADMAP item first said: the protocol would be pinned by no `.prism` source until the transform exists.
- Creating `/prism/compose/runtime` here: it would mix the low-level protocol with the application API and make `standard-library` D14's prelude module two things.
- Typed wrapper structs (`struct Composer(val id: Int)`): structs are passed by address, which is not the machine form of the transform's composer word.

### D11. Composition lifecycle entry points; the owned `Composition` is `compose-transform`'s (autonomous)

`prism_compose_start(content) -> Int` creates a composition with the node tree applier and runs its first pass. `prism_compose_set_content`, `prism_compose_recompose`, `prism_compose_dispose`, `prism_compose_root_scope` and `prism_compose_dump` complete the lifecycle. At the Prism level a composition is a `Copy` handle disposed explicitly. §5 D11's owned `Composition`, whose drop disposes it, needs a type whose drop glue calls an external. No Prism declaration can express that yet, so it is left to `compose-transform`, which adds it with `composition { }`. Until then, the allocation check reports a composition that was never disposed (D13).

Rejected: an intrinsic owned type here, which would pull a compiler feature into a runtime change.

### D12. Misuse is one runtime error kind with a detail, without a location (autonomous)

`PanicKind::CompositionMisuse` (the next free code) formats as `composition misuse: <detail>`, raised with `panic_without_location`. An entry point has no caller location, as `standard-library` D10 notes for externals. The details are the spec's seven, worded for the hand-written code that can trigger them. Invalidating a stale scope is not an error: a state's readers may legitimately include a scope that has since left the composition, and `state-handles` invalidates readers without filtering them.

Rejected:
- A kind per misuse: seven codes for one class of bug.
- Undefined behavior for misuse, as C APIs allow: it breaks the memory-safety invariant.
- Locations through an extra argument: a word per call for an error that the transform never produces.

### D13. The allocation check counts undisposed compositions (autonomous)

The leak check of `--check-allocations` would miss a composition that holds no allocation of the program, such as one whose restart functions have inline environments. `prism-runtime` gains `set_finish_report(fn() -> Option<String>)`, which `finish_tracking` calls before the leak report. `Program::run` installs `prism_compose::live_compositions_report`, which reads the program thread's live count. That is the thread `finish` runs on, so the count needs no synchronization. The report line comes before the allocation line, and either one makes the status 102. Counting live groups, cells and nodes stays with `reactive-app-verification` (§5 D24).

Rejected: waiting for `reactive-app-verification`, which would leave this change's disposal untested by the check.

### D14. A line-per-group text dump (autonomous)

The dump format of the spec is easy to read in a fixture's `stdout.snap` and stable: it holds keys, kinds, tags, values and node numbers, never addresses or generations. Typed slots print their record's name, and word slots their tag and value. The applier dumps its own tree, so the UI applier can print its nodes' properties later. `ui-toolkit`'s `--events` `dump` prints the laid-out tree, which is a different dump.

### D15. Hot reload interplay (implements the runtime side of `compose-architecture` D23)

- Keys decide only which slots a group may reuse. Slot types (D6) and node kinds (D9) are checked on every read and start, so a key that matches the wrong group after an edit costs that state, never memory safety.
- The runtime holds no code address (spec "The Runtime Runs Program Code Only Through Function Values"). Restart functions reach reloaded code through their descriptors' slots, and a closure whose captures changed has a new slot (`hot-reload` D5), so an old restart function keeps running code for its own layout until its scope ends again and stores the new one.
- `Composition::invalidate_where(predicate)` invalidates every restart scope whose restart function's descriptor names a slot the predicate accepts. `compose-hot-reload` uses it with the slots of the reloaded units, together with a forced pass flag that makes `skipping` false.
- Under `prism run --watch` with the default reload policy, a reload may be applied in the middle of a pass, at a function entry. That is memory-safe by the two points above. `compose-hot-reload` moves reloads between frames (`hot-reload` D4).

### D16. Costs, and how they are measured

| Operation | Cost |
|---|---|
| start group, end group, slot read, write or compare, skip, invalidate | O(1) amortized (D5, D6) |
| first mismatch under a parent in a pass | O(remaining old children), once per parent per pass |
| pass | O(bodies run + groups and slots visited) + O(k log k · d) ordering + O(released) |
| dispose | O(groups + slots + nodes) |
| memory | one group record (about 100 bytes) per group, 32 bytes per slot plus its value beyond 16 bytes, one node tree entry per node |

The runtime's own buffers (the composer stack, the scratch children, the key map, the release list and the ordering buffer) are kept and reused by each composition, so a pass that creates nothing allocates nothing.

- `crates/prism-compose/tests/allocations.rs` installs a counting `#[global_allocator]` and checks the spec's scenario. Its restart functions are function values with static descriptors, slots and inline environments, so that only the runtime's allocations are counted.
- `crates/prism-compose/tests/runtime_timing.rs` is an ignored benchmark in the style of `compile_timing.rs`. It reports the first composition, steady-state passes and the recomposition of one leaf scope for 1,000, 10,000 and 100,000 groups, and a keyed reversal of 1,000 movable groups. Under `PRISM_ASSERT_TIMING=1` it asserts the spec's growth bounds. It also records reference medians in this file at apply time, as `codegen-core` did for its benchmark.

Reference medians at apply time (2026-10-18, release build, Apple silicon), from `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-compose --test runtime_timing -- --ignored --nocapture`:

| groups | first composition, per group | steady-state pass (root recomposed, children skipping) | one invalid leaf scope |
|---|---|---|---|
| 1,000 | 54 ns | 125 ns | 83 ns |
| 10,000 | 54 ns | 125 ns | 83 ns |
| 100,000 | 57 ns | 125 ns | 83 ns |

The leaf pass grows x1.00 and the first composition per group x1.06 from 1,000 to 100,000 groups. A keyed reversal of 1,000 movable groups takes 79 µs. `allocations.rs` records no allocation and no free in its last 98 passes.

Startup after the change (task 9.2, `cargo test --release -p prism-cli --test run_timing -- --ignored --nocapture`, the same machine): `prism run` of `examples/run/main.prism` reaches `main` in a median of 8.8 ms cold (p90 8.9 ms) and 6.3 ms warm (p90 8.5 ms), under the 15 ms target. Registering the compose table adds 23 symbols to the JIT's table, and the finish report is one function pointer stored per run.

Re-measured at archive time on an idle machine (2026-10-19, release build, two runs of the benchmark): 8.8 ms cold (p90 8.8 to 9.0 ms) and 1.74 to 1.79 ms warm (p90 1.9 to 2.0 ms), against about 7.8 ms and 1.5 ms before this change; a one-line program reaches `main` in 2.07 ms cold and 0.70 ms warm (1.8 ms and 0.73 ms before). The 6.3 ms warm median above came from a busy machine: a first run of the benchmark during another build gave 13.1 ms cold and 8.0 ms warm, with the stages' sum unchanged. Nothing of the compose runtime runs for a program that does not use it: its per-thread state is a `const` thread local touched only by an entry point, or by the finish report under `--check-allocations`, and the 23 symbols are inserted into the JIT's hash map once per run, so no fix was needed.

Compile-time cost of the whole change: none. No stage, analysis or pass is added. The backend's entry-point lookup covers one more table, and `/prism/composer` is parsed only when imported.

### D17. A new `compose` capability, and how the change is tested

The runtime's behavior is new and is a part of compose, which `compose-transform` and `state-handles` will extend with the language side, so it is a new `compose` capability rather than part of `execution`. `execution` gains the misuse exception in "Runtime Errors End the Program" and the composition report, and `standard-library` gains `/prism/composer` in its module list. There is no new error code, since misuse is a runtime error.

Behavior a `.prism` program can observe is pinned by executable fixtures under `crates/prism-cli/tests/fixtures/compose/` and `.../execution/`, all under `--check-allocations`. Behavior it cannot observe is pinned by Rust tests in `prism-compose` with recording drop entries and a recording applier: release order, slot addresses outliving removals, and typed slot records. Misuse that Prism code cannot reach (`no slot to read`) is pinned by a Rust test in a child process.

### D18. Revisions made while implementing (autonomous)

Implementing and reviewing the design showed five places where it was wrong or underspecified. Each is now what the code does:

- **Typed values live in blocks of their own (revises D6).** D6 kept values of at most 16 bytes inline in the slot. A group's slot vector moves its elements when it grows, and a forgotten slot moves onto the release list, so an inline value's address would change during the pass, against "Removals Are Deferred to the End of the Pass". Every typed value is now in a block of the record's size and alignment, allocated when the slot is created and freed at its release. A write over a slot of the same type whose values need no drop overwrites the value in place (no address is given out twice in a pass, since the cursor moves past each slot it reads), so `changed` of a plain value allocates nothing. Word slots stay inline: their values are only read by value. Run-time cost: one allocation per new typed slot, none in steady state; the size-class allocator follow-up of the archive list covers small blocks. Rejected: inline storage with addresses valid only until the next slot operation (it breaks generated code that keeps a remembered value's address), and a per-thread slab of 16-byte cells now (more code before a measurement asks for it).
- **A context stays in order while matches only skip unvisited siblings (refines D9).** With removals deferred, a node after a removed one has the removed node as its previous sibling until the pass ends, so placing it after the last placed node would move it, although nothing visible changes. Each node context now records whether it is *in order*: every node placed so far is where it belongs, and every other node of the context belongs to an old group not visited yet, which will be removed or matched later. While it is, a reused node is not placed again. A match at an old position below the highest one matched so far in its parent, a new node of a node group (after a kind change) or a skipped child behind a placed one makes the context out of order, and from then on every node group places its node, and skipping a group places the top-level nodes of its subtree again. Run-time cost: one compare per match; the subtree walk only after a reorder. A randomized Rust test checks after every pass of keyed edits, skips and restarts that each node's children are the top-level nodes of its group's subtree.
- **The applier numbers nodes for dumps (refines D9 and D14).** `Applier` gains `number(node) -> u64`, which the composition dump prints as `#<node>`, since handles hold generations that dumps must not show. A kind-changed node group's old node is released at the group's end, after the nodes of its removed children, so nodes are released children first.
- **Two fixes from the review (refine D8).** Only the top group of a removed subtree is marked removed until the end of the pass, so a pass now walks up from each scope it opens and drops one below a removed group (O(depth), as its ordering path already costs). And a parent that skips marks its kept children visited, so a scope below it that recomposes alone in the same pass starts its window with its group unvisited again. Before, the window failed to match the group, created a new one and spliced the old one out of the tree without releasing it, which leaked its slots, restart function and node. A model-checked randomized Rust test (`random_tree_edits_match_the_model`) now edits whole trees of node, replaceable, movable and restart groups at the front, middle and end, moves, wraps and removes subtrees holding invalid scopes, and checks the node tree, one live value per node group, no removed scope recomposing, and that disposing releases every value, restart function and node once.
- **An eighth misuse detail (refines D12).** A restart function that reads or writes a slot before starting its group has no group of its own open: its frame is its scope's parent window. That is the misuse `slot outside a group`, rather than reading or writing the parent's slots.

Smaller differences from the plan:
- The pass's buffers (composer frames, scratch children, key maps, ordering buffers, release list and walk) belong to the thread rather than to each composition, since a thread runs one pass at a time (D4). Their capacity is shared by every composition of the thread, and a composition's invalidation queue is swapped with a spare buffer when a pass orders it, so neither reallocates.
- `set_finish_report` takes `Option<fn() -> Option<String>>`, so a test can remove the report.
- The Rust API also has `Composition::recompose_forced` (D15's forced pass, which `compose-hot-reload` D6 may drop), `Composition::with_applier`, `thread::active_scope` for `state-handles`, and `active_composer` for tests.
- The `E6005` test of the compose table is in `lowering_tests.rs` over a library test module, and the prelude test of `/prism/composer` is in `cache_tests.rs`, beside "Unreached modules are not parsed", whose helpers it shares.
- The scenario "Disposing a composition frees what it holds" now clones the name in its restart function (`Greeting(c, name.clone())`): a closure never gives away what it captures (`E4017`), so the proposal's source did not compile.

## Risks / Trade-offs

- **[The protocol is public in `/prism/composer`]** Programs may call it and depend on it. → It is outside the prelude and documented as the lowering protocol, every misuse is a runtime error, and Prism is unreleased, so `compose-transform` may make it private later at no cost.
- **[Thread-local state]** Compositions cannot move between threads. → Prism has one program thread (§5 D16). A handle used on another thread finds nothing or a stale entry, which is an error, not undefined behavior.
- **[Hand-written fixtures diverge from generated code]** → Both call the same entry points with the same words (D3, D10), and `compose-transform`'s fixtures re-pin the behavior through real composables.
- **[Restart functions allocate when they capture more than a word]** → The runtime itself allocates nothing in steady state (D16), and the transform can keep parameters in slots (D8).
- **[Re-entrancy from program code]** Program code runs inside passes and releases. → The runtime holds no Rust reference across calls into it (D4), and the only re-entrant path, a drop disposing another composition, is allowed and tested.
- **[A reload in the middle of a pass]** → Memory-safe (D15). Its state effects are `compose-hot-reload`'s to avoid.

## Migration Plan

No existing program changes meaning: the change adds a crate, a library module, a panic kind, a report line under `--check-allocations` for programs that use `/prism/composer`, and entry points. `prism-codegen`'s lookup of runtime functions changes from one table to two, which no output shows.

## Open Questions

- Whether `compose-transform` keeps `/prism/composer`'s wrappers public, or keeps only its externals. This does not change this change's specs.
