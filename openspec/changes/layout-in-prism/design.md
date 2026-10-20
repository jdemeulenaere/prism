# Design: layout-in-prism

## Context

See `proposal.md` for the motivation. This change is item 11b of "After Milestone 2: the review round" in `ROADMAP.md`, the second half of the layout part of agreed item **A4** of `docs/milestone-2-review-notes.md`:

- "as much as possible ... normal prism code that is imported like any other code" (user, 2026-10-20);
- "layout in Prism this round, as the last item" (user decision, 2026-10-20).

The orchestrator relayed the user's direction for this item:

- follow Compose;
- compile speed stays primary;
- no composed modifiers: modifiers stay plain data, and layout modifiers become elements like Compose's `LayoutModifierNode` whose measure logic is Prism code.

The user is away. Every decision the user might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Decisions the user took in the review are marked **(user direction 2026-10-20)**. Each decision states its compile-time and run-time cost (AGENTS.md §5).

**Baseline.** Proposed on top of `f35a446` ("Propose compose-library-in-prism"), together with `layout-primitive`, which lands first, after `vararg`, `core-library-in-prism`, `field-visibility` and `compose-library-in-prism`. This design is written on `layout-primitive`'s design:

- the measurement protocol of its D6: frames, stamps, measure once, `layout` with immediate placement;
- its lent measurables (D4) and Rust test policies (D17);
- its rule that a reload measures policy nodes again (D12);
- its budgets (D18).

The `ui` block "Layout and Drawing After a Reload" is modified here on `layout-primitive`'s version. Task 1.1 rebases every block once `layout-primitive` is archived.

**Baseline update (`layout-primitive` reviewed and archived, 2026-10-20).** What task 1.1 should know besides the archived design (`openspec/changes/archive/2026-10-20-layout-primitive/design.md`):

- `Context::measure` is gone (its D21): a measurement is steps of one state borrow each (`measure`, `start`, `start_full`, `complete`, `finish` in `layout.rs`) over a `Fold` of the chain passed by reference.
- Sizes are clamped to `geometry::MAX_EXTENT` (2,147,483,647) after the content and each padding in `finish`, modifier sizes and paddings when decoded, and `place` positions (its D23, requirement "Layout Sizes and Positions Are Bounded"); Prism measurement steps must keep that bound, or the native side must clamp what they return.
- The containers call the package externals directly, and `Measurable.measured()` and `prism_ui_arrangement` were replaced by `prism_ui_measured(node): MeasuredChild` (size and own alignment) and `prism_ui_line_properties` (its D25).
- `App::settle` lays out again, up to eight times, while a layout's policies invalidated scopes (its D24).
- At archive, the main text of "Constraint Layout in Logical Pixels" and "Dumping the Laid-Out Tree" was shortened under 500 characters, with the same rules.

**Current state**, once `layout-primitive` has landed:

- `Context::measure` (`crates/prism-ui/src/layout.rs`) folds a node's chain in two loops over at most 16 elements:
  - forward, `Padding` deflates and offsets, `Size` and `FillMax` make axes exact (clamped), and `DefaultMinSize` raises zero minimums;
  - then the content is measured, natively for texts and the window, by the node's policy for the five policy kinds;
  - backward, `Padding` adds itself to the size, clamped into its constraints, and drawing and input elements record their bounds (`ChainEntry::bounds`) at their offset with the size of the rest.
- Chains come from Prism as `Modifier` values, decoded by `ModifierMirror` (`modifier.rs`) into `Element`s. `ModifierElement` is a `Copy` enum of `/prism/compose/ui` with the variants `Padding`, `Size`, `DefaultMinSize`, `FillMax`, `Weight`, `Align`, `Background`, `Border`, `Clickable`, `Toggleable` and `TextFieldFrame`, its fields hidden by `field-visibility`.
- Handler elements (`Clickable`, `Toggleable`) name handler table entries that their node claims.

## Goals / Non-Goals

**Goals:**

- Every layout rule of the toolkit is Prism code; the native pass keeps the tree walk, the cache, the protocol, the bounds of drawing and input elements, and text measurement.
- Apps write layout modifiers with Compose's `Modifier.layout { measurable, constraints -> ... }`, and offset content with `Modifier.offset`.
- Modifiers stay plain `Copy` data compared by value; no modifier composes or remembers.
- Every existing output byte-identical; no allocation in layout; budgets stated.

**Non-Goals:**

- Stateful modifier nodes, intrinsics through modifiers, and the other Compose layout modifiers (D12).

## Decisions

