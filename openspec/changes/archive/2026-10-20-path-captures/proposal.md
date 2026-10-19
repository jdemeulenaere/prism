# Proposal: path-captures

## Why

A closure captures whole bindings today (`borrowck-exclusivity`, "Fields are disjoint by path; closures capture whole bindings"). So `onCheckedChange = { todo.done.value = it }` inside `compose fun TodoRow(todo: &Todo)` captures the borrowed `todo`, and a handler, which escapes, cannot own a reference: `E5007`. Apps work around it with `val id = todo.id` before every handler (`examples/apps/todo/screen.prism`), which Kotlin never needs. The post–Milestone 2 review agreed item A2b (`docs/milestone-2-review-notes.md`): closures capture *paths* such as `todo.done`, like Rust 2021's disjoint captures, decided syntactically at no inference cost. It is item 3 of "After Milestone 2: the review round" in `ROADMAP.md`, and a prerequisite of `owned-state` (A2′, A2‴), whose handlers capture a `MutableState` field of a borrowed item.

## What Changes

- **Closures capture paths.** Each use of an enclosing binding inside a closure captures the path it reads: the binding followed by the field accesses and box dereferences it goes through, and a first dereference of the binding itself when it is a `val` reference (`todo.done` for `todo: &Todo`, `p.x`, `h.value` through a `Heap`). A path stops at an index, a safe navigation, a state's `value`, a call, and any other dereference of a reference. When one captured path is a prefix of another, only the prefix is captured: `{ p.x + size(p) }` captures `p`.
- **Borrowed paths are disjoint.** A closure that borrows its captures borrows each path, shared or mutably as today, so `val bump = { p.x += 1 }; p.y = 5; bump()` is accepted, as borrows of different fields already are.
- **Owning closures copy the paths they can.** A closure that owns its captures (it escapes, or outlives a captured binding) copies a captured path where it is built when the path's type is `Copy`, its binding is a `val`, it dereferences no `&mut` reference, and every field it reads past a reference is a `val` field: nothing later in the program could then change what a Kotlin closure would see. Otherwise it owns the path's whole binding, exactly as today: moved, copied, `E4028` for a `var`, `E5007` for a reference. There are no partial moves (`ownership`, D7 of `payload-patterns`), so a non-`Copy` field is never moved out alone. **BREAKING (relaxing)**: `fun f(r: &Int): () -> Int = { r }` and `Button({ r + 1 })` now copy the `Int` instead of being `E5007`, and `Button({ u.id })` for a by-value `u` copies `u.id` and leaves `u` usable. Every program accepted today keeps its meaning, except that a binding a closure no longer takes is dropped at the end of its own scope rather than with the closure.
- **Handlers become keepable.** A handler copying `item.id` or a state handle field owns only `Copy` captures, so compose compares it by its captures and skips an unchanged composable, as for handlers capturing `Copy` bindings today.
- **Diagnostics.** No new code. `E5007` for a closure owning a reference, and `E4028` for a `var`, name the path the closure reads in their help: "copy 'c.n' into a 'val' before the closure and capture that instead: 'val n = c.n'", or the `.clone()` form, with a note when a `var` field or a mutable reference kept the path from being copied. The catalog rows of `E4028` and `E5007` get path examples.
- **Typed tree, MIR, backend.** A capture records its binding and its projections (fields and dereferences) with its mode, and why an owned capture was widened to its binding. MIR gives each captured path a capture local of the path's type and builds the closure value from the path's place; the borrow checker, environments, drops, descriptors and hot-reload contracts need no new rule, since they already work on places and on capture locals' types. `prism dump-typed-tree` prints captures as paths (`copy todo.done`, `borrow p.x`, `copy *r`).
- **The todo example** drops its `val id = todo.id` workarounds.
- **Not in scope:** capturing through indices, safe navigations or state values; partial moves of non-`Copy` fields; `owned-state`'s weak handles (it adds a capture mode for state objects on this change's paths); environments sharing one pointer for several borrowed paths of one binding (a possible optimization).

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `references`: closures capture paths; borrowed paths are borrowed independently; owning closures own paths; escaping closures copying through a reference hold no borrow; `E5007`'s help names the path; scenarios that relied on `{ r }` for `r: &Int` being `E5007` use non-`Copy` referents.
- `ownership`: owning closures copy the paths they can, and own the whole binding otherwise; `E4028`'s help names the path.
- `typed-tree`: captures record paths.
- `mir`: capture locals and closure values per path.
- `code-generation`: a handler copying a field through a borrow holds its environment inline.
- `compose`: a handler copying a field of a borrowed parameter is kept and compared.
- `collections`: the scenario of a borrowing closure stored in a list uses a non-`Copy` referent.
- `execution`: path captures run, read what they copied, and free every value once.

## Impact

- `crates/prism-typed-tree`: `Capture` gains `projections` (`CaptureProjection::{Field, Deref}`) and `widened` (the path and reason an owned capture was widened to its binding); dump and validator.
- `crates/prism-typechecker`: `captures.rs` computes paths in its existing walk, merges them per binding, and decides copy or widening after the ownership decision; `E4028`'s help; the capture-decision benchmark.
- `crates/prism-mir`: `build/closure.rs` builds a capture local per path and the closure value from each path's place; `build/place.rs` maps a place chain onto a captured path.
- `crates/prism-borrowck`: `E5007`'s help and note from the widened path (`escapes.rs`); tests for disjoint captures. No algorithm change.
- `crates/prism-compose-transform`, `crates/prism-codegen`: checks that a captured place with projections is handled where a binding's place was (`rewrite.rs` mutable-borrow tracking, closure environments, contracts).
- `crates/prism-cli`: fixtures under `tests/fixtures/{references,ownership,typed-tree,mir,code-generation,compose,collections,execution}/`, executable ones under `--check-allocations`; the existing E5007 fixtures rewritten; compile-timing shapes `field-path-captures` and `nested-path-captures`.
- `examples/apps/todo/screen.prism` (and `large.prism` if it has the same workaround).
- `openspec/specs/diagnostics/spec.md`: the `E4028` and `E5007` rows.
- Unblocks `owned-state` (item 4), whose handlers capture `todo.done` once `MutableState<T>` owns its cell.
