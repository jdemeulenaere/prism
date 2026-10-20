# Design: layout-primitive

## Context

See `proposal.md` for the motivation. This change is item 11a of "After Milestone 2: the review round" in `ROADMAP.md`, the first half of the last part of agreed item **A4** of `docs/milestone-2-review-notes.md`:

- "as much as possible ... normal prism code that is imported like any other code" (user, 2026-10-20);
- layout native "for now (later a Compose-style `Layout` and `MeasurePolicy`)";
- "layout in Prism this round, as the last item" (user decision, 2026-10-20).

The orchestrator relayed the user's direction for this item: follow Compose's `Layout`, `MeasurePolicy`, `Measurable`, `Placeable`, `Constraints` and `MeasureScope` as closely as Prism allows; compile speed stays a primary criterion; modifiers stay plain data, never composed.

The user is away. Every decision the user might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Decisions the user took in the review are marked **(user direction 2026-10-20)**. Compile speed is a primary criterion (AGENTS.md §5), so each decision states its compile-time and run-time cost.

**Baseline.** Proposed on top of `f35a446` ("Propose compose-library-in-prism"), with these packets landing before this one, in `ROADMAP.md`'s order:

- `vararg` (item 9, being implemented);
- `core-library-in-prism` (10a): extension properties with getters, used by D16;
- `field-visibility` (10b): `package` fields, so library handles cannot be forged, and `Modifier`'s hidden representation;
- `compose-library-in-prism` (10c):
  - `NodeId` is a `Copy` struct over a `package` word, and the UI externals take `Int` words (its D8);
  - `Node` and `LeafNode` are `@ExplicitGroupsComposable` Prism functions (its D9);
  - `runApp` is Prism (its D10);
  - registrations return a handler word that the toolkit wraps.

Library code in this design is written on that baseline. No open packet modifies a block this one modifies. Task 1.1 rebases each block on the main specs once they are archived. `layout-in-prism` (11b) is proposed alongside and lands after this change.

**Baseline update (task 1.1, 2026-10-20, on `e3d0e070` "Archive compose-library-in-prism").** `vararg`, `core-library-in-prism`, `field-visibility` and `compose-library-in-prism` are archived. Reconciliation:

- **MODIFIED blocks.** Each was compared with the current main text of `ui` ("Constraint Layout in Logical Pixels", "Layout Reuses Clean Measurements", "Script Targets by Kind", "Dumping the Laid-Out Tree", "Layout and Drawing After a Reload", "Layout and Drawing Take Linear Time") and `example-apps` ("The Toolkit Tour"). None changed since the proposal: every main scenario is kept, and only this change's sentences differ. No ADDED block names a requirement that exists by now.
- **Names the tasks use** all exist as named: `Context::measure`, `measure_content`, `measure_box`, `measure_line`, `relayout_from`, `remeasure_dirty_children` and `Layout::run` (`layout.rs`); `NodeKind`, `Node`, `UiTree::set_on_value_change`, `value_change_pair`; `HandlerKind`, `HandlerTable`, `runtime::borrowed`, `runtime::run_handler`; `UI_FUNCTIONS`, `TreeBuilder`, `App::reload_point`, `App::settle`.
- **How `compose-library-in-prism` landed.** `NodeId` is `package struct NodeId : Copy { package val word: Int }` and `HandlerId` a public struct over a `package` word; `Node(kind, update, content)` and `LeafNode(kind, update)` are `@ExplicitGroupsComposable` package functions of `/prism/compose/ui`; the setters are `package external fun prism_ui_*(node: Int, ...)`; registrations return an `Int` word. D9's `LayoutNode` is written on exactly this shape.
- **`Layout::run` and the thread state.** Today the app calls `with_state(|state| layout.run(&mut state.tree, measurer))`, so the whole pass holds the thread's borrow. D14's rule means the pass itself moves into the thread state's reach: see D21.

`openspec validate layout-primitive --strict` is clean with no INFO lines.

**Current state** (main specs and code):

- **Node kinds.** `crates/prism-ui/src/node.rs` has `NodeKind::{Window, Column, Row, Box, Text, Spacer, TextField}`. A node holds its arrangement and alignment, its decoded chain (`Vec<ChainEntry>`) and its layout state:
  - `last_constraints`, `size` and `x`/`y` relative to the parent;
  - `content`, the rectangle where its text or children start;
  - `needs_measure` and `child_needs_measure`.
- **The layout pass** (`crates/prism-ui/src/layout.rs`, ui-toolkit D11) runs under one `&mut UiTree`.
  - `Context::measure` folds the chain natively (padding, size, defaultMinSize, fillMax; drawing elements record bounds), then `measure_content` dispatches on the kind: text breaking for `Text`, one line for `TextField`, `measure_box` for `Box` and the window, `measure_line` for `Column`/`Row`, and the minimum for `Spacer`.
  - Weights and alignments are parent data read from the child's chain (`Node::weight`, `Node::own_alignment`).
- **The cache.**
  - A clean node measured with its last constraints returns its size.
  - A node that is only `child_needs_measure` re-measures its dirty children with their last constraints and keeps its own size and their positions when each keeps its size (`remeasure_dirty_children`).
  - A relayout climbs from the dirty nodes (`relayout_from`) and stops at the first node whose size stays.
- **Program code in `prism-ui`.**
  - Handlers live in the handler table (`handlers.rs`, ui-toolkit D23), are claimed by nodes, and compare by code and captures through the descriptor's equality entry, outside the thread state's borrow (ui-toolkit D2).
  - One-word owning-nothing handlers are interned, so equal registrations are one id and allocate nothing.
  - Calling a function value loads its code from its function-table slot (`prism-runtime/src/function_value.rs`), so a reload is followed.
- **The library.** `/prism/compose/layout` declares `Column`, `Row`, `Box`, `Spacer`, the scopes, `Arrangement` and the layout modifiers. Each container is a `Node(kind, update)` whose `update` calls `prism_ui_set_arrangement`, `prism_ui_set_alignment` and `prism_ui_set_modifier`.
- **Numbers.** Prism has no conversion between `Int` and `Double`. `/prism/core` declares only printing, `toString()` and `String.length` (plus, after `core-library-in-prism`, the members it adds).

## Goals / Non-Goals

**Goals:**

