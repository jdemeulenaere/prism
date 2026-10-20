# Proposal: layout-primitive

## Why

The post–Milestone 2 review agreed that the standard and compose libraries become "as much as possible ... normal prism code that is imported like any other code" (item **A4** of `docs/milestone-2-review-notes.md`, "Agreed changes"; user, 2026-10-20). Layout stayed native "for now (later a Compose-style `Layout` and `MeasurePolicy`)", and the user then asked for "layout in Prism this round, as the last item" (item 11 of "After Milestone 2: the review round" in `ROADMAP.md`).

Today every layout rule is Rust code in `crates/prism-ui/src/layout.rs`:

- columns, rows and boxes are node kinds measured by `measure_line` and `measure_box`;
- arrangements, alignments and weights are Rust;
- a program cannot write a layout of its own: no `FlowRow`, no grid, no custom measurement.

In Jetpack Compose, `Column`, `Row`, `Box` and `Spacer` are ordinary library code over one primitive, `Layout(content, modifier, measurePolicy)`, and any app writes its own layouts the same way.

The item is too big for one change, so it is split in two (design D1), both proposed now:

- **`layout-primitive`** (this change, item 11a): the `Layout` primitive, `MeasurePolicy`, `Measurable`, `Placeable`, `Constraints`, `MeasureScope` and `layout(width, height) { place(x, y) }`, called from the native layout pass; and `Column`, `Row`, `Box` and `Spacer` rewritten in Prism over it.
- **`layout-in-prism`** (item 11b): the layout modifiers (`padding`, `size`, `fillMax*`, ...) as Prism code, Compose's `LayoutModifierNode`, with `Modifier.layout { }` and `offset`.

## What Changes

- **The primitive**, in `/prism/compose/ui`, where Compose declares it (`androidx.compose.ui.layout` and `ui.unit.Constraints`):
  - `Layout(content: compose () -> Unit, modifier: Modifier = Modifier, measurePolicy: attached MeasureScope.(&List<Measurable>, Constraints) -> MeasureResult)`, Compose's signature. A measure policy is that function type (D3).
  - `Constraints`, a `Copy` struct of four `Int`s, with `Constraints.Infinity`, `fixed`, `fixedWidth`, `fixedHeight`, `hasBoundedWidth`, `constrainWidth`, `offset`, `copy`, ... (D16).
  - `Measurable.measure(constraints): Placeable`, at most once per measurement (D6).
  - `Placeable`, with `width` and `height`.
  - `MeasureScope.layout(width, height) { ... }`, whose placement block runs at once and gives the node its size, and `place(x, y)` / `placeRelative(x, y)` on a `Placeable`, available only in the placement block (D5, D6).
  - `Dp.roundToPx()` in a measure scope.
- **The native side** (`prism-ui`) keeps the node tree, the layout pass, its cache and incremental relayout, text measurement, drawing, input and hit testing. A layout node's content is measured by calling its policy, a function value kept in the handler table and called through the function table, so hot reload swaps layout code (D8). Measuring and placing a child calls back into the native pass.
- **Measurables are lent**, as `&List<Measurable>` over a native buffer reused across passes, so calling a policy allocates nothing (D4).
- **`Column`, `Row`, `Box` and `Spacer` are Prism code** in `/prism/compose/layout`: capture-free policies over `Layout`, with arrangements, alignments and weights computed in Prism. The native `measure_line` and `measure_box` are removed (D9, D17).
  - Every dump, display list and script target stays byte-identical: the containers keep their node kinds (`column`, `row`, `box`, `spacer`), and their arrangement and alignment stay node properties (D10).
  - A user `Layout` is a node of the new kind `layout`.
- **Children a policy does not place** are neither drawn nor hit, nor script targets, and their dump lines end with `unplaced` (D7).
- **Relayout.**
  - A changed policy lays its node out again.
  - The climb from dirty nodes is unchanged: a node whose dirty children keep their sizes does not run its policy (D11).
  - After a hot reload, every node measured by a policy is measured again, and a frame is drawn only when a node moved or changed size (D12).
