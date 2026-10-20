# Proposal: layout-in-prism

## Why

This is the last item of "After Milestone 2: the review round" in `ROADMAP.md` (item 11b) and the end of agreed item **A4** of `docs/milestone-2-review-notes.md`. The user wants the libraries to be "as much as possible ... normal prism code that is imported like any other code", with "layout in Prism this round, as the last item" (2026-10-20).

`layout-primitive` (item 11a, proposed with this change, lands first) gives Prism Compose's `Layout` and `MeasurePolicy`, and moves `Column`, `Row`, `Box` and `Spacer` onto them. What stays native is the layout modifiers. `crates/prism-ui/src/layout.rs` still folds each node's modifier chain in Rust:

- `padding` deflates the constraints and offsets the rest;
- `width`, `height` and `size` make an axis exact;
- `defaultMinSize` raises a minimum;
- `fillMaxWidth`, `fillMaxHeight` and `fillMaxSize` take a fraction of the maximum.

In Jetpack Compose, each of these is a `LayoutModifierNode`, whose `MeasureScope.measure(measurable, constraints)` is library code that measures the rest of the chain and places it. Apps write their own with `Modifier.layout { measurable, constraints -> ... }`. Prism has neither: an app cannot write a layout modifier.

## What Changes

- **Layout modifiers are measurement steps written in Prism** (D1). Each layout element of a chain measures the rest of the chain, the elements after it then the node's content, under constraints it chooses, and places it, as Compose's `LayoutModifierNode.measure` does. Other elements are not steps: drawing (`background`, `border`, the text field frame), input (`clickable`, `toggleable`) and parent data (`weight`, `align`). They keep recording the bounds at their position, natively, as today.
- **The built-in steps** of `padding`, `width`/`height`/`size`, `defaultMinSize` and `fillMax*` are one Prism function of `/prism/compose/ui` that dispatches on the element (D2). `runApp` hands it to the runtime once per app, as a capture-free function value. Elements stay plain `Copy` data, as the user directed: no composed modifiers, no per-element closures.
- **`Modifier.layout { measurable, constraints -> ... }`** (Compose's `androidx.compose.ui.layout.layout`) in `/prism/compose/ui` (D3):
  - an element naming a step function registered in the handler table, as `clickable` names its handler;
  - compared by code and captures, so a modifier with an equal step stays stable;
  - dumped as `layout`.
- **`Modifier.offset(x: Dp = 0.dp, y: Dp = 0.dp)`** in `/prism/compose/layout`, Compose's offset: it places the rest of the chain at an offset without changing the size, and is dumped as `offset(<x>,<y>)` (D4).
- **Measuring names a position in a chain** (D5). `Measurable`, `Placeable`, `MeasureScope`, `PlacementScope` and `MeasureResult` gain a `package` element index:
  - a child's measurable starts at the first element of its chain;
  - a step's measurable starts at the element after the step;
  - the lent measurables become two words each.
- **The native fold shrinks** to walking the chain, calling steps, recording the bounds of drawing and input elements, and measuring text (D6). Its padding, size, minimum-size and fill code moves to `test_support`, where the engine's tests use it as Rust steps (D7).
- **A step must place the rest of its chain.** Not placing it is `ui misuse: a layout modifier did not place its content` (D10).
- **Hot reload** measures again every node with a `Modifier.layout` step, as `layout-primitive` does for policies (D8).
- **Behaviour is unchanged.** Every dump, display list and session keeps its output, except the toolkit tour, which gains `offset` and `Modifier.layout` (D11).
- **Costs**: one step call per layout element per measurement, and no allocation. Budgets in D9.
- **No new error code.**

Not in scope, left for follow-ups (D12):

- stateful user-defined `LayoutModifierNode`s, which need interfaces;
- `wrapContentSize`, `requiredSize`, `sizeIn`, `widthIn` and `heightIn`;
- `aspectRatio`;
- `absoluteOffset` and the lambda form `offset { IntOffset(x, y) }`;
- `padding(PaddingValues)`;
- intrinsic measurements through modifiers.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `ui`:
  - layout modifiers as measurement steps;
  - `Modifier.layout`;
  - `offset`;
  - steps that do not place their content;
  - reloads of steps;
  - no allocation in steps.

## Impact

- **`library/prism/compose/ui.prism`**:
  - the element index in the handles;
  - `ModifierElement.Layout(HandlerId)` and `Offset(Int, Int)`;
  - `Modifier.layout` and its registration external;
  - the built-in steps and their dispatcher;
  - `runApp` passing the dispatcher.
- **`library/prism/compose/layout.prism`**: `Modifier.offset`.
- **`crates/prism-ui`**:
  - `layout.rs`: steps in the chain fold, frames naming an element, and the native padding, size and fill code removed;
  - `modifier.rs`: the two elements, their decoding and dump text, and the raw element words kept per chain entry;
  - `handlers.rs`: the `LayoutModifier` kind;
  - `entry.rs` and `functions.rs`: `prism_ui_run_app`'s new parameter and `prism_ui_register_layout_modifier`;
  - `app.rs`: the reload rule;
  - `dump.rs`, `draw.rs` and `input.rs`: offset bounds;
  - `test_support.rs`: Rust steps.
- **Tests**: `crates/prism-ui/tests/allocations.rs` and `ui_timing.rs`, `crates/prism-cli/tests/app_frame_timing.rs`, fixtures under `crates/prism-cli/tests/fixtures/ui/`, and `examples/demo/ui/ui.prism` with its session.
- **Documents**: `ROADMAP.md` (item 11b and item 11) and `docs/milestone-2-review-notes.md` (A4 done).
- **Dependencies**: `layout-primitive` and everything it depends on (`vararg`, `core-library-in-prism`, `field-visibility`, `compose-library-in-prism`).
