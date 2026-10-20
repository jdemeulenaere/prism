# Design: drop-and-clone-hooks

## Context

See `proposal.md` for motivation. This change is part **L2** of agreed item **A4** of the post–Milestone 2 review (`docs/milestone-2-review-notes.md`, "Agreed changes": "Language additions so the rest can be Prism: ... (L2) custom drop and clone hooks"), item 8 of "After Milestone 2: the review round" in `ROADMAP.md` ("custom drop and clone for owning library types"). The user decided in the same item that there is no `unsafe` layer for now: `List`, `String` and `Heap` stay Rust-backed. So hooks serve types that wrap **runtime handles** (`Int` words from `external` functions: compositions, nodes, state cells, and later files or windows), not raw memory: a hook releases or copies a resource the runtime owns, and never frees memory itself. `library-in-prism` (item 10) will rewrite `Composition`, state and node handles in Prism over externals and relies on this change. The user is away: every decision the user might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`; those taken in the review are **(user direction 2026-10-20)**. Compile speed is a primary criterion (AGENTS.md §5), so each decision states its compile-time cost.

**Baseline.** Proposed on top of `f473e0e` ("Propose operator-conventions"), with `owned-state` (item 4), `strong-skipping` (item 5), `explicit-library` (item 6) and `operator-conventions` (item 7) proposed and not archived, all landing before this change. This packet is written for the state after them:
- `operator-conventions` gives the `operator` modifier, the `E4035` `InvalidOperatorDeclaration` code and the rule that `equals` belongs to its type (its D5, D6, D15); hooks reuse all three.
- `owned-state` gives `remember` of owned values (its D6), retired cells freed after the next pass (its D7, review rule R2) and the release order R1 (its D8); the `compose` delta's scenarios rely on them.
- `explicit-library` removes the prelude; scenarios omit library imports (its D12), except `/prism/composer` and `/prism/system`, which were never in the prelude.
- `path-captures` (archived) asked this change to state that a closure keeps exactly the values it captures by value (its D9); `execution` "A Closure Keeps Only What It Captures by Value" does.

The blocks this packet MODIFIES (`ownership` "Marker Declarations Are Checked Structurally"; `execution` "Values Are Freed Where Their Owner's Scope Ends", "Cloning Values That Own Storage", "Instances Drop and Clone Type-Parameter Values"; `code-generation` "Drops Run Drop Glue", "Drop Glue Drops Each Owned Part", "Clone Glue Builds Independent Values"; `hot-reload` "Slots Record the Contract of Their Code") are modified by no other open packet. The first task re-checks this.

**Baseline update (from `operator-conventions`' archive, 2026-10-20).** `operator-conventions` is archived (`openspec/changes/archive/2026-10-20-operator-conventions/`), with its rules in the new main `operators` spec. Its declaration checks live in `crates/prism-typechecker/src/checker/operators.rs`, a submodule of the checker (`check_operator_declaration`, `check_operator_result`, `check_equality_operator`, `check_builtin_operator_clash`), not in `src/operators.rs` as `proposal.md` and tasks 1.1 and 2.3 say. `E4035` is `InvalidOperatorDeclaration` and `E4036` is free. A type's `equals` flag is recorded by `TypeTable::insert_equality_operator` (`crates/prism-typed-tree/src/types.rs`), from the file's own declaration and, in an importer, from both the exported struct and enum symbols (`ExportedSymbol::{Struct, Enum}::equality`) and the reachable definitions (`AbiTypeDefinition::equality`), a review fix: hook flags beside it need both sources too. The `mut` accessors are kept in the extension tables as `get#mut` and `next#mut` (`operators::extension_name`, `is_mutable_accessor`). A compound assignment through a consuming operator on an element or field is `E4017` (`ConsumingCompoundTarget`); taking the element out and putting the result back is a ROADMAP follow-up that owning library types may want.