- **Misuse** is reported as `ui misuse:` runtime errors (D13):
  - measuring a measurable twice, or outside its parent's measurement;
  - placing outside a placement;
  - invalid constraints;
  - a policy returning without `layout`.

  Placing outside a placement block is also a compile-time `E4034`, because `place` needs the placement scope as its context.
- **`Int.toDouble()` and `Double.toInt()`** in `/prism/core`, with Kotlin's semantics, which Prism code needs for weights (D15).
- **Costs** (D18):
  - one policy call per measured layout node, and one entry call per child measured and per child placed;
  - no allocation in a layout of the toolkit's containers;
  - stated budgets for `ui_timing`, `app_frame_timing` (with a new full-relayout session) and startup.
- **The toolkit tour** gains a custom `Layout` (D19).
- **No new error code.**
- **BREAKING (small)**: `prism abi` of `/prism/compose/layout` and `/prism/compose/ui` changes. The tour's session snapshot gains its custom layout's nodes. Everything else keeps its output.

Not in scope, left for follow-ups (D20):

- intrinsic measurements (`IntrinsicSize`, `minIntrinsicWidth`, ...);
- `SubcomposeLayout` and `BoxWithConstraints`;
- alignment lines;
- parent data for user layouts (`Modifier.layoutId`, `ParentDataModifier`);
- observing state reads in measure policies;
- placement without measurement;
- right-to-left layout;
- `typealias MeasurePolicy`;
- the layout modifiers, which are `layout-in-prism`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `ui`:
  - the `Layout` primitive, constraints, measuring and placing;
  - measure policies kept by their nodes, and their misuse;
  - unplaced children;
  - the containers written over `Layout`;
  - the `layout` kind in dumps and script targets;
  - relayout and hot reload with policies;
  - the costs: calls per node, no allocation in layout.
- `standard-library`: `Int.toDouble()` and `Double.toInt()` as intrinsics of `/prism/core`.
- `execution`: the semantics of the two conversions.
- `example-apps`: the tour's custom layout, and a full-relayout budget for the 1,000-item app.

## Impact

- **`library/prism/compose/ui.prism`**: the primitive and its externals.
- **`library/prism/compose/layout.prism`**: `Column`, `Row`, `Box`, `Spacer` and the arrangement, alignment and weight logic in Prism, over private externals.
- **`library/prism/core.prism`**: two intrinsics. `crates/prism-typechecker/src/intrinsics.rs` adds two catalog entries, `crates/prism-typed-tree` adds two `BuiltinFunction` variants, and `crates/prism-codegen` lowers them to `fcvt_from_sint` and `fcvt_to_sint_sat`.
- **`crates/prism-ui`**:
  - `layout.rs`: the pass made re-entrant, with no tree borrow held while program code runs, measurement frames, lent measurable buffers, and the native containers removed;
  - `node.rs`: the `Layout` kind, the policy entry, `placed`, the measurement stamp;
  - `handlers.rs`: the `Measure` kind;
  - `tree.rs`, `setters.rs`, `entry.rs` and `functions.rs`: new entry points;
  - `runtime.rs`: calling a policy;
  - `draw.rs`, `input.rs`, `dump.rs` and `script.rs`: unplaced nodes and the `layout` kind;
  - `app.rs`: re-measuring after a reload;
  - `test_support.rs`: Rust policies for the engine tests.
- **Tests**:
  - `crates/prism-ui/tests/ui_timing.rs` and `allocations.rs`;
  - `crates/prism-cli/tests/app_frame_timing.rs` (resize session), `compile_timing.rs` (a `layout-policies` shape) and `library_tests.rs`;
  - fixtures under `crates/prism-cli/tests/fixtures/{ui,standard-library,execution}/`;
  - `examples/demo/ui/ui.prism` and its session.
- **Documents**: `openspec/specs/ui/spec.md`'s Design Rationale at archive, `docs/architecture.md` (the planned "Compose-style `Layout`" note), `ROADMAP.md` (item 11a) and `docs/milestone-2-review-notes.md` (A4).
- **Dependencies**: `vararg`, `core-library-in-prism` (extension properties with getters, which D16 uses), `field-visibility` (`package` fields, so handles cannot be forged) and `compose-library-in-prism` (`NodeId` as a library struct, `Node`/`LeafNode` and `runApp` in Prism) land first.