### D1. Each layout element is a measurement step (user direction 2026-10-20: Compose's `LayoutModifierNode`)

A node is measured as a series of steps, Compose's chain of layout modifier nodes ending in the node's content:

- **Measuring from element `k`** of a chain under constraints `c`:
  - when element `k` is a **layout element** (`Padding`, `Size`, `DefaultMinSize`, `FillMax`, `Offset`, `Layout`), its step runs: a Prism function called with a `MeasureScope` naming `(node, k)`, the measurable `(node, k + 1)` for the rest of the chain, and `c`. It measures the rest once, under constraints it chooses, and returns `layout(width, height) { rest.place(x, y) }`;
  - otherwise, element `k` is passed through natively, and measuring continues from `k + 1` under `c`;
  - past the last element, the content is measured as `layout-primitive` measures it.
- **Sizes.** A step's result is clamped into `c` (`layout-primitive` D6), and is the size of the chain from `k`.
- **Positions.** Placing `(node, k + 1)` at `(x, y)` sets the offset of the rest of the chain inside step `k`. The content's origin, and the bounds that drawing and input elements record, are the sums of these offsets, computed once the node's measurement is done (at most 16 elements).
- **A child's measurable** is `(child, 0)`, the whole node. The node-level cache (`layout-primitive` D11) applies to it alone. Steps inside a node run whenever the node is measured, as the native fold runs today.

**Compile-time cost.** None: library code.

**Run-time cost.** Per layout element per measurement:

- one call through the function table;
- one measure entry call;
- two `layout` entry calls;
- one place entry call.

Non-layout elements cost what they cost today.

Rejected:

- **Keeping the built-in layout elements native** and adding only `Modifier.layout`: the user asked for layout in Prism, and Compose's padding and size are library nodes.
- **Folding steps through an accumulated constraint transform** (each step a pure constraints-to-constraints function): cheaper, but not Compose's model. `offset` and `Modifier.layout` need to place the rest, not only constrain it.

### D2. The built-in steps are one capture-free Prism dispatcher given to the runtime by `runApp` (autonomous)

The data stays where it is: `ModifierElement.Padding(start, top, end, bottom)`, `Size`, `DefaultMinSize` and `FillMax`. Their steps are written once, in `/prism/compose/ui`:

```kotlin
package fun MeasureScope.measureElement(
    element: &ModifierElement,
    measurable: Measurable,
    constraints: Constraints,
): MeasureResult = when (element) {
    ModifierElement.Padding(start, top, end, bottom) -> {
        val inner = measurable.measure(constraints.offset(-(start + end), -(top + bottom)))
        layout(
            constraints.constrainWidth(inner.width + start + end),
            constraints.constrainHeight(inner.height + top + bottom),
        ) {
            inner.place(start, top)
        }
    }
    ModifierElement.Size(width, height) -> ...
    ModifierElement.DefaultMinSize(width, height) -> ...
    ModifierElement.FillMax(width, height, fraction) -> ...
    ModifierElement.Offset(x, y) -> ...
    else -> prism_ui_not_a_step()
}
```

**How the runtime gets it.** `runApp` (Prism since `compose-library-in-prism` D10) passes `{ element, measurable, constraints -> measureElement(element, measurable, constraints) }` to `prism_ui_run_app` as a fifth parameter, `attached MeasureScope.(&ModifierElement, Measurable, Constraints) -> MeasureResult`.

- The app keeps it for its life, calls it through its slot for every built-in step, and releases it when the app ends.
- It captures nothing, so it costs nothing per pass and allocates nothing.
- The native side passes the element's own Prism value by address. Each chain entry keeps the raw words `ModifierMirror` read for it (D11).

**Arithmetic.** Each step ports today's native rule exactly:

- `Padding` uses `constraints.offset` (`deflated`: floored at 0, `Infinity` kept);
- `Size` makes each specified axis exact at `constrainWidth(width)` (clamped);
- `DefaultMinSize` raises a zero minimum to `min(value, maximum)`;
- `FillMax` makes each bounded axis exact at `(maximum.toDouble() * fraction)` rounded down, clamped, 0 for NaN.

**Where the code lives.** Compose declares `PaddingNode` and `SizeNode` in `compose-foundation-layout`. Here the steps sit in `/prism/compose/ui` beside `ModifierElement`, since one dispatcher must see every variant and `runApp` lives in `compose-ui`. The modifier functions (`padding`, `size`, ...) stay in `/prism/compose/layout`. The `ui` spec's design rationale records this deviation.

**Compile-time cost.** About 80 lines of library code. **Run-time cost.** One dispatch (`when` over the tag) per step.