**Baseline update (reconciliation before applying, 2026-10-20, on `31eccd6`).** `owned-state`, `strong-skipping`, `explicit-library` and `operator-conventions` are all archived. Every MODIFIED block (`ownership` "Marker Declarations Are Checked Structurally"; `execution` "Values Are Freed Where Their Owner's Scope Ends", "Cloning Values That Own Storage", "Instances Drop and Clone Type-Parameter Values"; `code-generation` "Drops Run Drop Glue", "Drop Glue Drops Each Owned Part", "Clone Glue Builds Independent Values"; `hot-reload` "Slots Record the Contract of Their Code") still carries every scenario of its main requirement with the main text unchanged, and no ADDED requirement exists in the main specs; no other open packet modifies them. `E4035` is `InvalidOperatorDeclaration` and `E4036` is free. The names the tasks use exist as written, with these precisions: the operator checks are in `crates/prism-typechecker/src/checker/operators.rs` (now corrected in tasks 1.1 and 2.3 and in the proposal); `TypeTable`'s markers are the private map `markers` read through `marker`/`marker_entry`, beside which the hook sets go as `equality_operators` does (`restricted_to_ids`, `extend_from` and the record codec of `crates/prism-mir/src/codec.rs` carry them); `owns_storage` is also read by `LayoutTable::environment_layout` (`crates/prism-codegen/src/layout.rs`), which must use the drop predicate too so that a hooked capture never sits in an inline environment without a drop entry; and `check_take_apart_source` (`checker/patterns.rs`) is called from both `when` (`checker.rs`) and destructuring declarations, where the hook check goes. Since explicit-library, fixtures import what they use (`import /prism/core`, `/prism/collections`, `/prism/compose/runtime`). `openspec validate drop-and-clone-hooks --strict` is clean with no INFO lines.

Current state (main specs and code):

- **Drops are decided by MIR, run by glue.** `needs_drop` (`crates/prism-mir/src/types.rs`) gives every non-`Copy` type a drop where its scope ends; drop elaboration removes drops of moved values and guards conditional ones with drop flags (`mir`, "Drops Are Elaborated After Borrow Checking"). The code generator runs each kept drop as the type's drop glue (`crates/prism-codegen/src/glue.rs`), and a type for which `owns_storage` is false has no glue, so its drops run no code. `Composition` is special-cased there (its glue disposes it, `compose-transform` D16).
- **Glue runs no user code** (`code-generation` rationale; `execution` "Cloning Values That Own Storage": "MUST run no user code"). It is local to its unit, called directly, and checks the stack on entry, which is a reload point (`hot-reload`).
- **Clone is field-wise**: the built-in `clone()` runs clone glue; `: Clone` requires every field to be `Clone` (`E4016`). On a type that is not `Clone`, `x.clone()` calls a visible extension named `clone`.
- **Taking a value apart moves it whole** (`payload-patterns` D6, D7): no partial moves.
- **Top-level values are never dropped**; under `--check-allocations`, `finish_tracking` (`crates/prism-runtime/src/allocation.rs`) runs their registered drop glue in reverse order of initialization before reporting, "running no user code" (`execution` D13).
- **Releases run after the pass.** `run_pass` (`crates/prism-compose/src/composition.rs`) calls `end_pass` (which clears the active pass) before `process_releases`, so drop entries of released slots and functions already run outside the pass.
- **Instance planning reads summaries only** (`plan_instances`, `crates/prism-codegen/src/instances.rs`; `FileSummary`, `crates/prism-mir/src/summary.rs`): roots from non-generic bodies, generic calls substituted per instance; it has no type definitions.
- **Type fingerprints** (`TypeFingerprints`, `crates/prism-codegen/src/contract.rs`) cover names, markers, type arguments, fields and payloads of every type reached.

## Goals / Non-Goals

**Goals:**
- A struct or enum written in Prism releases the runtime resource it owns when dropped, and acquires its own when cloned, wherever the value lives (locals, temporaries, fields, payloads, nullables, lists, boxes, closures, generic instances, remembered slots, state cells).
- No new ownership rules for moves: the existing drop elaboration and drop flags decide whether a hook runs.
- Compilation avoidance and hot reload unchanged in spirit: a hook's body edit recompiles no importer and is swapped by a reload.
- Constant compile-time cost per declaration and use; hook instances planned from summaries in linear time.

**Non-Goals:**
- Rewriting library types with hooks (`Composition`, state, nodes): `library-in-prism`. `Composition` keeps its compiler drop here.
- `unsafe`, raw memory or custom allocators (user direction 2026-10-20: none for now).
- A way to skip a hook (`forget`, `ManuallyDrop`), a library `drop(x)` function, `close()`/`use { }`.
- Hooks on intrinsic structs and built-in types; generic `equals` (`operator-conventions` D6), although D11's planning would serve it.
- Unwinding: a runtime error still ends the process without drops.

## Decisions

### D1. Hooks are operator conventions of their type: `operator mut fun T.drop()` and `operator fun T.clone(): T` (autonomous; the hooks themselves are the user's L2)

Kotlin has no destructors; its nearest feature, `AutoCloseable.close()` with `use { }`, is an explicitly called method, which single ownership makes unnecessary: Prism knows statically where each value dies. The other languages with deterministic destruction attach it to the type: Rust's `impl Drop` (`fn drop(&mut self)`), Swift's `deinit { }`, C++'s `~T()`. Prism has no traits and no members inside struct bodies (methods are extensions), and `operator-conventions` just introduced the form for "a function that the language calls for you, on a conventional name": `operator fun T.equals(...)`, which belongs to its type (its D6). Drop and clone hooks follow that form exactly: `operator mut fun Session.drop()` and `operator fun Ticket.clone(): Ticket`. The names are the language's own words: values are *dropped* and *cloned* in every spec, and `x.clone()` is already Prism's built-in.

