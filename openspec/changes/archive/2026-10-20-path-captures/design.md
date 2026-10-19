# Design: path-captures

## Context

See `proposal.md` for motivation. This change is agreed item A2b of the post–Milestone 2 review (`docs/milestone-2-review-notes.md`, "Agreed changes": "captures the path `todo.done` as its `Copy` handle instead of the borrowed `todo` (E5007 today), like Rust 2021's disjoint captures; decided syntactically, at no inference cost"), item 3 of "After Milestone 2: the review round" in `ROADMAP.md`. Item 4, `owned-state` (A2′, A2‴), builds on it. The user is away: every decision the user might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`; those taken in the review are **(user direction 2026-10-20)**. Compile speed is a primary criterion (AGENTS.md §5), so each decision states its compile-time cost.

Current state:

- **Captures are whole bindings.** `BodyBuilder::use_local` (`crates/prism-typechecker/src/builder.rs`) records, for each open closure a binding is older than, a `Capture { local, span, mode: Borrowed }` at the binding's first use; `ExpressionKind::Closure { captures, ownership_reason }` holds them in first-use order. The rationale "Fields are disjoint by path; closures capture whole bindings" of `references` (from `borrowck-exclusivity`) left precise captures for later: "precise captures would need capture paths in the typed tree and partial environments in `closures`".
- **Capture modes are decided by one linear pass** (`crates/prism-typechecker/src/captures.rs`, `decide_captures`, D1/D2/D19 of `escaping-closures`): after a body with closures is checked, one walk of its typed tree builds a flow graph of closure literals and function-valued bindings with roots at owning positions; a graph search marks escaping closures, which own every capture; block extents give the captures a closure outlives, which it owns. Owned captures are `Copied` (a `Copy` binding) or `Moved`; `E4028` rejects a `var`, a non-`Copy` capture of the enclosing closure and an earlier parameter read in a default value. On bodies far denser in closures than real code the pass measured 10% to 14% of `prism check`.
- **The typed tree's places are explicit.** `Field { base, index }`, `Deref { pointer }` (a `&T`, `&mut T` or `Heap<T>`), `Index`, `NonNullPayload` (inside a `SafeNavigation` chain) and `StatePlace { handle }` (a state's `value`), wrapped by `Copy`, `Move` or `Borrow` where used. Copy-out of `r: &Int` is a copy of `Deref(Local r)`; `todo.done` for `todo: &Todo` is `Field(Deref(Local todo), done)`.
- **MIR** (`crates/prism-mir/src/build/closure.rs`) gives a closure one `LocalKind::Capture` local per capture, of the binding's type, mapped in the closure builder's `bindings`, and builds `Rvalue::Closure { captures: Vec<Capture> }` from `Capture::Borrowed(place)` or `Capture::Owned(operand)`, taking each binding's place in the enclosing builder. `Builder::place` (`build/place.rs`) lowers a place expression recursively into a `Place { local, projections }`.
- **The borrow checker** (`crates/prism-borrowck`) already compares places by projections ("Disjoint Fields Are Borrowed Independently"), infers each capture local's mode from writes through places rooted at it (`captures.rs`), makes a loan of each `Capture::Borrowed` place (`borrows.rs`), and rejects at escape positions a closure owning a value that may hold a borrow (`escapes.rs`, `E5007`), whose help today is "pass the value 'r' refers to instead of a reference".
- **The backend** lays a borrowing environment out as one pointer per capture (the captured place's address, in the creating frame), an owning one as each owned capture by value and each borrowed one as a pointer, held inline in the environment word when it fits and needs no drop; a closure owning only `Copy` captures has environment equality and duplicate entries, which compose uses to compare kept handlers. A slot's contract records each capture's mode and type fingerprint (`hot-reload`).
- **Apps work around whole captures**: `examples/apps/todo/screen.prism` writes `val id = todo.id` before its handlers, as does its hot-reload session file `sessions/editing-the-row-layout-keeps-the-list.screen.prism.v2`.

**Baseline (task 1.1, 2026-10-20).** Proposed on top of `a62f48f`, which touched only `ui` and `desktop`, neither of which this change modifies; nothing has landed in `openspec/specs/` since. Every MODIFIED block was checked against the current main text and keeps all of its requirement's scenarios, no ADDED requirement exists in a main spec yet, `owned-state` and `strong-skipping` have not landed, and every function `tasks.md` names exists (`decide_captures`, `Graph::decide`, `capture_help` and the `capture_decision_timing` benchmark in `captures.rs`, `BodyBuilder::use_local`, `lower_closure`, `Builder::place`, the borrow checker's `escapes.rs`, `borrows.rs` and `captures.rs`). `openspec validate path-captures --strict` is clean.

## Goals / Non-Goals

**Goals:**
- Kotlin-like handlers over borrowed items (`{ state.setDone(todo.id, it) }`, `{ todo.done.value = it }`) with no workaround and no reference held by the handler.
- Disjoint closure borrows: a closure borrowing `p.x` leaves `p.y` usable, as two `&mut` borrows of different fields already do.
- Every program accepted today keeps its meaning (apart from drop timing, D9); programs rejected today for capturing a whole binding become valid where a path suffices.
- No new analysis: paths are decided in the existing capture pass, in time linear in the body; MIR, the borrow checker and the backend keep their algorithms.
- A design `owned-state` extends without revisiting: its weak state handles are one more mode on these paths (D11).

**Non-Goals:**
- Paths through indices, safe navigations, state values or calls (D1).
- Partial moves of non-`Copy` fields out of a captured binding (D3).
- Implicit clones of `Clone` paths (D3).
- Sharing one environment pointer among several borrowed paths of one binding (a possible optimization, D8).
- `owned-state`'s weak handles, composition-bound parameters and owning `MutableState` (item 4).

## Decisions

### D1. A capture is a path: fields, box dereferences and a first dereference of a `val` reference (autonomous: the path shape)

For each use of a binding of an enclosing scope inside a closure, the closure captures the path the use reads: the binding, then each `Field` and each dereference of a `Heap<T>` the use goes through, and a first dereference of the binding itself when the binding is a `val` of reference type (`&T` or `&mut T`). The path stops at the first node that is none of these: an `Index` (its index is computed at the use), a `NonNullPayload` (the `?.` test happens at the use), a `StatePlace` (the state is read at the use), a call or temporary, and a dereference of any other reference (the referent of `*rr` for `rr: &&T`, or of a `var` reference binding). So `todo.done` for `todo: &Todo` is the path `(*todo).done`, `t.id` for `t: Todo` is `t.id`, `c.inner.x` through a `Heap<P>` field is `(*(*c).inner).x`, `r + 1` for `r: &Int` reads `*r`, and `xs[i]` captures `xs` and `i`.

The two limits on references keep closures from silently missing a re-pointing: a `var r: &T` may be re-pointed (`r = &b`) while a closure that borrowed `(*r).x` still reads the old referent, and the borrow checker's writes are shallow (re-pointing `r` does not conflict with a loan of `*r`, `borrowck-exclusivity`), so the program would be accepted with a meaning that differs from capturing `r`. Stopping at the binding keeps today's behaviour (`E5004` for `r = b` while the closure lives). The same holds for the inner reference of `&mut &T`, which `*rr = &b` re-points; structs cannot hold references, so these are the only references a path can meet.

**Compile-time cost.** A path is read off the place chain of one use, in time linear in the chain, during the capture pass's existing walk (D5).

Rejected:
- **Whole bindings** (today): handlers over borrowed items cannot escape (`E5007`), and closures borrowing one field freeze the whole value.
- **Rust 2021's rule for `move` closures, truncating at the dereference of a reference** and moving the reference: Prism cannot own a reference (`E5007`), so it would reject exactly the motivating handler.
- **Paths through `?.`**: the test would move from the call to the closure's creation, and an owning closure would need a nullable copy of a payload that may not exist.
- **Paths through indices**: the index expression would be evaluated when the closure is built, not when it runs, changing evaluation order (Rust excludes them too).
- **Paths through `s.value`**: the state would be read at the closure's creation, losing the reads that register restart scopes.
- **Paths through any dereference**: silently changes what a closure reads after a `var` reference is re-pointed.

### D2. Overlapping paths merge into their prefix; order by binding, then by the paths' steps (autonomous, revised at implementation)

When a closure captures a path and a prefix of it (`{ p.x + size(p) }`), it captures the prefix only, so the closure holds no two overlapping captures and the borrow checker never sees a closure conflict with itself. Captures are grouped by binding, the bindings in order of first use as today, and a binding's paths in the order of their steps: fields in declaration order, a dereference after the fields of the same value. The closure's environment follows that order.

**Revised at implementation (2026-10-20).** The proposal ordered a binding's paths by the first use each covers. That made a closure's environment, and so its slot contract (`hot-reload`), depend on the order of uses inside it: the todo example's reload session, which moves `Text(todo.title)` before `Checkbox(checked = todo.done)`, changed the row lambda's captures from `todo.done, todo.title` to `todo.title, todo.done` and printed that values made before keep calling the previous version. Ordering by steps keeps the captures of a body whose uses of one binding are only reordered, as capturing the whole binding did. The scenario "A use of the whole binding subsumes its paths" now expects the field order for both closures, and the requirement "A Binding's Captured Paths Follow Its Fields" pins the rule.

**Compile-time cost.** Merging sorts a closure's recorded paths by binding position and steps, which puts every path right after its prefix, and merges them in one scan: O(m log m) comparisons of short step lists for m recorded paths, with no per-binding structure.

Rejected: keeping both the prefix and the longer path (two environment entries for overlapping places, and a closure's own captures conflicting in the borrow checker); ordering captures by first use across bindings (moves a binding's environment entries apart for no benefit, and the per-binding grouping is what the ownership decision works on); ordering a binding's paths by first use (the proposal: reordering uses changes the environment and the hot-reload contract).

### D3. Owning closures copy a path when it is `Copy` and no later write could change it; otherwise they own the whole binding (autonomous)

A closure that owns its captures (it escapes, or outlives a captured binding, as decided today) copies a captured path that has projections, where it is built, when:

1. the path's type is `Copy`;
2. its binding is a `val`;
3. it dereferences no `&mut` reference; and
4. every field it reads after dereferencing a reference is a `val` field.

Conditions 2 to 4 say that nothing the program does later can change the value the path names while the closure lives, so a copy reads exactly what a Kotlin closure would: Kotlin captures `todo`'s object reference, and a `val` field of that object never changes, while a `var` field, a `var` binding (Kotlin closures see later writes to captured `var`s, which is why `E4028` exists) or a referent replaced through a `&mut` (`*r = Todo(...)`) would make the copy stale. A `var` field of a `val` binding of value type cannot be written either, so it is copied (`Counter.n` of a by-value `c`).

Otherwise the closure owns the path's **whole binding**, merged with the binding's other paths, and today's rules apply unchanged: a `Copy` binding is copied, any other moved, a `var` is `E4028`, a non-`Copy` capture of the enclosing closure is `E4028`, a reference is `E5007`. A path without projections is the binding itself and follows those rules directly. So widening never rejects a program that is valid today, and a program that was `E5007` or `E5001` only because a closure took a whole binding for a `Copy` field becomes valid.

The decision is made after the ownership decision, per closure, innermost first (D5). The motivating handler `{ todo.done.value = it }` in `TodoRow(todo: &Todo)` copies the `Copy` handle `(*todo).done` (today `MutableState<T>` is a `Copy` handle; `owned-state` changes the leaf, D11).

**Compile-time cost.** Conditions 1 to 4 are read off the path's projections and the declared field mutability and markers the type table already holds: constant work per projection.

Rejected:
- **Rust 2021's partial move of a non-`Copy` field into the closure**: partial moves are out of the language (`ownership`, D7 of `payload-patterns`) because they need field-level drop flags and partial drops; the whole-binding move keeps every value whole.
- **A new error for a path the closure cannot copy**: it would reject programs that are valid today (a closure owning `u` for `u.name`).
- **Copying `var` fields read through a reference, or paths through a `&mut`**: the closure would silently stop seeing writes that a Kotlin closure sees.
- **Cloning a `Clone` path implicitly**: a hidden allocation and clone per closure built, against explicit `.clone()` (`ownership`); `E5007`'s help suggests the explicit clone instead (D10).

### D4. A path may end with a dereference, so `{ r }` for `r: &Int` copies the `Int` (autonomous)

Copy-out is a copy of a dereference in the typed tree, so `r + 1` and `println(r)` for `r: &Int` read the path `*r`, an `Int`, which an owning closure copies by D3. `fun f(r: &Int): () -> Int = { r }`, `Button({ r + 1 })` and `store { r }` become valid, as `{ t.id }` for `t: &Todo` does, where they were `E5007`. A closure that uses the reference itself (`{ r }` as a `() -> &Int`, or passing `r` on) still captures `r` and stays `E5007`.

This changes existing scenarios whose point was `E5007` for a closure capturing a reference: "An escaping closure capturing a reference", "Escaping through an escaping parameter", "Escaping closure labels" and "A borrowing closure stored in a state rejected" (`references`), and "Storing a borrowing closure rejected" (`collections`). Each keeps its intent with a non-`Copy` referent (`r: &String` with `greet(r)` or `r.length`), and new scenarios pin the copying cases.

**Compile-time cost.** None beyond D1.

Rejected: **stopping a path before a final dereference**, which keeps those scenarios as they are but makes `{ t.id }` valid and `{ r }` an error for the same kind of read, a distinction that would have to be explained; Kotlin has no such difference.

### D5. Paths are decided in the existing capture pass, flow-insensitively and in linear time (autonomous: where)

The type checker keeps recording whole-binding captures while it checks (`use_local`), since the checker's own rules (a move of a captured variable, `E4017`; an assignment to a captured variable) need only the binding, and a use's enclosing field chain is not known when its name is resolved. `decide_captures`, which already walks every node of a body with closures once, gains:

1. **Recording.** At a `Field` or `Deref` node reached from a node that is not one, it walks the chain down to its bottom. If the bottom is a `Local` that the innermost open closure captures, it records the path (cut at the first projection D1 excludes) for that closure and skips the chain's interior; otherwise it walks the bottom normally (an `Index`'s base and index, a call). A `Local` reached outside such a chain records the whole binding. The chain is visited once either way.
2. **Ownership**, unchanged: escaping closures and outlived bindings are decided per binding, as today, on the whole-binding captures.
3. **Paths, innermost closure first** (closures are numbered in creation order, so reverse order puts children first): a closure's paths are its own recorded paths plus its children's final paths whose binding is older than it, merged per binding (D2). For an owned binding, each path is copied or widened (D3), which can only shorten paths, so a parent's set always covers its children's. Then `E4028` is checked on the widened whole-binding captures as today.

The decision stays syntactic and flow-insensitive, as A2b asks (user direction 2026-10-20: "decided syntactically, at no inference cost").

**Compile-time cost.** No new pass and no new graph: the walk does O(chain length) more work per place chain inside a closure, and the merge a sort of its recorded paths (O(m log m) for m paths, D2), plus the propagation to parents, which is bounded by the captures the closures end up with (today's `use_local` already does O(closure depth) per use). The capture-decision benchmark (`capture_decision_timing`) gains field paths, and `compile_timing.rs` gains two shapes (tasks 8.1): `field-path-captures` (one closure reading `n` distinct fields and nested fields of a few bindings, the merge's width) and `nested-path-captures` (closures nested `n` deep, each reading paths of outer bindings, the propagation's depth). Both must grow linearly.

**Measured (task 3.3, 2026-10-20).** The `capture_decision_timing` generator gained, per step, a struct with a box field, a borrowing closure reading fields through the box and a reference, and an escaping closure copying fields through references with a nested borrowing closure. On it, the pass takes 3.6 to 50 ms for 1,000 to 8,000 steps, ×2.0 to ×2.3 per doubling, 9% to 19% of `prism check` (the share assertion is 20%). The pass before this change took 11% to 12% of the check on the same bodies: recording paths at each place chain and merging them costs about half again of the old pass on bodies this dense in closures, about 6% of the check. Merging reuses one buffer per pass, with a per-local position table instead of hashing, and a closure's repeated whole uses of a binding update one entry.

**Measured at review (2026-10-20).** The benchmark above timed a second run of the pass on bodies whose captures were already decided, which took a deduplicating copy of every closure's paths that the checker's own run never takes. Fed the checker's input (each captured binding once, whole), the pass takes 3.3 to 37 ms for 1,000 to 8,000 steps, ×2.0 to ×2.4 per doubling, 8% to 15% of `prism check`, against the 20% ceiling; the copy for decided trees is gone.

**Measured (task 8.1, 2026-10-20).** `PRISM_ASSERT_TIMING=1` passes for every shape. `field-path-captures` grows ×4.1 to ×4.8 per ×4 in every stage (26.6 ms to type-check 4,096 paths of one binding) and `nested-path-captures` ×1.9 to ×2.1 per doubling of the depth. The first run of `field-path-captures` found two super-linear steps of the type checker that predate this change, fixed in their own commit: a field access cloned all of its struct's fields (×15 per ×4), and recording a use inside a closure searched the closure's captures (`many-captures`, ×10 per ×4).

Rejected:
- **Recording paths in `use_local`**: the field chain around a name is built after the name is resolved, so the builder would have to patch captures as each `Field` node is added, for every open closure.
- **Deciding in MIR lowering or in the borrow checker**: the typed tree records modes and MIR builds owned captures from them (D2 of `escaping-closures`), and the borrow checker's elaborated MIR already depends on them.
- **Rust's per-use capture kinds with a flow-sensitive analysis**: no precision gained for Prism's rules, at a dataflow's cost.

### D6. The typed tree records each capture's projections and why an owned capture was widened (autonomous: representation)

`Capture` gains `projections: Vec<CaptureProjection>`, with `CaptureProjection::{Field(index), Deref}` (empty for a whole binding), and `widened: Option<Widening>`, for an owned whole-binding capture whose closure used it only through paths it could not copy: the first such path and the reason (`NotCopy`, `VariableBinding`, `MutableReference`, `VariableField`). The span stays the first use of any path the capture covers. Uses inside the closure body are unchanged nodes: a `Field(Deref(Local todo))` chain still names `todo`; the capture only says which part of it the closure holds.

`prism dump-typed-tree` prints a capture as source writes it: the binding and its fields joined by `.`, with the dereferences between them implied by the types, and a `*` for each dereference after the last field (`borrow p.x`, `copy todo.done`, `copy *r`), followed by `(widened from u.name: not Copy)` when it applies. Diagnostics name paths the same way. The validator checks each capture's projections against the binding's type and that no capture of a closure is a prefix of another.

**Compile-time cost.** A short vector per capture.

Rejected: a new expression kind for uses of a captured path (it would rewrite the closure's body and every consumer of `Field`); recording paths as expression ids (several uses share a path).

### D7. MIR has one capture local per path, and place lowering maps a chain onto it (autonomous)

`lower_closure` creates one `LocalKind::Capture` local per capture, of the path's type (`MutableState<Bool>` for `(*todo).done`, `Int` for `*r`), named by the path's text, and maps it in the closure builder by binding and projections. `Builder::place`, given a `Field` or `Deref` node whose chain bottoms at a binding with path captures, walks the chain once to its binding, finds the captured prefix (every use has one, by D1 and D2) and returns the capture local projected by the rest of the chain; a `Local` node of a whole-binding capture maps as today. Building the closure value, each capture's place is lowered the same way in the enclosing builder, which may itself be a closure holding the path or a prefix of it: `Capture::Borrowed(place)` for a borrowed capture, `Capture::Owned(Operand::Copy(place))` for a copied path, `Capture::Owned(Operand::Move(..))` only for whole bindings.

The borrow checker needs no new rule: a borrowed path is a loan of its place (`borrows.rs` already takes any place), overlap compares projections, the mode inference reads writes through places rooted at each capture local, and an owned copy is a read of the path's place where the closure is built (a use-after-move of the binding is still `E5001`). The compose transform's rule that a closure's mutable borrow stops a local from forwarding a parameter (`rewrite.rs`) takes the borrowed place's root local, as it must for any projected place.

**Compile-time cost.** O(chain length) per place chain in a closure body, with one hash lookup per chain. The number of capture locals grows from bindings to distinct paths, bounded by the uses.

Rejected: one capture local per binding with a partial environment (a local of a type with only some fields present, which MIR's types cannot express).

### D8. Environments, equality and contracts are unchanged rules over paths (autonomous: the environment trade-off)

A borrowed path's environment entry is the address of its place, computed where the closure is built, and a copied path's entry is its value; the inline-word rule applies as before, so a handler copying one handle or one `Int` through a borrow allocates nothing. A closure owning only `Copy` captures, copied paths included, has environment equality and duplicate entries, so a handler such as `{ println(item.id) }` in a composable with `item: &Item` is now keepable and compared by its captures: compose skips an unchanged `Plus`, where today it was `E5007`. A slot's contract keeps recording each capture's mode and type fingerprint; a path capture's type is its leaf's, so a reload that changes a closure from capturing `t` to `t.id` gives it a new slot, as any change of capture types does.

A closure borrowing several fields of one binding now holds one pointer per path where it held one pointer for the binding, as in Rust 2021. Borrowing closures are built in the creating frame without allocating, so the cost is a few words of stack per closure; sharing one pointer among the borrowed paths of a binding is a possible later optimization that changes no rule.

**Compile-time cost.** None: the same layout and glue code over a different list of captures.

### D9. A binding a closure no longer takes is dropped by its owner (autonomous)

When a closure copies `u.id` instead of moving `u`, `u` is dropped at the end of its own scope instead of with the closure's environment. Nothing can observe the difference today: drops free memory and run no user code (liveness ignores drops, D14 of `escaping-closures`), and `--check-allocations` sees every value freed once either way. `drop-and-clone-hooks` (item 8) will make drop timing observable; it must state that a closure keeps exactly the values it captures by value, as Rust 2021 did when its disjoint captures changed drop order.

Rejected: moving the whole binding into the closure whenever any of its paths is captured by an owning closure, which would keep today's drop timing and defeat D3 (`u` unusable after `Button({ u.id })`).

### D10. No new code; `E5007` and `E4028` name the path in their help (autonomous)

The causes stay the same, so the codes do:

- **`E5007`** for a closure owning a reference reached through a widened capture names the path in its help: "copy 'c.n' into a 'val' before the closure and capture that instead: 'val n = c.n'" for a `Copy` path, "clone 't.title' into a 'val' before the closure and capture that instead: 'val title = t.title.clone()'" for a `Clone` one, and today's "pass the value 'r' refers to instead of a reference" for a whole-reference use or a path of neither marker. A note says why the path was not copied when the reason is a `var` field ("a closure copies a field read through a reference only when the field is a 'val'") or a `&mut` reference ("'t' is a mutable reference, whose referent may be replaced"). The related location stays "'t' is captured here", at the first use of the path. MIR carries the widening from the typed tree on the capture local's declaration, which `escapes.rs` reads.
- **`E4028`** for a `var` binding used through a path suggests `val x = p.x` (the path's last field names the `val`, as Kotlin style would), rather than `val m = p`.
- **`E5003` to `E5006`** name the conflicting place as today; a closure's loan is now of the path, so its label reads "'p.x' is captured here".
- The catalog rows of `E4028` and `E5007` (`diagnostics`) get path examples; `E5007`'s current example `{ *r }` is not even valid syntax (`*` only writes) and becomes `{ r.length }` for `r: &String`.

Rejected: a new code for "a path that cannot be copied" (its fixes are those of `E5007` and `E4028`, and codes are one per fix, `diagnostics`).

### D11. `owned-state` adds a capture mode for state objects on these paths (forward note)

`owned-state` (A2′, A2‴, agreed by the user 2026-10-20) makes `MutableState<T>` a move-only object owning its cell, so `todo.done` will no longer be `Copy`, and by D3 alone an owning handler would widen to `todo` and be `E5007` again. A2′ says that "escaping closures that capture a state place capture an automatic generational `Copy` handle", and A2‴ (R3) limits that to closures passed to composition-bound parameters. That is one more outcome of D3's per-path decision: a path whose type is a state object, in a closure owned for a composition-bound reason, is captured as a weak handle copied from the place (a fourth `CaptureMode`, a `Copy` word in the environment, so equality and inline environments keep working), and every other non-`Copy` path still widens. D1's path shape, D2's merging, D5's pass, D6's representation and D7's capture locals are what `owned-state` needs; this change adds none of it, and `MutableState<T>` stays a `Copy` handle here.

### D12. The todo example captures `todo.id` directly (autonomous)

`examples/apps/todo/screen.prism` and its hot-reload session copy drop `val id = todo.id` and write `{ state.setDone(todo.id, it) }` and `{ state.remove(todo.id) }`, which exercises path captures in every example run and session. The example-apps spec states what the apps do, not how, so it needs no change.

## Risks / Trade-offs

- [Some scenarios that expected `E5007` now pass (D4)] → each is rewritten with a non-`Copy` referent keeping its intent, and new scenarios pin the copying; fixtures named after the old scenarios move with them.
- [A copied path snapshots a value read through a borrow, which a reader might expect to stay live] → only paths no later write could change are copied (D3); every other path keeps today's whole-binding rules and errors, whose help now shows the explicit copy.
- [Borrowing closures hold one pointer per path, more words for closures reading many fields of one value (D8)] → stack words in the creating frame, no allocation; an optimization can share a pointer later without changing any rule.
- [Drop timing of a binding no longer taken changes (D9)] → unobservable until `drop-and-clone-hooks`, which must define it.
- [MIR place lowering must never meet a binding that only has path captures] → by construction every use of such a binding inside the closure has a captured prefix; the typed tree's validator checks the captures cover the uses, and MIR panics with an internal error otherwise, which a fixture per path shape exercises.
- [Typed-tree and MIR snapshots of existing fixtures change (`mir/capture-modes-in-elaborated-mir` borrows `u.name` now)] → reviewed one by one when regenerated; only capture lists and closure operands may change.

## Migration Plan

Unreleased language: no migration. Every accepted program keeps its meaning except D9's drop timing; programs rejected only because a closure captured a whole binding for a `Copy` path become valid. Build-cache entries are keyed by the compiler's hash, so cached check and code entries from before the change are not reused. Rollback is a revert.

## Open Questions

- The exact wording of the new notes and helps (D10) may be tuned during implementation.

## Review (2026-10-20)

Probed with scratch programs: path shapes (fields, boxes, `val` references, the stops at indices, `?.`, state values, `&&T` and `var` references), merging, disjoint and overlapping closure borrows, owning closures copying or widening (also through loops, receivers, defaults, lambdas with a receiver and generic instances), drops under `--check-allocations`, compose skipping with copied paths, and closures with borrowed parameters returning borrows of captured paths. That last case, the implementer's flagged risk, is `E5009` as "Function Types Elide by the Same Rules" requires for a closure whose type takes a borrowed input, with or without a path (`app { k -> &t.lead }` as `app { k -> lead(t) }`), so it is not a regression. Fixed:

- `E5007` for a handler stored in a field that owns a `&mut` reference (`Button({ t.id })` for `t: &mut Todo`) labelled the parameter and gave the generic help: the moved reference no longer held its input where the closure escapes, so the escape check did not find the capture. It now names the path and the `&mut` note (fixture `references/escaping-closure-help-names-the-path--stored-in-a-field`).
- The capture benchmark timed a second run of the pass on already decided captures, which took a deduplicating copy the checker's run never takes; with the checker's input the pass is 8% to 15% of `prism check` (D5's review measurement), and the copy is gone.
- Closure parameters were declared in the enclosing closure, so the pass recorded their uses as captured paths and then dropped them; no capture changed.

Left as a ROADMAP follow-up: a slot contract fingerprints a captured path by its type only, so a reload swapping `{ t.id }` for `{ t.count }` keeps the slot and an old closure value runs the new code with the old path's copy.