Rejected:

- **A function value per element** (each `padding(...)` call registering a closure over its four values): an allocation and a registration per element per pass, and a comparison through program code for every chain.
- **One registration per element kind at app start** (a table of five steps): five function values instead of one, and more entry points, for the same calls.
- **Calling a library function by symbol from `prism-ui`**: the runtime would hard-code a library path, and slots resolved by symbol are a hot-reload mechanism, not an API.

### D3. `Modifier.layout { measurable, constraints -> ... }` (user direction 2026-10-20: follow Compose)

In `/prism/compose/ui`:

```kotlin
public fun Modifier.layout(
    measure: attached MeasureScope.(Measurable, Constraints) -> MeasureResult,
): Modifier = append(ModifierElement.Layout(HandlerId(prism_ui_register_layout_modifier(measure))))
```

- **Signature.** It is Compose's: a measurable by value, since `Measurable` is `Copy`.
- **Ownership.** The step function is a handler table entry of the new kind `LayoutModifier`. The node whose chain names it claims it, exactly as `clickable`'s handler (ui-toolkit D23), with the same interning, duplication and misuse rules. The native side calls it directly for that step, through its slot.
- **Equality.** The element compares by the handler ids' equality, so a chain with an equal step (same code, equal `Copy` captures) is equal, and a composable taking it skips ("Modifiers Are Stable Parameters").
- **Dumps.** The element is dumped as `layout`.
- **`attached`**, as a handler, since the node keeps it.

**Compile-time cost.** None beyond an ordinary call. **Run-time cost.** Per pass, one registration and comparison (as `clickable`). Per measurement, one step call.

Rejected: **`Modifier.layout` as a composable that remembers its lambda**: the user's rule against composed modifiers.

### D4. `Modifier.offset(x, y)` (autonomous)

In `/prism/compose/layout`, Compose's `offset(x: Dp = 0.dp, y: Dp = 0.dp)` appends `ModifierElement.Offset(x.value, y.value)`.

- Its step measures the rest under the same constraints, takes its size, and places it at `(x, y)`.
- So the elements after it, the content and the children are drawn and hit at the offset, while the node's size and its position in its parent do not change.
- It is dumped as `offset(<x>,<y>)`.

It is in this change because it is the smallest layout modifier that only places, which the native fold could not express, and Compose code uses it often.

**Compile-time cost.** None. **Run-time cost.** One step.

### D5. Handles name a position in a chain (autonomous)

`Measurable`, `Placeable`, `MeasureScope`, `PlacementScope` and `MeasureResult` gain `package val element: Int`, the index of the chain position they start at.

- A child's measurable has element 0. The content's position is the chain's length.
- The lent measurables of `layout-primitive` D4 become two words each, and the `#[repr(C)]` mirrors follow, checked against `prism dump-layout` as before.
- The frame stack of `layout-primitive` D6 holds `(node, element)` frames. Every entry check compares the pair: measuring `(n, k + 1)` is accepted only inside the step frame `(n, k)`, and measuring `(c, 0)` only inside a frame of `c`'s parent's content.

**Compile-time cost.** None. **Run-time cost.** One more word per handle, and one more comparison per entry call.

### D6. The native fold walks steps; text stays native (autonomous)

`Context::measure` becomes a walk from element 0:

- non-layout elements are skipped, with their indices kept for recording bounds;
- a layout element pushes a frame and calls its step (D2 or D3);
- the content is measured at the end.

Once the node's measurement returns, one backward loop computes each position's offset from the placements and records the bounds of drawing and input elements. That is the same loop as today, with offsets from placements instead of from padding.

The native `Padding`, `Size`, `DefaultMinSize` and `FillMax` arms are removed from production code. Text measurement, text breaking and the window's measurement stay native.

**Run-time cost.** The walk stays linear in the elements. Its state is on the stack: offsets for at most 16 positions.

### D7. The engine's tests use Rust steps (autonomous)

`test_support` gains a Rust element measurer, `extern "C"` closure code porting today's native arms, given to test apps as the app's dispatcher. So `prism-ui`'s unit tests keep running through the step protocol: the layout tests with paddings and sizes, `incremental_relayouts_match_full_layouts`, `ui_timing` and `allocations`.

### D8. Reloads measure nodes with program steps again (autonomous)

`layout-primitive` D12 extends: after an applied reload, the next layout also measures again every node whose chain holds a `Layout` element (D3), since its step is program code.

- Built-in steps are library code, which a reload never changes.
- The display list is rebuilt only when a node's bounds changed, as in `layout-primitive`.