- Compose's `Layout(content, modifier, measurePolicy)` and its measuring API, written in Prism, and usable by any program to write layouts such as a flow row or a grid.
- `Column`, `Row`, `Box` and `Spacer` written in Prism over the same primitive, with every existing dump, display list and session output unchanged.
- The native layout pass keeps its cache and its incremental relayout, and adds no allocation: a layout of the toolkit's containers allocates nothing.
- Hot reload swaps layout code, as it swaps handlers.
- No new compiler stage. Constant cost per node in each layout pass, with stated budgets.

**Non-Goals:**

- The layout modifiers in Prism (`layout-in-prism`).
- Intrinsic measurements, `SubcomposeLayout`, alignment lines, user parent data, lookahead, right-to-left layout (D20).
- Changing what any existing layout computes.

## Decisions

### D1. Two changes: the primitive with the containers, then the modifiers (autonomous)

Item 11 is split:

- **`layout-primitive`** (this change, 11a): the primitive, its native protocol, user layouts, and `Column`, `Row`, `Box` and `Spacer` moved onto it.
- **`layout-in-prism`** (11b): the layout modifier elements (`padding`, `width`, `height`, `size`, `defaultMinSize`, `fillMax*`) measured by Prism code as Compose's `LayoutModifierNode`, with `Modifier.layout { }` and `offset`.

The containers are in the first change because they are the strongest test of the primitive: every existing ui fixture and session must keep its byte-identical output through it. The modifiers change the native chain fold, a separate risk, and need the measurable to name a position in a chain.

**Compile-time cost.** None by itself.

Rejected:

- **One change**: about twice the risk in one review, across the chain fold and the container policies at once.
- **Containers in the second change**: the primitive would land with only new fixtures to prove it.

### D2. The primitive lives in `/prism/compose/ui` (user direction 2026-10-20: Compose's module split)

Compose declares `Layout`, `MeasurePolicy`, `Measurable`, `Placeable`, `MeasureScope` and `MeasureResult` in `androidx.compose.ui.layout`, and `Constraints` in `androidx.compose.ui.unit`, all in the `compose-ui` artifact. So they go in `/prism/compose/ui`. `Column`, `Row`, `Box` and `Spacer` stay in `/prism/compose/layout` (`compose-foundation-layout`), now written over `Layout`.

**Compile-time cost.** `/prism/compose/ui` grows by about 120 lines, parsed and checked once per cache for programs importing it.

### D3. `Layout(content, modifier, measurePolicy)`, and a measure policy is a function type (autonomous)

```kotlin
public compose fun Layout(
    content: compose () -> Unit,
    modifier: Modifier = Modifier,
    measurePolicy: attached MeasureScope.(&List<Measurable>, Constraints) -> MeasureResult,
): Unit
```

**Compose's shape.**

- This is Compose's parameter order, defaults and names.
- Compose's `MeasurePolicy` is a `fun interface` whose single method is `MeasureScope.measure(measurables: List<Measurable>, constraints: Constraints): MeasureResult`. Prism has no interfaces and no SAM conversion, so a measure policy is the function type of that method.
- Compose code reads the same: `Layout(content, modifier) { measurables, constraints -> ... }`.

**`attached`.**

