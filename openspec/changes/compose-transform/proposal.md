# Proposal: compose-transform

## Why

A `compose fun` parses, type-checks and lowers to MIR today, and `compose-runtime` gives it a composer, a slot table and restart scopes, but no composable written in Prism can run: the backend rejects every composable body with `E6001`, and nothing turns a composable into the calls the runtime expects. This change is item 28 of "Then: the road to Milestone 2" in `ROADMAP.md`, the compiler half of the composition layer planned in `docs/architecture.md` §5 (`compose-architecture` D3 to D5 and D8 to D11). It is a MIR pass that gives composables their calling convention, groups, positional memoization, skipping and restart scopes, the `/prism/compose/runtime` prelude with `remember`, `key` and an owned `Composition`, and the backend support that makes them run. `state-handles`, `ui-toolkit` and `compose-hot-reload` then build on it without changing how composables are lowered.

## What Changes

- **The compose transform** (D2). A new pass over each file's elaborated MIR, after borrow checking and before generic records, instances and code generation, so a generic composable is transformed once and its instances substitute the composed MIR. It runs in `prism check` too, since records hold composed MIR. `prism dump-mir --composed` prints its output; `--composed --instances` prints composed instances.
- **The calling convention** (D3, D4). Every composable function, composable closure and composable function type takes, after its declared parameters (the receiver first), the composer, the identifier word of the current pass (`compose-runtime` D3), and change words: two bits per parameter (uncertain, same, different, static), 31 parameters per 64-bit word, and a force bit. Callers pass static for constants, forward what they know of their own parameters, and pass uncertain otherwise. Defaults stay caller-evaluated, so no default bits are passed.
- **Groups and keys** (D5, D6). Each composable body runs in one group, a restart group when it is skippable and a replaceable group otherwise, keyed by the body's identity: its declaration path, or for a composable closure the lexical path from its top-level declaration to the call it is an argument of. Calls get no group of their own: equal keys match in order. Each branch, conditional operand and loop iteration that contains a composable call runs in its own replaceable group keyed by its position in the source structure. Keys are 64-bit BLAKE3 hashes chained from the declaration path, never from offsets, lines or columns, so comments, argument edits and inserted calls of other composables keep them. MIR lowering marks those regions, and ends them on every `return`, `break` and `continue`.
- **Stability, skipping and restart scopes** (D7 to D11). A parameter is stable when its type is `Copy` and borrow-free by its declared markers (a type parameter only through a `Copy` bound), or an `escaping` non-composable function type whose value, at run time, is *keepable*: it captures nothing or owns only `Copy` captures. A composable returning `Unit` whose parameters are all stable is skippable. It keeps its parameters in its restart group's slots, compares them through each type's equality glue, skips its body when nothing changed and its scope is valid, and is a restart scope whose restart function, a static closure that allocates nothing, recomposes it alone from the kept values. Every other composable, and every composable lambda, is not a restart scope, as §5 D10 plans.
- **Keepable closures** (D9). Closure descriptors grow from two words to four: an environment equality entry and an environment duplicate entry, non-null for closures that capture nothing or own only `Copy` captures, so a handler such as `{ count.value += 1 }` can be kept and compared without a clone.
- **`/prism/compose/runtime`, a prelude module** (D12 to D16). `remember<T : Copy>(calculation)`, with up to three `Copy` keys compared by `==`; `key(value: Int) { }` for movable groups; `currentRecomposeScope()` and `RecomposeScope.invalidate()`, Kotlin's API, which lets fixtures invalidate scopes before `state-handles` exists; and the owned `Composition`: `composition { }` composes its content once, `c.setContent { }`, `c.recompose(): Bool`, `c.dump(): String`, and dropping it disposes everything it holds.
- **Intrinsic structs** (D16). `intrinsic struct` declarations in bundled files, laid out and given glue by the compiler from its catalog: `Composition` (one word, neither `Copy` nor `Clone`, disposed by its drop) and `RecomposeScope : Copy`. `state-handles` declares `State<T>` the same way.
- **What is rejected** (D17). Starting, recomposing or setting the content of a composition inside a composable context is the new `E4030` (`CompositionInComposable`). Default values of composables are not composable contexts, so a composable call there is `E4008`. Remembering a non-`Copy` value or key is `E4021`. `E6001` no longer names composable functions.
- **The backend** (D19, D20). Composer operations become calls of `compose-runtime`'s entry points; each unit emits a static slot type record per kept or remembered type (fingerprint, size, alignment, drop and equality glue, name); composable machine signatures carry the composer and change words. One runtime entry is added: reading a restarted scope's kept slots.
- **Seams** (D22, D23). `state-handles` gets the restart scopes, the read-only calculation window of `remember` and the static composable-context flag it needs; `compose-hot-reload` gets keys that survive body edits and restart closures in the composable's own unit.
- **Costs** (D24). One linear pass per composable body, once per body rather than per instance; one hash per group; one marker lookup per parameter type. Shapes in `compile_timing.rs` pin linear growth.
- **Not in scope.** State handles, reads that register scopes and writes that invalidate them (`state-handles`); nodes, `Node` and the toolkit (`ui-toolkit`); recomposing reloaded scopes (`compose-hot-reload`); composable calls in default values; keys other than `Int` for `key`; stable shared borrows of `Copy` values; effects and subcompositions (`compose-architecture` D25).

