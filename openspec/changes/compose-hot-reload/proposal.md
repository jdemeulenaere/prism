# Proposal: compose-hot-reload

## Why

`hot-reload` keeps a program running while its code changes: `prism run --watch` patches the function table at the program's next call, splits slots whose contract changed, and keeps top-level values. But it knows nothing of compositions. After a reload, a running app keeps showing what the old code composed until some state write happens to recompose the right scope, and nothing decides what happens to remembered state whose type changed, to restart functions of scopes whose composable's signature changed, or to handlers that nodes hold. The first-class compose and hot reload invariant asks for more: "the Tier 1 JIT swaps composable bodies while keeping UI state". This change is item 32 of "Then: the road to Milestone 2" in `ROADMAP.md`, the development-loop layer of `docs/architecture.md` §5 (`compose-architecture` D23, "Hot Reload of Composables Keeps State"). It is built on the seams the earlier packets left for it: `hot-reload`'s reload points and fingerprints (its D4, D6), `compose-runtime`'s fingerprinted slots (its D6, D15), `compose-transform`'s keys from declaration paths and static restart closures (its D5, D10, D22), `ui-toolkit`'s frame loop and reserved `edit` command (its D15, D16, D18), and `desktop-window`'s wake-up (its D1, D17).

## What Changes

- **Reloads land between frames** (D2). While `runApp` runs, reloads are deferred to a reload point at the top of each frame-loop iteration: after the handlers of the previous event have returned, and before the loop recomposes. A window host wakes the loop for a pending reload, and leaving `runApp` makes reloads apply at the next call again, a pending one included. Outside `runApp`, reloads keep `hot-reload`'s next-call rule. A reload that lands during a pass of a plain composition is handled when the pass ends (D3).
- **The scopes a reload recomposes** (D4, D5, D6). Applying a reload marks invalid the nearest restart scope above every group of an *affected* composable body. A body is affected when its code is in a unit the reload defines, file or instance, or when it is in a file that imports a reloaded file, directly or not, so that an edited helper function reaches the composables that may call it. Bodies are identified by the group keys `compose-transform` already gives them, carried in file summaries. Finding the scopes is one walk over the live groups, linear in their number. Invalidated scopes then recompose in an ordinary pass, through their restart functions, which run their bodies with the force bit. Their children skip as usual when their arguments did not change.
- **What state survives** (D7). A group whose key still matches keeps its slots. A remembered value or state cell whose type fingerprint still matches is kept, so editing a label keeps a counter's count. One whose type changed is reset: dropped at the end of the pass, its cell freed, and its calculation run again. A group no longer emitted is removed. Editing a `remember` calculation without changing its type keeps the value, as in Compose.
- **Restart functions after a signature change** (D8). A restart function's slot contract now covers its composable's parameter types, so when they change the restart function gets a new slot along with the composable. No old restart function can then run new code against slots laid out by old code. A scope whose restart function was retired this way is recomposed from the nearest enclosing scope whose restart function is current. Its own kept arguments and remembered values are reset, and its children keep their state. This corrects `compose-transform` D22, which assumed that a restart closure capturing nothing would split by itself.
- **Handlers held by nodes** (D9). A handler whose captures did not change runs its new body at its next click, through its repointed slot. One whose captures changed keeps its old code until its node is given a new handler. The reload's pass does that for every handler written by an affected body, so no event reaches an old handler of a changed environment after the reload.
- **Layout and drawing** (D10). After the reload's pass, layout and the display list run as after any pass: setters mark dirty only what changed, nodes keep their identity, focus and layout caches, and an app shows a new frame only if something visible changed. The toolkit's own code is bundled and never reloaded.
- **Errors** (D11). A reload with errors keeps the old UI running and responsive, with the diagnostics on stderr, as `hot-reload` does for every program. A runtime error in new composable code ends the program as any runtime error does. Under `--watch` that stops the program, not the session.
- **Scripted UI edits** (D12). Under `--watch`, a headless event script may hold `edit <target> <replacement>`, applied as one reload between two events, and `wait reload`, which waits for a reload from the file watcher. Without `--watch`, `edit` is a script error. An edit script's `after line` steps are applied at the next reload point while an app runs. Fixtures combine `// watch:` and `// events:` headers.
- **A latency target** (D13). On the counter app, a release build should go from saving a one-line composable edit to the frame that shows it in a median under 50 ms, debounce included. An ignored benchmark, `ui_reload_timing.rs`, measures this. With `--watch`, `--timings` also reports each reload's pass.
- **Not in scope.**
  - Recomposing only bodies whose code changed, through per-body hashes (a refinement of D4).
  - Calls of reloaded code through function values received from unaffected code, which update at their next recomposition (D4, D9).
  - Resetting a remembered value when its calculation is edited, and a restart command.
  - Reloading the content closure of a running `runApp` whose captures changed (D9).
  - `--frame-stats` and live counts of groups, cells and nodes (`reactive-app-verification`).
  - Tier 2, which has no reload.