- The node keeps the policy for as long as the composition keeps the node, exactly as it keeps a click handler (`owned-state` D9: the toolkit's handler parameters are `attached`).
- So the policy captures states through weak handles, holds no borrow, and is checked by the existing `E5010` rules.

**`&List<Measurable>`.** The measurables are borrowed from the runtime (D4), as Compose's `List<Measurable>` is read-only.

**No content-less overload.** Prism has no overloading. A leaf layout passes `content = { }`. The toolkit's own leaf, `Spacer`, uses a package helper over `LeafNode` (D9).

**Compile-time cost.** A call checks one lambda with a receiver against a function type, as `Box { }` checks its content today: linear, no inference across arguments.

Rejected:

- **A `MeasurePolicy` struct wrapping the function** (`Layout(content) { ... }` would become `Layout(content, measurePolicy = MeasurePolicy { ... })`): every call would deviate from Compose's trailing-lambda form.
- **A `typealias MeasurePolicy`**: a language feature of its own, which deserves its own review. It is a follow-up, and the function type stays valid under it.
- **`escaping` instead of `attached`**: an escaping closure cannot capture a state by reference (`E5007`), while a policy reading a remembered state's value is natural Compose code.

### D4. Measurables are lent as `&List<Measurable>` from a native buffer (autonomous)

When the native pass calls a policy, it fills a buffer with the node's children's `Measurable` values, in order, and passes the address of a list header `{ pointer, size, capacity }` (code-generation's layout of `List<T>`) that describes the buffer. The header lives on the Rust frame that makes the call.

- There is one buffer per depth of nested policy calls, kept by the layout pass and reused across passes. The buffer of depth `d` is not touched while a policy at depth `d` runs, so its address stays valid during the call.
- The policy sees an ordinary `&List<Measurable>`: `size`, `measurables[i]`, `for (m in measurables)`, `map`, `forEach`, `clone()`. It cannot keep the reference past the call, which the borrow checker enforces, since a function type's reference parameter is borrowed for the call alone.
- Prism code never frees or grows it, because a `&List` lends neither.

**Run-time cost.** One word written per child per policy call, and no allocation once the buffers have grown.

Rejected:

- **A new list per call**: an allocation per measured layout node per pass.
- **A custom `Measurables` view type** with `size`, `get` and an iterator: no allocation either, but it is not Compose's `List`, and the collection helpers would not apply.

### D5. Handles and scopes are `Copy` structs over `package` words; Compose's members are extensions (autonomous)

In `/prism/compose/ui`:

```kotlin
public struct Measurable : Copy { package val node: Int }
public struct Placeable : Copy { package val node: Int, val width: Int, val height: Int }
public struct MeasureScope : Copy { package val node: Int }
public struct PlacementScope : Copy { package val node: Int }
public struct MeasureResult : Copy { val width: Int, val height: Int, package val node: Int }

public fun Measurable.measure(constraints: Constraints): Placeable =
    prism_ui_measure(node, constraints)

public fun MeasureScope.layout(
    width: Int,
    height: Int,
    placementBlock: PlacementScope.() -> Unit,
): MeasureResult {
    prism_ui_begin_placement(node, width, height)
    placementBlock(PlacementScope(node))
    prism_ui_end_placement(node)
    return MeasureResult(width, height, node)
}

context(scope: PlacementScope)
public fun Placeable.place(x: Int, y: Int): Unit = prism_ui_place(scope.node, node, x, y)

context(scope: PlacementScope)
public fun Placeable.placeRelative(x: Int, y: Int): Unit = prism_ui_place(scope.node, node, x, y)

context(scope: MeasureScope)
public fun Dp.roundToPx(): Int = value
```

- **Unforgeable.** Every handle has a `package` field, so no program constructs one (`field-visibility`, `E4007`). The words are `NodeId` words of the native tree. `Placeable`'s `width` and `height` are public and read-only, as in Compose.
- **Placing.** `place` and `placeRelative` are context extensions, as Compose's `PlacementScope` members are and as `weight` already is. A call outside a placement block has no `PlacementScope` in its context, so it is `E4034` at compile time. There is no right-to-left layout yet, so `placeRelative` places as `place` does (D20).
- **Density.** `Dp.roundToPx()` needs a `MeasureScope`, as Compose's `Density` member does. A `Dp` is a whole logical pixel (ui-toolkit D7), so it gives `value`.

**Compile-time cost.** Ordinary declarations.

**Run-time cost.** Each handle is an aggregate passed by address ("Aggregates Are Passed by Address"); `Measurable` is one word in the lent buffer.

Rejected:

- **Intrinsic one-word structs** (in registers): `compose-library-in-prism` D8 moves `NodeId` and `HandlerId` away from intrinsic structs, and the catalog should shrink, not grow.
- **`place` as a plain extension with a runtime check only**: the context makes the common mistake a compile-time error.

### D6. The measurement protocol: re-entrant calls, measure once, placement inside `layout` (autonomous)

A layout pass calls into Prism and Prism calls back. While a policy runs, the pass keeps a stack of **measurement frames**. Each frame holds the node, a fresh stamp, the content origin given by the chain fold, whether `layout` was called, and whether placement is running.

**Measuring a child.** `prism_ui_measure(child, constraints)` is accepted only when the innermost frame is the child's parent's, and the child was not yet measured in that frame (Compose: `measure` may be called once per measurable per measurement).

- The constraints must have `0 ≤ minimum ≤ maximum` on each axis.
- The child is then measured by the native `measure`, with the cache, which may call the child's own policy and push a deeper frame.
- The child takes the frame's stamp, and the call returns `Placeable(child, width, height)`.

**`layout(width, height) { ... }`.**

- The first `prism_ui_begin_placement` of the frame records the size and opens placement.
- The placement block runs at once, inside the call.
- `prism_ui_end_placement` closes placement.
- A second `layout` in the same frame is misuse.

**Placing.** `prism_ui_place(parent, child, x, y)` is accepted only while the innermost frame is the parent's and is placing, and when the child was measured in that frame. It sets the child's position to the content origin plus `(x, y)` and marks it placed.

**The result.**

- When the policy returns, `layout` must have been called in its frame. Otherwise it is misuse.
- The content's size is the result's size clamped into the content constraints, as the native kinds clamp today. Compose instead centres a measured size that falls outside the constraints. Prism's containers never produce one, and clamping keeps today's numbers.

**Placement runs at once.** Compose returns a `MeasureResult` whose `placeChildren()` runs in a later placement pass. Prism runs the block inside `layout`, for three reasons:

- the block stays a non-escaping closure whose environment borrows the policy's frame, so nothing is allocated;
- the native pass stays one measure-and-place walk (ui-toolkit D11);
- the pass needs no stored closures.

The cost is that a change that only moves children cannot skip measuring (D20).

**Compile-time cost.** None: library code.

**Run-time cost**, per policy call:

- one call through the function table;
- two entry calls for `layout`;
- per child, one entry call to measure (a frame check, then the cache check) and one to place.

Rejected:

- **Compose's deferred placement pass**: an owned closure per measured node (an allocation whenever it captures more than one word, as `placeables` does), and a second walk.
- **Allowing two measurements of a child**: useful for measure-twice layouts, but the relayout cache keeps one set of last constraints per child (D11), and Compose forbids it. A layout that needs it waits for intrinsics (D20).
- **Centring an out-of-range size** as Compose does: no container needs it, and it would change today's clamping numbers.

### D7. Children a policy does not place are not shown (autonomous)

A policy may leave a child unmeasured or unplaced, for instance one that overflows.

- An unplaced node and its subtree are not drawn, not hit-tested and not script targets (`click "<text>"` and kind targets skip them).
- Dumps print them, with `unplaced` after the node's other properties and before its modifier elements, at their last position and size (`0,0 0x0` for a node never measured).

This is Compose's behaviour, where an unplaced `Placeable` is neither drawn nor hit. Every child of a native kind (text, text field, window) is placed.

**Run-time cost.** One flag per node, cleared for the children of a node before its policy runs. One flag test per node in drawing and hit testing.

Rejected:

- **An error for an unplaced child**: legitimate layouts hide children.
- **Omitting unplaced nodes from dumps**: a dump would no longer show the whole tree.

### D8. Policies are kept in the handler table, claimed by their node (autonomous)

`Layout` registers its policy and gives the node the returned word, the way `clickable` registers `onClick` and a text field claims its edit handler:

- `prism_ui_register_measure_policy(policy)` moves the function value into the handler table as a new `HandlerKind::Measure` entry and returns its word.
- The node's `update` calls `prism_ui_set_measure_policy(node, word)`. As `set_on_value_change` does, it keeps the node's policy when the named entry is it or is equal to it (same code, equal captures, compared outside the state's borrow, ui-toolkit D2). Otherwise it claims the new entry, releases the old one, and marks the node `needs_measure`.
- Unclaimed entries are released at the end of the pass, and a released node releases its policy.
- The table's existing rules apply:
  - a capture-free or one-`Copy`-word policy is interned, so an equal registration on a later pass is the same id and allocates nothing;
  - a policy that captures more allocates its environment, as any such closure does.

**Calling a policy.** The pass copies the function value out of its entry (`runtime::borrowed`) and calls it through its descriptor's slot, as handlers are called. So a reload's repointed slot runs the new code, and a slot whose contract changed keeps the old code until the node is given a new policy, as handlers do ("Handlers Written by Reloaded Code Are Replaced Before the Next Event").

**Compile-time cost.** None.

**Run-time cost.** Per `Layout` per pass: one registration, one comparison (the descriptor's equality entry) and one release, as for a clickable handler today.

Rejected:

- **A function value stored in the node itself**: it duplicates the table's ownership, equality, interning and release rules.
- **Equality by descriptor alone**: a policy whose captured gap changed must lay its node out again.

### D9. The containers are capture-free policies over node properties (autonomous)

`Column`, `Row`, `Box` and `Spacer` move to Prism, in `/prism/compose/layout`, over two package helpers of `/prism/compose/ui`:

```kotlin
@ExplicitGroupsComposable
package compose fun LayoutNode(
    kind: Int,
    measurePolicy: attached MeasureScope.(&List<Measurable>, Constraints) -> MeasureResult,
    modifier: Modifier,
    update: (NodeId) -> Unit,
    content: compose () -> Unit,
): Unit {
    val policy = prism_ui_register_measure_policy(measurePolicy)
    Node(kind, { node ->
        prism_ui_set_measure_policy(node.word, policy)
        prism_ui_set_modifier(node.word, modifier)
        update(node)
    }) {
        content()
    }
}
```

`LeafLayoutNode` is the same over `LeafNode`. `Layout` is `LayoutNode(layoutKind, measurePolicy, modifier, { }, content)`.

- **No extra group.** The helpers are `@ExplicitGroupsComposable` (`compose-library-in-prism` D5), so composition dumps and node identity are unchanged.

```kotlin
public compose fun Column(
    modifier: Modifier = Modifier,
    verticalArrangement: Arrangement = Arrangement.Top,
    horizontalAlignment: Alignment = Alignment.Start,
    content: compose ColumnScope.() -> Unit,
): Unit {
    LayoutNode(
        columnKind,
        { measurables, constraints -> measureLine(true, measurables, constraints) },
        modifier,
        { node ->
            prism_ui_set_arrangement(node.word, verticalArrangement.code, verticalArrangement.space)
            prism_ui_set_alignment(node.word, horizontalAlignment.code)
        },
    ) {
        content(ColumnScope())
    }
}
```

**Capture-free policies.**

- The policy lambdas capture nothing, so they are interned (D8), and a pass that recomposes a container registers, compares and releases its policy without allocating.
- Their arrangement and alignment are node properties, set by the setters the containers already call, and read back by the policy through private externals of `/prism/compose/layout` (`prism_ui_arrangement(node): Arrangement`, `prism_ui_alignment(node): Alignment`).
- The same properties feed the dumps, so `arrangement spacedBy(8)` is printed as today.
- A changed arrangement marks the node `needs_measure` through its setter, as today.

**Parent data.**

- A child's first `weight` and first `align` in its chain are read through `package val Measurable.parentData: ParentData`, over `prism_ui_parent_data(node)`.
- `ParentData` is a `package` struct of `weight: Double` (0.0 for none), `fill: Bool` and `alignment: Int` (-1 for none).
- This is Compose's `Measurable.parentData` (`RowColumnParentData`, `BoxChildDataNode`), kept package-only (D20).

**No list of placeables.**

- Compose's `RowColumnMeasurePolicy` keeps an array of placeables between measuring and placing. The toolkit's policies instead get each child's placeable again in the placement block, through `package fun Measurable.measured(): Placeable` (`prism_ui_measured`, valid only for a child measured in the running frame).
- In Compose, a child's measurable and placeable are the same object (its `MeasurePassDelegate`), so this is a lookup, not a second measurement.
- So a layout of the containers allocates nothing.

**Arithmetic in Prism.** `measureLine` (columns and rows) and `measureBox` port `measure_line`, `measure_box`, `Arrangement::start_and_gap` and `AxisAlignment::offset` exactly:

- the same order of measurement (unweighted children first, then weighted ones);
- integer division truncating toward zero;
- weight shares by `(remaining.toDouble() * (weight / total)).toInt()`, with the last weighted child taking the remainder.

`Spacer` is `LeafLayoutNode(spacerKind, { measurables, constraints -> layout(constraints.minWidth, constraints.minHeight) { } }, modifier, { })`.

**Compile-time cost.** About 200 lines of library code in `/prism/compose/layout`, checked once per cache.

**Run-time cost**, per container per measurement:

- two property reads;
- per child: one parent-data read, one measure, one `measured`, one place;
- arithmetic in Tier 1 code instead of Rust.

Budgets are in D18.

Rejected:

- **Policies capturing the arrangement and alignment** (Compose's `RowColumnMeasurePolicy` fields): three words of captures, so an allocated environment for every container on every pass that recomposes it.
- **Compose's `remember(arrangement, alignment) { policy }`**: the remembered closure is borrowed from its slot, and registering it would need a duplicate, which allocates for the same captures.
- **Packing arrangement and alignment into one captured word**: obscure, and arrangements with spacing do not fit cleanly.
- **A `List<Placeable>` per measurement**, as Compose's array: an allocation per container per layout.

### D10. The containers keep their node kinds; user layouts are kind `layout` (autonomous)

- `Column`, `Row`, `Box` and `Spacer` keep node kinds 1, 2, 3 and 5. A user `Layout` creates kind 7, `layout`, whose dump line and script target word are `layout`.
- The native pass measures every one of these five kinds only through its policy.
- `Text` (4), `TextField` (6) and the window (0) keep their native content measurement: text breaking and metrics stay in Rust, behind the `TextMeasurer`. The window measures its children under its loose size and places them at its top-left corner, a few native lines left of `measure_box`.
- The kind keeps its other roles: dump names, script targets, default arrangement and alignment, and node identity (a node group composed with another kind gets a new node).

**Compile-time cost.** None. **Run-time cost.** None.

Rejected:

- **One `layout` kind for every container**: every dump, every ui scenario and every session snapshot would change, and dumps would lose which container a node is.
- **A dump label set from Prism per node** (`prism_ui_set_label(node, "column")`): a string per node per pass, for what the kind already says.

### D11. The cache and the relayout climb are unchanged; policies are pure (autonomous)

The layout pass keeps ui-toolkit D11's cache and `layout.rs`'s climb from dirty nodes. That is correct because a policy's result is taken to depend only on:

- its constraints;
- its children's sizes and parent data;
- its node's properties;
- its captures, whose change gives a new policy (D8).

So:

- **A clean node** measured with its last constraints returns its size without calling its policy.
- **A node that is only `child_needs_measure`** re-measures its dirty children with their last constraints, and does not call its policy when each keeps its size: the policy would give the same size and positions.
- **A child the policy did not measure** in its last run has its last constraints cleared when the run ends. Its later changes therefore always re-run the parent's policy, since there are no constraints to re-measure it with alone.

**State reads.** A policy that reads a state's value outside composition is not observed: a write to that state does not re-measure the node. Compose observes reads during layout and re-measures. That needs a layout observer in the state runtime, a follow-up (D20). The ui spec says so.

**Compile-time cost.** None.

**Run-time cost.** A relayout stays linear in the dirty nodes, their ancestors and those ancestors' children ("Layout and Drawing Take Linear Time"), plus one policy call per node re-measured.

### D12. A reload measures every policy's node again (autonomous)

After a reload is applied, the next layout measures again every node of the five policy kinds (D10). A reload may have changed a policy's code, or the code of a function a policy calls in another file, and the cache cannot see either. That layout marks the display list stale only when it changed a node's size, position or content rectangle. So "no frame is presented when nothing visible changed" still holds.

**Compile-time cost.** None. **Run-time cost.** One full layout after each applied reload, against the reload's own pass, compile and debounce (the 50 ms latency target).

Rejected:

- **Re-measuring only nodes whose policy's slot was repointed**: it misses a policy calling an edited helper in another file, whose own slot does not change.
- **No re-measure**: an edited policy would show only after something else invalidated its node.

### D13. Misuse is a `ui misuse:` runtime error (autonomous)

These end the program as UI runtime errors, with no location ("UI Runtime Errors Have No Location"):

| Detail | When |
|---|---|
| `a measurable was measured outside its parent's measure policy` | no measurement frame is running, or the innermost frame is not the parent's |
| `a measurable was measured twice in one measurement` | second `measure` of one child in one frame |
| `invalid constraints: a minimum is negative or above its maximum` | `measure` with such constraints |
| `layout was called twice in one measurement` | second `layout` in one frame |
| `layout was called outside its node's measurement` | `layout` on a scope whose frame is not the innermost |
| `a placeable was placed outside its parent's placement` | `place` while the parent's frame is not placing, or for a child not measured in it |
| `a measure policy returned without calling layout` | the frame ends with no `layout` |
| `a layout node has no measure policy` | a node of a policy kind measured with no policy, which only a raw node group of `/prism/composer` can make (D22) |

Placing outside a placement block in source is also `E4034` (D5).

**No new error code**: they are runtime errors, and `E4034` exists.

**Run-time cost.** A comparison or two per entry call.

### D14. Policies run as program code of the app (autonomous)

A policy runs during the frame loop's layout, outside any pass, with no thread state borrowed. That is the rule for handlers (ui-toolkit D2): every entry call re-borrows the tree briefly, and `Layout::run` no longer holds `&mut UiTree` across calls into Prism.

- It runs as a handler of the app's composition (`runtime::run_handler`), so a state it creates belongs to the app, and a state it writes invalidates its readers, which the frame loop recomposes before the next event, as after a handler.
- A runtime error in a policy ends the program, with its location, as any.

**Run-time cost.** Each entry call borrows the thread's UI state once.

### D15. `Int.toDouble()` and `Double.toInt()` in `/prism/core` (autonomous)

Weights need `Int` to `Double` and back, and Prism has neither conversion.

`/prism/core` declares, as public intrinsics:

- `fun Int.toDouble(): Double`, the nearest `Double`;
- `fun Double.toInt(): Int`, which truncates toward zero, gives 0 for NaN, and saturates at `Int`'s bounds.

These are Kotlin's names and semantics, and Kotlin's `toInt()` also saturates.

- They are members of the built-in types, so no import is needed.
- The backend lowers them inline to Cranelift's `fcvt_from_sint` and `fcvt_to_sint_sat`. NaN gives 0 in the saturating instruction.

**Compile-time cost.** Two catalog entries and two `BuiltinFunction` arms. **Run-time cost.** One instruction each.

Rejected:

- **A native helper for weight shares** (`prism_ui_weight_share(remaining, weight, total)`): it hides arithmetic every user layout needs too.
- **Trapping conversions** (Rust's `as` saturates too; Java's `(long)` saturates): saturation is Kotlin's behaviour.

### D16. `Constraints`: a `Copy` struct with Compose's helpers (autonomous)

```kotlin
public struct Constraints : Copy {
    val minWidth: Int
    val maxWidth: Int
    val minHeight: Int
    val maxHeight: Int
}

public struct ConstraintsCompanion : Copy { val Infinity: Int }

public val Constraints: ConstraintsCompanion =
    ConstraintsCompanion(Infinity = 9223372036854775807)
```

**`Infinity`.** `Constraints.Infinity` is `Int`'s maximum, the native `UNBOUNDED` (Compose uses `Int.MAX_VALUE` for its 32-bit `Int`).

**Helpers**, Compose's names:

- on the companion: `fixed(width, height)`, `fixedWidth(width)` and `fixedHeight(height)`;
- extension properties: `hasBoundedWidth`, `hasBoundedHeight`, `hasFixedWidth`, `hasFixedHeight` and `isZero`;
- functions: `constrainWidth(width)`, `constrainHeight(height)`, `constrain(other)`, `offset(horizontal = 0, vertical = 0)` (adds to each bound, floored at 0, with `Infinity` kept), and `copy(minWidth = this.minWidth, ...)`.

**Construction.**

- Compose's `Constraints(minWidth = 0, maxWidth = Infinity, ...)` is a factory function with defaults beside a value class. Prism has no struct field defaults (type-system, companion values) and a function cannot share the struct's name, so `Constraints(...)` takes all four values.
- The struct's fields are public, so any program builds constraints. `measure` validates them (D13).
- The companion's type is named `ConstraintsCompanion`, since the plural that names the other companions (`Arrangements`, `Colors`) does not exist for `Constraints`.

**Compile-time cost.** About 50 lines of library code. **Run-time cost.** Prism calls, as written.

Rejected:

- **Private fields with a validating factory**: Prism cannot name the factory `Constraints`, and validation at `measure` is where Compose's own invariant matters.
- **32-bit `Infinity`**: the native tree is 64-bit, and a second sentinel would need translating at every crossing.

### D17. The engine's Rust tests drive Rust policies through the same entry points (autonomous)

`prism-ui`'s own tests build trees through the Rust API: the layout unit tests, the randomized `incremental_relayouts_match_full_layouts`, `tests/ui_timing.rs` and `tests/allocations.rs`. They need policies without Prism.

- `test_support` gains Rust policies: `extern "C"` closure code with the policy's signature and a null environment, for columns, rows, boxes and spacers.
- They call the same entry functions a Prism policy calls, through their Rust API, and they are ports of today's `measure_line` and `measure_box`.
- `TreeBuilder::add` gives each container node its Rust policy.

The tests keep their expectations, and they now exercise the re-entrant protocol (frames, stamps, lent buffers, placement).

The production `measure_line` and `measure_box` are removed. The Prism policies are checked against the same numbers by the CLI fixtures and sessions, which keep their byte-identical dumps.

**Compile-time cost.** None.

Rejected:

- **Keeping the native containers as a fallback**: two implementations of one behaviour in production.
- **Running Prism inside `prism-ui`'s tests**: the crate depends on neither the compiler nor the backend ("The Toolkit Is a Native Node Tree Without Window Code").

### D18. Cost model and budgets (autonomous)

**Compile time.**

- No new stage and no new analysis. A call of `Layout` is an ordinary composable call with a lambda with a receiver.
- The library grows by about 370 lines over `/prism/compose/ui` and `/prism/compose/layout`, checked and compiled once per build cache for programs importing them.
- A `layout-policies` shape is added to `compile_timing.rs`: one file with `n` composables, each calling `Layout` with a policy that measures and places its children. It runs at 256, 1024 and 4096 and must grow within ×4.6 per ×4, like the other shapes.

**Run time**, per layout pass:

- one policy call per measured node of the five policy kinds;
- per measured child, one measure entry call and, in the containers, one parent-data read, one `measured` call and one place call;
- two `layout` entry calls and two property reads per container;
- no allocation once the lent buffers and the frame stack have grown;
- text nodes, the window and cache hits cost what they cost today.

**Budgets**, measured on an idle machine on a release build before the first implementation commit (task 1.2) and after the last (task 8.3):

- **`ui_timing`** (the engine, with D17's Rust policies): its assertions hold, with each tenfold tree at most ×15, and a one-text relayout of 10,000 nodes under a tenth of the full layout. The full layout of 10,000 nodes must stay within 2× its baseline, which bounds the protocol's overhead.
- **`app_frame_timing`**:
  - every existing target is still met;
  - the paced toggle and typing sessions' `layout` medians may grow by at most 0.05 ms;
  - a new session resizes `large.prism` between 800 × 600 and 801 × 600 fifty times, each a full relayout of about 7,000 nodes, 5,000 of them through their policies (seven nodes a row, five of them containers). Its `layout` p99 must be under 4 ms, it must report the median against task 1.2's native baseline (at most 3×), and every resize frame after the first must allocate nothing (`example-apps` "A Full Relayout of a Thousand Items").
- **Startup**: the todo app's first frame within 3 ms of its baseline cold and 1 ms warm, and under the 50 ms and 20 ms targets.
- **`ui_run_timing`**: under 1 ms per click.

A missed budget is investigated before archiving, as `compose-library-in-prism` D15 does.

**Baseline (task 1.2, 2026-10-20, release build on `e3d0e070`, M4 Pro, no other build running).**

| Measurement | Median | p99 / other |
|---|---|---|
| `ui_timing` full layout, 1,000 / 10,000 / 100,000 nodes | 0.066 / 0.493 / 2.708 ms | growth ×7.5, ×5.5; relayout / layout at 10,000: 0.0002 |
| `ui_timing` display list, same trees | 0.035 / 0.259 / 1.829 ms | growth ×7.4, ×7.1 |
| `app_frame_timing` counter click, frame | 0.003 ms | 0.003 ms |
| todo toggle (paced), `layout` / frame | 0.002 / 1.958 ms | 0.005 / 3.686 ms |
| todo typing (paced), `layout` / recomposition / frame | 0.002 / 1.499 / 2.474 ms | 0.004 / 3.031 / 4.489 ms (frame p99 target of 4 ms already missed before this change: the `strong-skipping` follow-up) |
| todo toggle back to back, `layout` / frame | 0.000 / 0.159 ms | 0.000 / 0.202 ms |
| todo first frame | 31.568 ms cold | 3.143 ms warm |
| `ui_run_timing`, a click among 1,000 texts | 0.208 ms | |
| `large.prism`, 50 × (`resize 800 601`, `frame`, `resize 800 600`, `frame`), `layout` | 0.017 ms | 0.023 ms; 0 allocations per frame, none in 99 of 101 |
| `large.prism`, 50 × (`resize 801 600`, ..., `resize 800 600`, ...), `layout` | 0.177 ms | 0.196 ms; 0 allocations per frame, none in 99 of 101 |
| `compile_timing` `ui-calls` 256 / 1024 / 4096, `compose` stage | 11.5 / 51.2 / 227.3 ms | ×4.7 per ×4 |
| `compile_timing` `nested-compose-ui` 16 to 128, `compose` stage | 4.9 / 9.3 / 19.1 / 37.4 ms | ×2.01 per ×2 |

**Correction of the resize session (autonomous).** A height change of one pixel does not relayout `large.prism` in full: past the first screen, the column gives its rows a main-axis maximum of 0 at either height, so they keep their constraints and their cached sizes (0.017 ms above). A width change gives every row a new cross-axis maximum, so each of the about 7,000 nodes is measured again (0.177 ms, the first frame's layout). The resize session and the scenario "Resizing a thousand items allocates nothing" therefore alternate between 800 × 600 and 801 × 600. Rejected: keeping the height change, which would measure the cache, not the policies.

**After (task 8.3, 2026-10-20, release build on the implementation, same machine, no other build running).**

| Measurement | Baseline | After | Budget |
|---|---|---|---|
| `ui_timing` full layout, 1,000 / 10,000 / 100,000 nodes (Rust policies) | 0.066 / 0.493 / 2.708 ms | 0.047 / 0.503 / 6.122 ms | 10,000 within 2× (1.02×): met; growth ×10.7, ×12.2 and display list ×11.0, ×13.5, under ×15: met |
| `app_frame_timing` counter click, frame median / p99 | 0.003 / 0.003 ms | 0.004 / 0.005 ms | under 1 / 2 ms: met |
| todo toggle (paced), `layout` median | 0.002 ms | 0.009 ms | +0.05 ms at most: met |
| todo toggle (paced), recomposition median / frame p99 | 0.134 / 3.686 ms | 0.181 / 4.227 ms | under 1 ms: met; under 4 ms: **missed** (see below) |
| todo typing (paced), `layout` median | 0.002 ms | 0.003 ms | +0.05 ms at most: met |
| todo typing (paced), recomposition median / frame p99 | 1.499 / 4.489 ms | 1.827 / 4.948 ms | under 2 ms: met; under 4 ms: missed, as at baseline |
| todo toggle back to back, frame median / p99 | 0.159 / 0.202 ms | 0.175 / 0.268 ms | no target |
| frames changing nothing, allocations | none | none | met |
| resize `large.prism` between widths 800 and 801, `layout` median / p99 | 0.177 / 0.196 ms (native) | 0.610 / 0.635 ms | p99 under 4 ms: met; median within 3×: **missed** (3.45×) |
| resize frames after the first, allocations | none in 99 of 101 | none in 100 of 101 | met (a first relayout's growth of the tree's dirty list, also native, now reserved) |
| resize between heights 600 and 601, `layout` median | 0.017 ms | 0.058 ms | no target |
| todo first frame, cold / warm | 31.6 / 3.14 ms | 39.4 / 3.73 ms | +3 ms cold: **missed** (+7.8 ms); +1 ms warm: met (+0.59 ms); under 50 / 20 ms: met |
| `ui_run_timing`, a click among 1,000 texts | 0.208 ms | 0.231 ms | under 1 ms: met |
| `compile_timing` `ui-calls`, `compose` 256 / 1024 / 4096 | 11.5 / 51.2 / 227.3 ms | 12.2 / 53.5 / 238.1 ms | linear: met |
| `compile_timing` `layout-policies`, slowest stage growth | — | ×4.1 (`borrow`, `clif`), `compose` ×4.9 / ×5.1 per ×4 | under ×8: met |

**The misses, investigated (autonomous: accepted, with follow-ups).**

- **Resize layout at 3.45× the native layout.** The Rust side of the protocol is cheap: `ui_timing`, whose containers are Rust policies through the same entry points, lays out at 1.02× the native pass once the chain fold was passed by reference (it was 2.2× while the fold's arrays were moved through each step) and parent data was kept in the node (two chain scans per read before). What remains is the count of crossings: about 55 entry calls per todo row (per child: a parent-data read in each of up to three loops, a measure, a `measured` and a place; per container: two property reads and the two halves of `layout`), each a call out of Tier 1 code and one borrow of the thread's state, plus the arithmetic in Tier 1 code. A sample of fifty thousand resizes shows thread-local access and `RefCell` borrows as the largest single items. The absolute target (p99 under 4 ms) holds with a margin of six. Follow-ups: package entry points that return a child's placeable with its parent data in one call, and a Tier 2 build that inlines the entry calls.
- **Cold first frame +7.8 ms.** The library is compiled cold: `/prism/compose/ui` grew by the primitive (about 30 functions) and costs 3.0 ms more to compile without a cache, `/prism/compose/layout` by its policies and helpers, 4.3 ms more (an import-only program: 7.5 → 10.5 ms with `ui`, 12.5 → 19.8 ms with `layout`). Warm, from the build cache, it is +0.59 ms. Follow-up, as `compose-library-in-prism` noted: compiling library functions on first use.
- **Paced toggle frame p99 4.23 ms** (3.69 at baseline, target 4 ms). Its `layout` stage grew by 0.007 ms; the rest is the paced measurement's spread on a core clocked down between inputs (F4 in `docs/milestone-2-review-notes.md`: the same frame measured 3.0 to 6.0 ms p99 across runs), and about 10 % in stages that walk every node (display list, hit test), which `Node`'s 40 more bytes (policy, parent data, stamps) slow on 7,000 nodes. Back to back, the frame grows from 0.159 to 0.175 ms median.


### D19. Behaviour is unchanged; the tour shows a custom layout (autonomous)

Every program keeps its output and exit status, `--check-allocations` included, and every example session keeps its snapshot, except the toolkit tour, which gains a custom `Layout` (a `FlowRow` over a few `Text`s) so that the examples show the primitive.

The accepted differences:

- `prism abi` of `/prism/compose/ui` and `/prism/compose/layout`: the primitive's declarations, and the containers' bodies no longer naming `Node`, which `prism abi` does not show anyway;
- the tour's snapshot;
- `--explain-cache` listings that name the two modules' new compile work, if any test pins it.

### D20. Deferred (autonomous)

Each of these becomes an unchecked follow-up in `ROADMAP.md` at archive:

- **Intrinsic measurements** (`IntrinsicSize`, `Modifier.height(IntrinsicSize.Min)`, `minIntrinsicWidth` on `Measurable` and in policies): Compose's second protocol, with its own cache.
- **`SubcomposeLayout` and `BoxWithConstraints`**: composition during layout, which the frame loop's order (compose, then lay out) does not allow yet.
- **Alignment lines** (`alignBy`, baselines).
- **Parent data for user layouts** (`Modifier.layoutId`, `ParentDataModifierNode`): the containers' parent data stays a package accessor.
- **Observing state reads in measure policies** (D11).
- **Placement-only relayout** (D6).
- **Right-to-left layout**: `placeRelative` places as `place`.
- **`typealias`**, which would give `MeasurePolicy` its name (D3).
- **`Int.coerceIn`, `coerceAtLeast`, `maxOf` and the rest of Kotlin's number helpers** in `/prism/core`, so that user layouts need fewer `if`s.

### D21. The pass runs on the thread's state, one borrow per step (autonomous, implementation)

Added at implementation (task 3.3). D14's rule means every entry call a policy makes must reach the tree, the handler table and the running pass. So the pass's own state moves into the thread's state, beside the tree:

- `State.layout: LayoutState` holds the frame stack, the measurable buffers per depth, the text-breaking buffer, the stamp counter, the run's text measurer (a pointer valid only while `Layout::run` runs) and the composition policies run as handlers of.
- `Layout::run(measurer)` lays out the thread's tree, and no longer takes `&mut UiTree`. `prism-ui`'s Rust tests and benchmarks build trees in a `TreeBuilder`, whose `lay_out` swaps its tree and handler table into the thread's state for the run (`TreeBuilder::installed`).
- A measurement is a short sequence of **steps**, each one borrow of the state: the cache check and the chain fold, then, for a text or text field, the content and the fold back to the outermost element in the same borrow; for a policy kind, pushing the frame and lending the children; the policy call with no borrow; then popping the frame and the fold back. `measure_child` does its frame checks and the child's first step in one borrow, so measuring a text child is one borrow.
- The fold's arrays (seventeen constraints and offsets) live on the measuring Rust frame and are passed by reference, never moved: moving them through the steps' results cost a third of the full layout's time in `ui_timing` (2.2× the native layout, against 1.3× by reference).

- A node keeps its parent data (first `weight` with its `fill`, first `align`), recomputed when its chain is set, so a policy's read is one load instead of two scans of the chain. The fold's arrays are left uninitialized past the chain's length, so a measurement the cache answers costs no 800-byte fill.
- The end of a policy's run visits its children again only when one was not measured or not placed, or was unplaced before.

**Run-time cost.** Per policy node, two borrows of the thread's state around the call; per child, one borrow per entry call. `ui_timing`'s full layout of 10,000 nodes: 0.50 ms against the native 0.49 ms (1.02×, budget 2×).

Rejected:

- **A pointer to the running pass's context in a thread-local**: a `&mut` alive in the calling frame while an entry call makes another is undefined behaviour in Rust.
- **The pass's buffers in the app (`App`)**: entry calls cannot reach them.

### D22. Raw node groups and native kinds (autonomous, implementation)

`/prism/composer` is public (`compose-library-in-prism`), so a program can start a node group of a layout kind without going through the toolkit, and so without a policy. The proposal assumed only the toolkit could. Found when the `ui` scenario "A node group composed with another kind gets a new node", which starts raw groups of kinds column, row, text and spacer, ended in `ui misuse: a layout node has no measure policy`:

- A node of a policy kind measured without a policy ends the program with that misuse, as Compose's `LayoutNode` without a measure policy throws. A new requirement, "A Layout Node Needs a Measure Policy", states it with the scenario "A raw column without a policy".
- A text or text field lays out no children: the children a raw node group gives one are unplaced (D7), never measured and not shown, rather than drawn at the text's corner.
- The scenario "A node group composed with another kind gets a new node" now alternates its raw groups between the two native kinds, text and text field, its texts inside the first unplaced. It still covers what it was written for: a kind change under the node key makes a new node, and the children keep theirs.

Rejected: a native fallback measuring a policy-less node as a box (D17 rejects native containers); a public way to set a raw node's policy (`ComposeNode`'s `update` with `SetMeasurePolicy`), a larger API for no current use.

**Material's surfaces.** `SurfaceBox` of `/prism/compose/material3` started a box node directly. It is now a `LayoutNode` with the box policy, which `/prism/compose/layout` declares `package` (`measureBox`), as Compose's `Surface` is a `Box`.

**Registering a policy** checks the pass as the setters do (`register_measure_policy`): a container composed outside `runApp` keeps ending with `ui misuse: toolkit composables run only inside runApp`, not with the handlers' `modifier handlers are made only while runApp runs`. `Measurable.measured()` of a child not measured in the running measurement, which only toolkit code can call, reuses the detail `a placeable was placed outside its parent's placement`.

### D23. Sizes and positions stay within Kotlin's `Int` range (autonomous, review)

Added at review. A policy can measure a child under `Constraints.Infinity`, return `layout(constraints.maxWidth, ...)` under unbounded constraints, or `place` a child at any `Int`, and a modifier can ask for `padding(9223372036854775807.dp)`. The native pass then added such values (padding around a size, a child's position to its parent's content origin, absolute bounds in dumps, drawing and hit testing) and overflowed: a Rust panic inside an entry point, which aborts the process ("panic in a function that cannot unwind"). The padding case predates this change; the protocol made the rest reachable.

- A node's size is clamped to at most `MAX_EXTENT`, 2,147,483,647 (Kotlin's `Int.MAX_VALUE`, the range of Compose's sizes and offsets), after its content and after each padding of its chain. The constraints themselves keep `Constraints.Infinity` and any bound a program gives.
- `place(x, y)` clamps `x` and `y` to `±MAX_EXTENT` before adding the content origin.
- A modifier's padding, size and minimum size are clamped to `MAX_EXTENT` when the chain is decoded, so dumps print the clamped value.
- So every sum the pass, dumps, drawing and hit testing make stays far inside `i64`: at most 17 extents per node and one node per level of a tree, whose depth the program's stack bounds.

A size clamped this way may be below its constraints' minimum when that minimum is above the range; nothing reads it as an error.

**Compile-time cost.** None. **Run-time cost.** A `min` per side in `finish` and per padding, two clamps per `place`.

Rejected:

- **Saturating arithmetic at every sum** (layout, dump, drawing, hit testing, scripts, the desktop rasterizer): many sites to keep right, and the values reach the rasterizer near `i64`'s limits.
- **A `ui misuse:` error for an out-of-range size or position**, as Compose throws for sizes it cannot represent: it would end programs for a padding no window shows, and `Constraints.Infinity` measured by a `Box` would be an error.
- **Clamping to a smaller range** (Compose's packed `Constraints` hold less): sizes up to `Int.MAX_VALUE` are Compose's `layout` range.

## Risks / Trade-offs

- **[Re-entrancy in `prism-ui`]** The pass calls program code that calls the pass. → No borrow of the thread's UI state is held across a call (D14). Frames are a stack, and every entry call checks the innermost frame (D13). The randomized incremental-versus-full test keeps running through Rust policies (D17).
- **[Lent buffers]** A `&List` over Rust memory. → The header lives on the calling Rust frame. A depth's buffer is not touched while its policy runs, and a `&List` cannot be kept, freed or grown (D4). The reviewer should probe nested layouts several levels deep, and a policy that clones its measurables.
- **[Purity assumption of the cache]** A policy reading state is not re-measured (D11). → Specified as such, with a follow-up.
- **[Frame times]** Arithmetic moves from Rust to Tier 1 code, plus entry calls. → Budgets in D18, and the resize session measures a full relayout through Prism.
- **[Dump drift]** A port that rounds differently. → Every existing ui fixture and session must stay byte-identical, and the D17 port is checked by the same unit tests.
- **[Packets landing in between]** → Task 1.1 rebases.

## Migration Plan

Programs keep their meaning. The library changes:

- `ui.prism` gains the primitive, `LayoutNode`, `LeafLayoutNode`, `layoutKind` and their externals;
- `layout.prism`'s containers call `LayoutNode`/`LeafLayoutNode` with policies;
- `core.prism` gains the two conversions.

The runtime gains the measure entry points and the `Measure` handler kind, and loses `measure_line` and `measure_box`. The build cache is keyed by the compiler identity, so nothing cached is reused. Rollback: revert the commits.

## Open Questions

- Whether user layouts should get `Measurable.measured()` publicly, so that they too avoid a list of placeables. Compose has no such member; for now it is package-only.
- Whether to observe state reads in policies (Compose's layout snapshot observer), and at what cost per read.