**Run-time cost.** None beyond the full layout that `layout-primitive` D12 already runs.

### D9. Costs and budgets (autonomous)

**Compile time.**

- No new stage. `Modifier.layout` calls are ordinary calls with a lambda with a receiver, which the `layout-policies` shape of `layout-primitive` already measures.
- The library grows by about 120 lines.

**Run time.**

- Per layout element per measurement: one step call and four entry calls (D1).
- The 1,000-item todo app has about four steps a row (the button row's `defaultMinSize` and `padding`, the spacer's `width`, the checkbox's `size`), so a full relayout makes about 4,000 step calls besides `layout-primitive`'s policy calls.
- No allocation: the dispatcher captures nothing, elements are passed by address, and frames and offsets reuse their buffers.

**Budgets**, measured before (task 1.2) and after (task 6.3) on an idle machine and a release build:

- every target of `layout-primitive` D18 still holds:
  - the resize session's `layout` p99 under 4 ms, with no allocation;
  - the paced sessions' targets;
  - startup within its targets;
  - `ui_timing`'s ratios;
- the paced toggle and typing sessions' `layout` medians may grow by at most 0.05 ms over this change's baseline;
- the resize session's `layout` median may grow by at most 1.5×.

### D10. A step must place the rest of its chain (autonomous)

A step that returns without placing its measurable ends the program with `ui misuse: a layout modifier did not place its content`. Measuring it twice, placing it outside the step's placement, and calling `layout` twice keep `layout-primitive`'s misuse errors.

Compose lets a layout modifier skip placing its content, which then is not drawn. Supporting that would make "unplaced" a property of a position inside a node, for every drawing and input element and for dumps. The strict rule can be relaxed later without breaking a program.

**Run-time cost.** One flag test per step.

Rejected: **Compose's silent hiding**, for the cost above, while no toolkit modifier needs it.

### D11. Elements keep their raw words; dumps are unchanged (autonomous)

- `ModifierMirror` decodes each element into `Element` for drawing, input and dumps, as today.
- Each chain entry also keeps the element's raw words, its `ModifierElement` value as Prism lays it out, so the dispatcher can receive it by address. `ChainEntry` grows by the element's size, at most 16 per node.
- Two variants are added to the decoding: `Offset` (`offset(x,y)`) and `Layout` (`layout`, a handler element).

Every existing dump line keeps its text. The tour gains `offset` and `Modifier.layout`, so its snapshot changes (the toolkit tour requirement asks for every modifier kind).

### D12. Deferred (autonomous)

Each becomes an unchecked follow-up in `ROADMAP.md` at archive:

- **Stateful modifier nodes** (Compose's own `LayoutModifierNode` subclasses keeping state between measurements): they need interfaces or a node-state protocol.
- **`wrapContentSize`, `requiredSize`, `sizeIn`/`widthIn`/`heightIn` and `aspectRatio`**: more steps of the dispatcher.
- **`absoluteOffset` and `offset { IntOffset(x, y) }`**, the lambda form that reads state in placement only.
- **`padding(PaddingValues)`**.
- **Letting a step leave its content unplaced** (D10).
- **Intrinsic measurements through modifiers** (with `layout-primitive`'s intrinsics follow-up).

## Risks / Trade-offs

- **[Frame time]** Native arithmetic becomes calls. → Budgets in D9, measured by the resize session of `layout-primitive`.
- **[Positions from placements]** The bounds of drawing elements now come from placements. → Every existing ui fixture and session keeps its byte-identical output, including "Order decides what a background covers", "The clickable area follows the chain" and "Padding before or after a size". The randomized incremental test runs through the Rust steps.
- **[Raw element words]** The runtime passes Prism's own element bytes back to Prism. → The mirror test of `layout-primitive` task 3.2 covers `ModifierElement`'s layout too.
- **[The dispatcher's home]** Steps of foundation-layout modifiers live in `compose-ui` (D2). → Recorded as a deviation from Compose's module split in the `ui` spec's rationale.

## Migration Plan

Programs keep their meaning. The library changes:

- `ui.prism` gains the element index, two element variants, `Modifier.layout`, the dispatcher, and `runApp`'s fifth argument;
- `layout.prism` gains `offset`.

The runtime gains the `LayoutModifier` handler kind and `prism_ui_register_layout_modifier`, `prism_ui_run_app` takes the dispatcher, and the native layout arms move to `test_support`. Rollback: revert the commits.

## Open Questions

- Whether user code should be able to write stateful layout modifier nodes before Prism has interfaces, for instance as a struct of state plus a step function.