**Compile-time cost.** Nothing in the parser (the modifier exists); a name match per `operator` declaration.

Rejected:
- **A `deinit { }` block in the struct** (Swift): a new grammar form, code inside a struct body, which Prism has nowhere else, and a second form for clone.
- **A `Drop` trait or interface** (Rust): no traits exist (`operator-conventions` D1 rejected them for the same reason).
- **`close()`/`dispose()`/`release()`**: in Kotlin and Compose those are called explicitly; reusing one for an implicit hook would mislead readers.
- **C++'s `~T()`**: not Kotlin-like.

### D2. The drop hook borrows its value mutably and runs before the value's parts (user's brief; receiver mode autonomous)

The hook receives `&mut this`, as Rust's `fn drop(&mut self)`: it can read every field, write `var` fields (reset a handle, replace an inner value), and call `mut` functions, but cannot move anything out (moving out of a borrow is `E4017`), so the value stays whole for the glue that follows. After the hook returns, the glue drops the parts as today: a struct's fields in declaration order, an enum's current payloads in order (Rust's order; Swift's `deinit` and C++ destructors also run before members are destroyed). A field with its own hook runs it in turn when its part is dropped, so `Outer`'s hook sees an intact `Inner`.

**Compile-time cost.** None beyond the declaration check.

Rejected:
- **A `consuming` receiver**: the hook would own `this` and drop it again at its end (infinite recursion) unless a special rule exempted it, and it could take `this` apart or move it elsewhere, resurrecting it.
- **A shared receiver**: a hook could not reset a field it releases, which a hook that hands a resource back needs.
- **Parts first**: the hook would see dropped fields.

### D3. Hooks belong to their type: same file, at most one each, no visibility modifier, the type's own type parameters, never called by name (autonomous)

A hook is declared in the file declaring its struct or enum, at most once per kind (a second is `E4010`, as for any extension), without a visibility modifier, with no parameter (the clone hook returns `T`, the drop hook `Unit`), and with exactly the type's type parameters, in order: `operator mut fun <T> Owned<T>.drop()`. Drop hook parameters are unbounded, since a drop must exist for every instantiation; a clone hook may bound a marker parameter by `Clone`, which every `Clone` instantiation satisfies (`ownership`, "Markers of Generic Types Are Conditional"). Built-in types and intrinsic structs cannot have hooks: they are declared in no file, or their drop is the compiler's. Every other shape is `E4035` (the code `operator-conventions` gives malformed operator declarations), with a message naming the rule.

A drop hook is never called by name: `s.drop()` would run it and leave `s` to be dropped again. Such a call, through any reference, box or `?.`, `this.drop()` in the hook included, is the new **`E4036` `DropHookCalled`**, whose help says that a value is dropped where its owner's scope ends and that moving it into a function or a block drops it earlier. For resolution, a drop hook is a member of its type, so an extension named `drop` on that type is `E4010` (shadowed by a member). A clone hook is reached through the built-in `clone()` member, whose glue calls it (D6), so `x.clone()` means the same with or without it.

Visibility differs from `operator-conventions`' `equals`, which takes its type's visibility because `a.equals(b)` is callable by name: a hook is never named, so a modifier would only export an uncallable function (and `public` would force the `: Unit` that public declarations spell out). A hook reaches importers as a flag on its type's ABI record (D10), and their glue calls it by its derived symbol (D9), whatever its visibility.

**Compile-time cost.** Phase 1: a hash lookup of the receiver's name among the file's types per `operator` declaration named `drop` or `clone`. Type checking: a fixed shape check per hook; one flag test when a method call resolves to `drop`.

Rejected:
- **Hooks in any file**: an importer could not know from the type's file alone whether its drop calls a hook (the single-file compilation invariant), and two files could give one type two hooks.
- **The type's visibility, as `equals`**: see above.
- **Allowing explicit calls**: a double release, or a rule that a called hook marks the value moved, which is a `consuming fun`'s job.
- **Specialized hooks** (`operator mut fun Owned<Int>.drop()`): which drop runs would depend on type arguments, and instances would need a lookup by argument.

### D4. A drop hook excludes `Copy`, and a `Clone` type with a drop hook needs a clone hook (autonomous)