## Capabilities

### New Capabilities
None. `compose`, `ui` and `hot-reload` are created by the packets this change builds on and are not in the main specs yet, so their deltas here are ADDED requirements.

### Modified Capabilities
- `compose`: the requirements on what a reload recomposes, which state survives it, retired restart functions, and reloads that land during a pass.
- `ui`: the reload point of the frame loop, handlers and layout after a reload, the old UI after a reload with errors, runtime errors in reloaded code, the `edit` and `wait reload` script commands, and the latency target.
- `hot-reload`: the contract of restart functions, and how edit scripts and pending reloads behave while reloads are deferred to reload points.

## Impact

- `crates/prism-runtime/src/reload.rs`:
  - `AppliedReload` gains the affected composable bodies' keys and the addresses of the slots the reload retired;
  - an applied-reload hook, `set_applied_hook`;
  - `apply_now` leaves a reload pending under reload points, and `apply_at_next_call` raises the interrupt for a pending one.
- `crates/prism-compose`:
  - `reload.rs` with `invalidate_reloaded`: the walk over live groups, retired scopes, and deferral to the end of a pass;
  - a reset-slots flag on groups;
  - compose-runtime's `invalidate_where` and forced-pass flag removed, if they landed.
- `crates/prism-compose-transform` and `crates/prism-mir`: `FileSummary::composable_bodies` and `GenericSummary::composable_bodies` (the body keys of each file's and each generic's composable bodies), in the summary codec.
- `crates/prism-codegen`:
  - `contract.rs`: a restart body's contract covers its composable's;
  - `reload.rs`: `PreparedReload` reports the old slots of its splits, and restart and root-content splits get no note;
  - `program.rs` installs the applied-reload hook.
- `crates/prism-ui`:
  - `app.rs`: the reload point, deferring reloads while `runApp` runs, and `--timings` of the reload's pass;
  - `script.rs` and `headless.rs`: `edit` and `wait reload`;
  - `host.rs`: the wake-up registered with the runtime.
- `crates/prism-desktop`: `WindowHost::wake` registered as the reload wake-up.
- `crates/prism-cli`:
  - `watch.rs`: the affected bodies from the import graph and the kept summaries, scripted edits from event scripts, and `after line` steps under reload points;
  - `main.rs` and `run.rs`: `--events` under `--watch`;
  - `fixture_tests.rs`: `// watch:` together with `// events:`;
  - fixtures under `tests/fixtures/compose/`, `ui/` and `hot-reload/`;
  - the ignored benchmark `tests/ui_reload_timing.rs`, and a test in `run_cli_tests.rs` that saves a file on disk under a headless app.
- No language change and no new error code.
- `docs/architecture.md` §5: "Hot Reload of Composables Keeps State" and the `edit` command of "Headless Rendering and Scripted Input" move into the specs on archive.
- **Prerequisites:** `hot-reload`, `standard-library`, `compose-runtime`, `compose-transform`, `state-handles`, `ui-toolkit` and `desktop-window` are archived first (design, "Baseline"). Only the window wake-up (task 6.3) needs `desktop-window`.