## Capabilities

### New Capabilities
None. The language side of compose is added to the `compose` capability that `compose-runtime` creates.

### Modified Capabilities
- `compose`: composables in compositions, the owned `Composition`, `E4030`, the compose prelude, body groups, keys, region groups, `remember` and its keys, `key`, stable parameters, skipping, restart scopes, `currentRecomposeScope`, composable lambdas, releasing kept values, reading a restarted scope's kept slots, and the transform's cost.
- `mir`: group markers in lowered composable bodies, composed MIR, `prism dump-mir --composed`, and generic composables composed once.
- `code-generation`: composable machine signatures, composer operations, slot type records, keepable closure descriptors, intrinsic struct layouts, and the `E6001` scenarios that named composables.
- `type-system`: default values are not composable contexts.
- `standard-library`: `/prism/compose/runtime` joins the prelude; intrinsic structs; `remember` is a rule-typed intrinsic.
- `syntax`: `intrinsic struct` declarations.
- `compilation-and-abi`: intrinsic structs in the ABI.
- `formatting`: `prism fmt` prints intrinsic structs.

## Impact

- New crate `crates/prism-compose-transform`: the pass (`lib.rs`, `keys.rs`, `stability.rs`, `body.rs`, `calls.rs`, `intrinsics.rs`, `restart.rs`).
- `crates/prism-mir`: group markers and regions (`build/groups.rs`, `build/scope.rs`, `build/control_flow.rs`, `build/closure.rs`), composer operations, `TerminatorKind::Remembered`, `Rvalue::Bits`, `MirPhase::Composed`, new body owners, dump, validation, instantiation and records.
- `crates/prism-borrowck`: group markers are no-ops.
- `crates/prism-typechecker`: the `/prism/compose/runtime` catalog entries, intrinsic structs, `remember`'s rule, `E4030`, defaults outside composable contexts.
- `crates/prism-parser`, `crates/prism-ast`, `crates/prism-abi`, `crates/prism-formatter`: `intrinsic struct`.
- `crates/prism-codegen`: composer operations, slot type records, four-word descriptors with environment equality and duplicate functions, intrinsic struct layouts and glue, `E6001` lifted for composables.
- `crates/prism-compose`: `prism_compose_kept_slot`.
- `crates/prism-library`: `prism/compose/runtime.prism`, in `PRELUDE`.
- `crates/prism-cli`: the pass in Phase 2, `dump-mir --composed`, `composed-mir.snap` fixtures, compose fixtures, compile-timing shapes.
- Specs: the deltas above; the diagnostics catalog (`E4030`, `E4008` and `E6001` reworded) edited directly. `docs/architecture.md` §5: "Composable Calling Convention", "Groups Keyed by Declaration Path and Lexical Position", "Positional Memoization with remember", "Keyed Movable Groups", "Skipping Composables with Stable Parameters", the language half of "Restart Scopes and Invalidation" and of "Compositions Are Owned Values" move out on archive.
- **Prerequisites:** `compose-runtime` is archived first, which brings `closures`, `escaping-closures`, `standard-library` and that change's own baseline (`monomorphization`, `instance-cache`, `collections`, `hot-reload`). See `design.md`, "Baseline".