A `: Copy` value is copied bitwise wherever it is read, so a hook would run once per copy: a drop hook on a `: Copy` type is `E4035`, naming the marker (Rust's `E0184`). A `Copy` type never reaches a hook, since its fields are all `Copy`. A `: Clone` type with a drop hook and no clone hook would be cloned field by field, duplicating the handle its drop releases: two values would release one resource. That is `E4035` at the drop hook, with help suggesting the clone hook. Rust allows `#[derive(Clone)]` beside `Drop` and leaves the bug to the author; Prism's types that need a drop hook are exactly those whose field-wise copy is wrong, and the check costs one lookup.

**Compile-time cost.** One flag test per hook declaration.

Rejected:
- **Rust's freedom**: a likely double release, found only at run time.
- **Making such a type not `Clone`**: the marker would say `Clone` while `x.clone()` fails.

### D5. A value with a drop hook is never taken apart (autonomous)

Taking a value apart moves its parts into bindings and drops nothing else of it (`payload-patterns` D6), so a pattern or destructuring declaration that moves a part out of a value whose type has a drop hook would skip its hook. That is `E4017` (the code of every take-apart error), saying the type has a drop hook, with help suggesting a borrow (`when (&s)`), as Rust's `E0509`. Binding `Copy` parts, wildcards and borrowing patterns do not take the value apart and stay accepted; a holder without a hook can still be taken apart, its hooked field moving into a binding with its hook intact. Inside the type's own `consuming fun`, `val (...) = this` is rejected too. A consuming function that wants to end a value without its hook's effect sets a `var` field the hook checks (`id = -1`), and the hook does nothing: no `forget` is needed for that.

**Compile-time cost.** One flag lookup per pattern or destructuring declaration that moves a part, in the existing take-apart check (`checker/patterns.rs`).

Rejected:
- **Allowing it and skipping the hook**: a silent resource leak.
- **Running the hook before taking the value apart**: the hook could release what the bindings then hold.
- **`forget`/`ManuallyDrop` now**: no library need yet; a follow-up if `library-in-prism` finds one.

### D6. The clone hook replaces field-wise cloning everywhere; `: Clone` stays required and the field check is waived (autonomous)

`operator fun Ticket.clone(): Ticket` is called by the clone glue of `Ticket` (D9), so it serves `x.clone()`, a cloned field or payload, nullable payloads, list and box clones, and `clone()` of a type parameter in an instance. The marker `: Clone` is still required (a clone hook on a type without it is `E4035`, with help suggesting the marker; on a `: Copy` type it is `E4035` too): the marker is what signatures, bounds and the ABI read (`is_clone` stays a marker lookup), and it keeps "which types clone" visible at the declaration. But the structural check of `: Clone` (`E4016` for a field that is not `Clone`) is waived when the type has a clone hook, since the hook builds the copy: a `Task` holding a function value can be `Clone` by building a new closure, as a manual `impl Clone` can in Rust. Conditional markers stay as they are (`Owned<X>` is `Clone` when `X` is).

A clone hook's result need not be `==` to its source (`Ticket(id + 100)`): equality is the type's own (`operator-conventions` D6), and nothing in the compiler or the compose runtime clones implicitly and compares (kept values are `Copy`; keepable closures own only `Copy` captures and weak handles). A clone hook that calls `this.clone()` recurses until the stack check raises `stack overflow`, as any recursion does.

**Compile-time cost.** One flag lookup per `: Clone` declaration (in `check_marker_declarations`) and per clone hook.

Rejected:
- **A clone hook making a type `Clone` without the marker**: two ways to declare one property.
- **Keeping the field check**: a type holding a function value or a non-`Clone` handle wrapper could never be cloned.

### D7. Moves, drop flags and borrows need nothing new (autonomous)

Whether a hook runs is whether a drop runs: MIR already drops every non-`Copy` value where its scope ends and on assignment, and drop elaboration removes the drops of moved values and guards conditional ones with drop flags. A type with a hook is non-`Copy` (D4), so its values already have those drops; only glue changes (D9). A moved-from value runs no hook; a value moved on some paths runs it on the others; assigning over a moved variable runs none.

No borrow can be observed dangling from a hook: a hook sees only its own value through `this`, and structs, enums and the values a hook can reach hold no borrows ("Struct Field Borrow Prohibition"), so Rust's drop check, which exists because a `Drop` type may hold references with lifetimes, has nothing to check here. A drop at a scope end is already an access of the whole value for the borrow checker.

**Compile-time cost.** None.

### D8. Hooks run wherever drop glue runs; a closure keeps only what it captures by value (autonomous)

Because hooks are called from glue, every place glue already serves runs them with no special case: struct fields and enum payloads of hook-less holders, nullable payloads, list elements (index order on drop, the old element on an indexed write), box contents (not on `unbox()`, which moves them), owned closure captures (environment drop functions), type-parameter values in instances, slot records of remembered values, state cells' values, and top-level values under the check (D13).

As `path-captures` D9 required, a closure keeps and drops only what it captures by value: when an owning closure copies a `Copy` path such as `t.id`, the binding `t` stays owned by its scope and runs its hook there, possibly while the closure lives on. Copying an `Int` out of a value whose hook releases what the `Int` names gives a stale handle, exactly as `val id = t.id` does; it is a logic error the runtime's handle checks report, never a memory error (no `unsafe`).

**Compile-time cost.** None.

Rejected:
- **Truncating copied paths at a value with a drop hook** (the closure owns all of `t`, which is what a Kotlin closure keeping `t` alive would do): `{ t.id }` over a reference would become `E5007`, rejecting code that is no less safe than `val id = t.id`.

### D9. Glue calls hooks through the function table (autonomous)

Drop glue of a type with a drop hook first calls the hook, then drops the parts; clone glue of a type with a clone hook calls it with the source's address and writes the result to the destination, with no field-wise clone (`code-generation`). The call goes through the slot of the hook's symbol (`<module>::<Type>.drop`, or the instance symbol `<module>::<Type>.drop<Args>` of a generic type), as every call of a Prism function does. So:
- glue stays local to its unit and its unit still depends only on its own MIR and the type's ABI flags, never on the hook's body: editing a hook recompiles no importer (deterministic hermetic builds; `compilation-and-abi` scenario);
- hot reload repoints the hook's slot like any function's (D12);
- the hook is a reload point at its own entry, and the glue already checks the stack on entry.

`owns_storage` becomes "needs glue": it also holds when the type, or a part through fields, payloads and nullables, has a drop hook (for drop glue) or a clone hook (for clone glue). The `Composition` special case stays until `library-in-prism` replaces it by a hook. Glue now runs user code, but only through hooks, which the `code-generation` rationale and "Glue Calls Hooks Through the Function Table" state.

**Compile-time cost.** One flag test per type when deciding whether glue is needed and per part when generating it, both already linear in the parts. **Run-time cost.** One indirect call per hooked value dropped or cloned; types without hooks are unchanged.

Rejected:
- **Calling the hook directly by symbol**: hot reload repoints slots, not direct calls, so old glue would keep running an old hook body.
- **Lowering hook calls in MIR** (a `Drop` of a hooked place becomes a call plus field drops): MIR would expand every nested type at every drop site, and lists, boxes and environments, which are dropped by glue, would still need glue to call hooks.

### D10. Hook flags are part of a type's definition in the ABI (autonomous)

`ExportedSymbol::{Struct, Enum}` and `AbiTypeDefinition` gain `drop_hook` and `clone_hook`, set in Phase 1 by a syntactic match of the file's own `operator mut fun T.drop` and `operator fun T.clone` declarations (as `operator-conventions` D15 sets `equality`), covered by the ABI hash and by the structural hash of definitions that keys instances. An importer's glue for a type holding an imported `Session` thus knows to call `lib/net::Session.drop`; adding or removing a hook re-checks importers; editing a hook's body changes no hash (it is not in the ABI). `TypeTable` holds the flags beside markers, filled from local declarations and from imported and reachable definitions.

**Compile-time cost.** Two flags per struct and enum in Phase 1; a hash lookup per hook declaration.

### D11. Hook instances of generic types are planned from summaries (autonomous)

`library-in-prism` will write `MutableState<T>` over runtime cells, so generic hooks are needed now, not later; a generic hook is an ordinary generic record whose instances the planner must find. The planner reads only summaries and holds no type definitions (`instance-cache` D7), so the walk over definitions happens in Phase 2, where each file has them all:

- A file's summary requests, as roots, the drop hook instance of every generic type with a drop hook reached by the types its non-generic bodies drop or clone and by the type arguments of its generic calls, through fields, payloads, nullables, boxes, lists and closure environments; and the clone hook instance of each such type that is `Clone` at its arguments.
- A generic's summary requests the same from its own body's types, with its type parameters left as they are (`Owned.drop<T>` for a dropped `Owned<T>`); the planner substitutes them as it substitutes calls.

This covers every hooked type an instance can meet: a type an instance drops is built from types its record names (walked in the record's Phase 2) and its type arguments (walked where they were concrete, at a root, or substituted from a record's request). It over-approximates slightly (a type argument that an instance only borrows still requests its hook), which costs one instance compiled, never correctness. Hook requests join the generics' calls in the cross-file polymorphic recursion check, so a hook whose instances request ever larger hook instances is `E6004`, wherever its file. Non-generic hooks need no request: they are ordinary functions with one symbol.

**Compile-time cost.** In Phase 2, one walk per distinct type a file drops, clones or passes as a type argument, memoized per file; planning stays linear in requests (a hash lookup each). Shape `generic-hook-instances` (D17).

Rejected:
- **Deferring generic hooks** (as `operator-conventions` D6 deferred generic `equals`): `library-in-prism`'s state needs them, and a second change would redo this planning.
- **Walking types in the planner**: it would need every file's definitions, against "instances found from summaries alone".
- **Requesting hook instances while generating glue**: code generation would discover instances after planning, breaking instance cache keys and `--explain-cache`.

### D12. Type fingerprints cover hooks; hook bodies reload like functions (autonomous)

A reload that edits a hook's body keeps its contract (`(&mut T) -> Unit` with an unchanged fingerprint of `T`), so its slot is repointed and every later drop runs the new body, also for values made before the reload, through old glue or new. This is sound: the value's layout and hook flags are those the new body was compiled for.

A reload that adds or removes a hook changes whether a value must be released. If old values could be dropped by glue of the other kind, a value made without a hook could be "released" by a hook that never acquired anything, or a held resource could be leaked by glue without the hook. So, unlike `equality` (`operator-conventions` D15, where type fingerprints stay representation-only), the fingerprint covers the presence of each hook: every contract mentioning the type changes, code compiled before keeps calling old slots with old glue, kept slots and top-level values of the type are reset (dropped by the glue of the code that made them) and made again by new code. That is what "A Slot Is Repointed Only to Code With the Same Contract" already does for a changed field.

**Compile-time cost.** Two bits per definition hashed.

Rejected:
- **Representation-only fingerprints**: the mismatches above.
- **Refusing reloads that add or remove hooks**: "no reload is refused" (`hot-reload`).

### D13. Top-level values: never dropped by the program; dropped with their hooks by the allocation check (autonomous)

A top-level value lives until the process ends and is never dropped while the program runs (`execution` D12, like a Rust `static`, avoiding C++'s static destruction order). Under `--check-allocations`, the check already drops initialized top-level values in reverse order of initialization before reporting, and its drop glue now calls hooks. Running them is what makes the check meaningful for resources: a top-level `Session` would otherwise report its composition as live, and once `library-in-prism` writes `Composition` with a hook, a top-level composition, which today's glue disposes at the check, would regress. Two edge cases get rules:
- a hook may read a top-level value the check already dropped: the accessor's state word gains a "dropped by the check" state, and the read raises `top-level value '<name>' read after the allocation check dropped it`, located, instead of reading freed memory (the accessor's fast path still tests one word);
- a hook may read a value not yet initialized: it is initialized, registered, and dropped in turn, the check looping until none is left.

Hooks' output appears only under the check; no correct program depends on running a top-level value's hook, which the program never does.

**Compile-time cost.** None. **Run-time cost.** Nothing without the check; a state store per value dropped by it.

Rejected:
- **Never running hooks of top-level values, even under the check**: resources they hold are reported as leaks.
- **Rejecting top-level values whose type reaches a drop hook**: global resources for a program's life are legitimate, and the check would still need the rule above for library types.
- **Dropping top-level values at every exit**: C++'s static destruction order problems, rejected by `execution` D12.

### D14. A runtime error in a hook ends the program; exiting runs no hook (autonomous)

Prism has no unwinding: a runtime error writes `panic:` and ends the process with status 101, running no further drop (`execution`). A runtime error in a hook does the same, located in the hook; the values left are not dropped, and their hooks do not run. `exitProcess` runs no drop and so no hook. A hook therefore cannot rely on running for cleanup that must survive a crash; resources the runtime owns are released with the process.

**Compile-time cost.** None.

Rejected: running the remaining drops after a panic (unwinding: a second panic during it, and code paths compiled for every call).

### D15. Hooks and the composition: after the pass when released, when the cell is freed for state values, by the pass's rules inside it (autonomous; follows R1 and R2 of the user's A2‴)

- A remembered value released because its group left, its key changed or its composition was disposed runs its hook when the runtime releases its slot. Releases run after the pass ends (`end_pass` before `process_releases`), so hooks there run outside composition, as handlers do: they may write states (invalidating readers for the next pass) and dispose compositions they own.
- A state's value is dropped when its cell is freed, which R2 defers to the end of the first pass after the state's retirement (`owned-state` D7). A resource held in a state is therefore released one pass after its state was dropped. That is the price of R2's guarantee that no handler meets a freed cell, and it is what `owned-state` already does for the value's memory.
- A value dropped while a pass runs (a local of a composable body, or of a plain function called from one) runs its hook during the pass, under the pass's rules: a state write is the run-time error `state written during composition` (the static `E4031` applies only to composable contexts, which a hook is not).

**Compile-time cost.** None.

Rejected: deferring every hook run during a pass to its end (a hidden queue of values to drop later, changing when locals die).

### D16. Diagnostics: `E4035` widened, one new code `E4036` (autonomous)

- **`E4035` `InvalidOperatorDeclaration`** (from `operator-conventions`) also covers hook shapes (D3), a drop hook on a `: Copy` type, a `: Clone` type with a drop hook and no clone hook (D4), and a clone hook without `: Clone` or on a `: Copy` type (D6).
- **`E4036` `DropHookCalled`** (new, the next free code of the type checker's range once `E4035` is taken; no open packet takes another `E40xx` code): a call of a drop hook by name. A new kind of mistake with its own fix ("let the value's scope end, or move it into a function"), so its own code.
- **`E4017`** gains taking apart a value with a drop hook (D5); **`E4010`** an extension named `drop` on a type with a drop hook (D3); **`E4016`**'s row notes the waiver for clone hooks (D6); **`E6004`** covers growing cycles of hook requests (D11).

### D17. Compile timing (autonomous)

`crates/prism-cli/tests/compile_timing.rs` gains two shapes at 256, 1,024 and 4,096: `drop-hooks` (that many structs, each with drop and clone hooks, each dropped, cloned, held in a list and in a field of another struct, and taken apart by a borrowing pattern), timing check and code generation; and `generic-hook-instances` (one generic type with hooks instantiated with that many distinct struct type arguments through a generic function), timing summary and planning. Each asserts growth of at most ×4.6 per ×4 under `PRISM_ASSERT_TIMING=1`.

### D18. The resource in fixtures is a composition handle (autonomous)

Executable fixtures need a runtime resource that user code can acquire and release and that `--check-allocations` counts. `/prism/composer` already offers one: `startComposition` returns an `Int` handle, `disposeComposition` releases it, and the check reports undisposed compositions with status 102. A `Session(val id: Int)` wrapping it shows the leak without a hook and none with one, with no new external. Hook order and counts are shown with hooks that `println`.

Rejected:
- **A test-only external counter in the shipped library** (`/prism/testing`): a test API in `/prism/...` that every program could import.
- **Library test cases** (`// library:` fixtures with their own externals): they cannot be executable (`fixture_tests.rs`).

## Risks / Trade-offs

- **[Glue now runs user code]** A hook can print, write state, start or dispose compositions, or panic, from inside a list drop, a box drop or a release. → Every such site already tolerates runtime calls (glue checks the stack, releases run with no borrow of the runtime's state held, `process_releases` is a queue). The reviewer should probe hooks that dispose compositions during a release (re-entrant `process_releases`), hooks that drop lists of hooked values, and a hook that panics inside list glue.
- **[A hook reading a top-level value during the check]** → D13's state guard; the reviewer should probe values initialized during the check.
- **[One pass late]** Resources in states are released a pass after their state is dropped (D15). → Documented; it follows R2.
- **[Over-approximate hook instances]** D11 compiles a hook instance for every hooked type reached by a type argument, even when an instance only borrows it. → One small instance each, cached; exact requests would need per-instance walks the planner cannot do.
- **[`drop` as a name]** Existing code with an extension named `drop` on a type that later gains a hook gets `E4010`. → No code in the repository has one; the message names the hook.
- **[Stale handles through copied paths]** D8 lets a closure keep `t.id` after `t`'s hook released it. → As with `val id = t.id`; runtime handle checks report misuse.
- **[No `forget`]** A consuming API that hands its handle to the runtime must disarm the hook through a field (D5). → Follow-up if `library-in-prism` needs one.

## Migration Plan

No migration: no program declares `operator` functions before `operator-conventions`, and nothing changes for types without hooks. The `execution` text "glue runs no user code" becomes "only through hooks". Build-cache entries are keyed by the compiler's hash, so nothing cached is reused. Rollback: revert the commits.

## Implementation Notes

Recorded while applying (2026-10-20); none changes a decision above.

- **Declarations (group 2).** `prism_abi::hook_types` is the one syntactic match of D10, used by Phase 1 and by the type checker, which records the file's own flags in `TypeTable` before any extension is registered, so that `E4010`, `E4036` and `E4035`'s marker rules see them; a malformed hook may still set a flag, but its `E4035` stops the program. The shape checks are `check_hook_declaration` and `check_hook_result` in `checker/operators.rs`. A hook's type parameters must also carry the type's own names, which is what rejects `operator mut fun <B, A> Pair<B, A>.drop()` (its parameters are positionally the type's). `E4036` is decided from the type's flag before extensions are looked up, so an importer, to which the private hook is invisible, gets it too. Taking apart a hooked value is the new `TypeError::TakeApartWithDropHook` (`E4017`), searched through nested constructor, `NonNull` and box patterns.
- **Scenario forms corrected.** In "Drop hook shapes rejected", `operator mut fun drop() {}` is a syntax error (a `mut` receiver mode needs a receiver), so the form without a receiver is `operator fun drop() {}`; and the public form is written `public operator mut fun Session.drop(): Unit {}`, since a public function without its result is `E4011` besides. In `code-generation`, symbols name an extension's receiver with its module (`main::main.Session.drop`, `lib/net::lib/net.Session.drop`, `main::main.Owned.clone<String>`), as `symbols.rs` always has; the scenarios now say so.
- **Glue (group 3).** `glue::needs_glue(kind, ty, types)` is the predicate of D9 (`owns_storage`, or `TypeTable::reaches_drop_hook`/`reaches_clone_hook`, which are constant time for a table without hooks and otherwise walk each reachable type once). The hook is called through the slot of `glue::hook_symbol`, with the glue's own parameters, which are exactly the hook's machine signature (`(&mut T)` is one pointer; `(&T) -> T` is a result address and a pointer). `LayoutTable::environment_layout` asks the same predicate, so a hooked capture is never kept inline without a drop entry. The program's type table that instances are compiled over (`plan::program_types`) carries the flags too, which the first probe of "Parts are cloned by their hooks" found missing.
- **Generic hooks (group 4).** `HookWalk` in `crates/prism-mir/src/summary.rs` walks the local types of every compiled body and the type arguments of its calls, which covers what a body drops (every dropped place is a local or a part of one), clones (a clone's result is a local), owns in a closure's environment (a closure body's capture locals) and passes to generics; a reference is not walked, since a borrow drops nothing. It is constant when the file's table has no generic type with a hook. A generic record is walked once for its function and closures and once for each default and its closures, since each is instantiated alone. Requests are ordinary `InstanceRequest`s of the hook extension, so the summary's encoding needs no new field and the cache, keyed by the compiler's hash, needs no version bump. `check_cross_file_recursion` already sees every generic's calls; its message now says "polymorphic recursion" (without "across files" when the cycle stays in one file), names generics as `app/main::Owned.drop`, and notes when the growing request is a hook's. In `code-generation`, "Clone glue calls a generic type's clone hook instance" now reads the glue from `prism dump-clif` (the file's unit) and the instance from `--instances`, which prints instance units only.
- **Top-level values (group 5).** `prism_rt_register_top_level_value` also takes the value's state byte; `finish_tracking` pops the latest registered value, writes the new state `TOP_LEVEL_VALUE_DROPPED` (3) into its byte and then runs its glue, until none is left, so values a hook initializes meanwhile are dropped in turn. The accessor returns 0 while its initializer runs (as before) and 1 once the value was dropped; a reader tests `address < 2` (one compare, as the null test was) and only its cold block tells the two errors apart. Reading the state byte in the reader was tried first and rejected: a reloaded unit cannot resolve another module's `#state` symbol. `prism_runtime::finish` flushes the program's output again after the drops, since hooks print. The new `PanicKind::TopLevelValueDropped` reads `top-level value '<name>' read after the allocation check dropped it`.
- **Hot reload (group 6).** `TypeFingerprints::hash_nominal` writes `drop hook` and `clone hook` for a type with them; a type without hooks keeps its fingerprint.
- **Composition (group 7).** The scenarios pass as written. The re-entrancy probe of task 7.1 (a `Session` in a remembered `MutableState<Session?>` that `main` fills through a weak reference, whose group leaves; its hook disposes the session's composition when the cell is freed at the end of the next pass, and again when the check frees the program's cells) passes without a runtime change; it is kept as the fixture `a-state-s-value-runs-its-hook-when-its-cell-is-freed--disposing-a-composition`.
- **Compile timing (group 8).** `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` with `PRISM_TIMING_SHAPES=drop-hooks,generic-hook-instances`, 2026-10-20, passes. `drop-hooks` at 256 / 1,024 / 4,096: type 8.9 / 38.7 / 193.0 ms, borrow 12.1 / 50.1 / 221.4 ms, lower 12.8 / 53.7 / 237.6 ms, clif 41.7 / 168.8 / 685.1 ms (every stage x4.0 to x5.4 per x4; the largest growth, compose x5.4, is a stage of 25 ms that hooks do not touch). `generic-hook-instances` (n instances of the generic function and 2n hook instances): inst 5.4 / 22.5 / 94.9 ms, lower 8.2 / 34.2 / 140.6 ms, clif 16.4 / 66.2 / 270.2 ms (x4.2 per x4); its warm instance collection, added to `LINEAR_INSTANCES`, grows x3.68 per x4 on average. D11's over-approximation costs nothing measurable here: every requested hook instance is used.

## Open Questions

- Should `library-in-prism` add `forget` (or a consuming `into...` pattern) for handles given back to the runtime? Decide with the first library type that needs it.
- Generic `equals` could use D11's request mechanism; worth revisiting in `operator-conventions`' follow-up.
