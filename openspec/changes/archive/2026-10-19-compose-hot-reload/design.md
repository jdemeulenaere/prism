# Design: compose-hot-reload

## Context

See `proposal.md` for motivation. This is item 33 of "Then: the road to Milestone 2" in `ROADMAP.md`, the development-loop layer of `docs/architecture.md` §5. It implements `compose-architecture` D23 ("Hot reload recomposes the reloaded files' scopes") and the requirement "Hot Reload of Composables Keeps State", and refines both where the packets written since then make a better answer possible. It fills the seams that the earlier packets left for it:

- `hot-reload` D4: reload points for frame loops, and `AppliedReload` naming the reloaded units. Its D5 and D6 give slot contracts and 256-bit type fingerprints.
- `compose-runtime` D6 and D15: slots that check their type fingerprint on every read, so a key that matches the wrong group costs state but never memory safety. It also planned an `invalidate_where` over restart descriptors and a forced-pass flag for this change (D6 below drops both).
- `compose-transform` D5: one group per composable body, keyed by its declaration path, and region groups. Its D10 gives restart functions as static closures that read kept arguments from slots, and its D22 lists the seams for hot reload (D8 below corrects one of them).
- `state-handles` D11 and D12: cells owned by their `remember` slot, freed with it.
- `ui-toolkit` D15, D16 and D18: the frame loop, the reserved `edit` script command, and handlers owned by nodes, replaced on every pass by their setters.
- `desktop-window` D1 and D17: the window's `wake()` seam, and the watch session off the main thread.

The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Fast compilation is a primary criterion (user, 2026-10-19). This change adds no compiler stage and no analysis, so each decision states its compile-time cost, which is nearly nothing, and its run-time cost at reload time.

**Baseline.** This change was proposed while `hot-reload` was being reviewed and archived, and `standard-library`, `compose-runtime`, `compose-transform`, `state-handles`, `ui-toolkit` and `desktop-window` were proposed but not implemented. It needs all of them archived. It takes these names from their packets, which task 1.1 checks against the code they leave:

- **`hot-reload`** (implemented): `prism_runtime::reload` with `request`, `withdraw`, `apply_pending`, `apply_now`, `defer_to_reload_points(wake: fn())`, `apply_at_next_call`, `reload_point` and `AppliedReload { units, message }`. Also `prism_codegen::contract::{TypeFingerprints, slot_contract, BodyKind}`, `prism_codegen::reload::{PreparedReload, ReloadWrites, LiveProgram, SlotKind}`, and `crates/prism-cli/src/watch.rs::WatchSession` with `reload`, `applied_message`, `install_line_hook`, `run_script`, `EditScript` and `SessionEvent`. Then `run.rs::{KeptFiles, KeptFile, Build}` and `prism_mir::summary::{FileSummary, GenericSummary}`, and the `// watch:` header of `fixture_tests.rs`.
- **`compose-runtime`:** the `prism-compose` crate with its per-thread state (`thread.rs`), the group arena and `Group` records (`group.rs`), restart functions as owned `(Int) -> Unit` values, `invalidate`, the pass driver, deferred releases (its D7), and `invalidate_where` with the forced-pass flag (its D15, task 3.x).
- **`compose-transform`:** `keys.rs::{body_key, closure_key}`, `BodyOwner::Restart` and `BodyOwner::RootContent`, and `compose_file`.
- **`state-handles`:** cells attached to the slot whose calculation created them.
- **`ui-toolkit`:** `crates/prism-ui/src/{app.rs, host.rs, script.rs, headless.rs}`, `prism_ui_run_app`, `Host::next_event`, `HostEvent`, `ScriptCommand`, and the `// events:` and `// headless:` headers.
- **`desktop-window`:** `WindowHost::wake`, a plain `fn()`.

The deltas were written as ADDED requirements of `compose`, `ui` and `hot-reload`, which were not in the main specs yet; task 1.1 reconciled them (below).

**Baseline update (`standard-library`, applied 2026-10-18).** Library modules are graph nodes of origin `SourceOrigin::Library`, never read through the session's `SourceStore` or watched, and their units are never rebuilt or reloaded, since their keys cannot change during a session (`standard-library` R3). A program that calls `exitProcess` ends its run as `main` returning does: `prism_runtime::StopHandler` takes `(status, exited)`.

**Baseline update (`ui-toolkit` revision, user direction 2026-10-19).** `ui-toolkit` now follows Jetpack Compose's modules and APIs. What this design relies on changes as follows:
- Handlers given to `clickable`, `toggleable` and text fields live in `prism-ui`'s handler table, named by a `Copy` `HandlerId` in the modifier chain and claimed by the node that uses them (`ui-toolkit` D23); there is no `set_on_click`. The composable that builds the modifier registers a new entry on each of its passes, and a node keeps its entry only while the new one compares equal: same descriptor and equal environments. Code from a new module has new descriptors, so after a reload the author's pass gives the node the new handler, as D9 below requires.
- Modifiers are plain data and never run composition (`ui-toolkit` D25), so a reload has no modifier groups or slots to reset; this change plans no composed-modifier mechanism.
- The toolkit's modules are `/prism/compose/ui`, `foundation`, `layout` and `material3`, and the runtime is `/prism/compose/runtime` (`ui-toolkit` D21). Library units are never reloaded (above), so an edit reaches a toolkit composable only through the user code that calls it.

**Baseline reconciliation (task 1.1, 2026-10-19, after `desktop-window` was archived).** Every prerequisite is archived. Compared with the code and main specs they left:
- **As named:** `prism_runtime::reload::{request, withdraw, apply_pending, apply_now, defer_to_reload_points(fn()), apply_at_next_call, reload_point, AppliedReload { units, message }, PendingReload, ReloadPolicy}`, `stack::interrupt` and `INTERRUPT_PENDING`; `prism_codegen::contract::{slot_contract, BodyKind}` and `prism_codegen::reload::{PreparedReload, ReloadWrites, LiveProgram, SlotKind}`; `watch.rs::{WatchSession, SessionEvent, EditScript, run_script, run_watching, install_line_hook}`, with the session on a `watch-session` thread under windows (`prism_cli::windows::{run_with_windows, watch_with_windows}`) and ending when its workspace root is removed; `run.rs::{KeptFiles, KeptFile, Build}`; `FileSummary`, `GenericSummary` and their codec; `prism-compose`'s `thread::State`, `group::Group`, the pass driver `composition.rs::run_pass` and `Composition`; `keys.rs::{body_key, closure_key}`, `BodyOwner::{Restart, RootContent}` and the symbols `<F>#restart` and `<F>#root-content`; `prism-ui`'s `App::run` loop, `Host`, `HostEvent`, `ScriptCommand`, `parse_script`, `HeadlessHost` and the handler table (`handlers.rs`); `currentRecomposeScope()` in the compose prelude; `WindowHost::wake`, a plain `fn()` turned into a frame.
- **D8 is already adopted.** `compose-transform` hashes a restart body's contract with its composable's (`slot_contract(body, environment, restarted, fingerprints)`, tested by `a_restart_contract_covers_its_composable`), so task 3.2 only adds the scenario's `Int`/`Double` test. `PreparedReload::splits` still lists `#restart` splits, which task 3.3 leaves out of the notes.
- **`invalidate_where` and the forced pass landed** (`Composition::invalidate_where`, `Composition::recompose_forced`, `ActivePass::forced`), and `ROADMAP.md` lists their fate as a follow-up of `compose-runtime`. Task 4.4 removes them, as D6 says, and ticks that follow-up; the compose spec's Design Rationale (`compose-runtime` D15) is updated on archive.
- **Body keys go on the MIR body.** Summaries are computed in `prism-mir` from composed MIR, whose bodies no longer hold their regions, so a closure's key cannot be computed again from it. The transform records each composable body's key on the body itself, `Body::body_key: Option<u64>` (in the MIR codec, so records hold it and instances inherit it), and `FileSummary::of` collects them (task 3.1).
- **The CLI already accepts `--headless --events` with `--watch`**, configuring the host once before the session starts, and the fixture runner already passes `--watch --edits` together with `--headless --events`. Tasks 5.3 and 5.4 reduce to the script parser, `read_event_script`, which now takes whether the run watches, and the runner's header comment.
- **Hosts and the waker.** `prism-desktop` depends on `prism-ui` but not `prism-compose`; `WindowHost::new`, which already records the wake channel, registers `WindowHost::wake` with `prism_ui::host::set_reload_waker` (task 6.3). `--timings` reaches `prism-ui` through `prism_ui::host::set_reload_timings`, which `RunOptions::configure_ui` calls.
- **Spec deltas.** `compose`, `ui` and `hot-reload` are now main specs. The hot-reload requirement "Edit Steps Wait for the Reload Point of a Frame Loop" became a MODIFIED block of "Edit Triggers", and the `ui` requirement "Headless Runs Follow an Event Script", which lists the script's commands, is MODIFIED to name `edit` and `wait reload`. Every other requirement of the three deltas is new: none overlaps a main requirement's name or rule. `openspec validate compose-hot-reload --strict` passes. On archive it was split out again as the ADDED requirement "Edit Steps Wait for the Reload Point of a Frame Loop", since the merged "Edit Triggers" grew past the length `openspec validate --strict` accepts for one requirement.

**Apply notes (groups 2 to 6, 2026-10-19).** What implementing the runtime, compiler, session and frame loop settled:
- **`wait reload` lives in the frame loop**, not the headless host: `App::script` runs it, waiting on `prism-ui`'s condition variable (`host::wait_for_reload`) until the runtime's requested flag is set, so a reload requested before the command returns at once. It flushes the program's output first, so that a test or benchmark reading stdout sees the dumps before the loop blocks; without a line hook, output to a pipe is otherwise buffered until 8 KiB or the end of the run.
- **Scripted edits are parsed into one command per group.** `parse_script_with` takes a `ScriptContext` (whether the run watches, the script's directory, how to read a replacement); consecutive `edit` lines merge into one `ScriptCommand::Edit`, numbered by its first line. The session makes each target absolute, as edit scripts do, so it matches the watched paths.
- **Instance units name their generic through the build.** `Build::instance_generics` maps each planned instance's symbol to its generic, whose `GenericSummary::composable_bodies` the session adds (D4).
- **The `--timings` pass time includes layout**, since the frame loop's `settle` runs the passes and then lays out; it is what a reload costs the frame beyond compilation.
- **Groups 5 and 6 share a commit**: the script commands of task 5.3 are run by the frame loop of task 6.1, and neither builds alone.

## Goals / Non-Goals

**Goals:**
- Saving an edit of a composable shows the result in the running app at its next frame, with every remembered value and state cell whose type did not change kept.
- No reload can make code read a slot, a cell or an environment laid out for other code, whatever the edit, and no reload is refused.
- Work proportional to what a reload affects at reload time, and nothing added to compilation or to frames without a reload.
- Deterministic fixtures that mix UI events and edits, under the allocation check.
- Under 50 ms from save to frame on the counter app.

**Non-Goals:**
- Recomposing only the bodies whose code changed (per-body hashes), or following calls through function values (D4).
- Migrating state between types (`hot-reload` D7), resetting values whose calculation was edited (D7).
- Frame statistics and live counts (`reactive-app-verification`).

## Decisions

### D1. The boundary with the other changes (autonomous)

| Concern | `compose-hot-reload` (this change) | Elsewhere |
|---|---|---|
| Patching code, contracts, fingerprints, top-level values | the contract of restart bodies (D8) | `hot-reload` |
| Policy and wake-up | when `runApp` defers reloads, what wakes it, `after line` steps under reload points | the mechanism is `hot-reload` D4; the window's `wake()` is `desktop-window` D17 |
| Which scopes recompose | all of it (D3 to D6) | the runtime's invalidation and passes are `compose-runtime`'s |
| Slot-table state | the rule of D7, and resetting retired scopes' slots (D8) | type checks on every slot read are `compose-runtime` D6 |
| Handlers | when they are replaced (D9) | registering on every pass and claiming by nodes is `ui-toolkit` D23 |
| Scripts | `edit` and `wait reload` (D12) | the script grammar is `ui-toolkit` D16; edit scripts are `hot-reload` D16 |
| Measuring | the reload benchmark and the pass timing (D13) | `--frame-stats` is `reactive-app-verification`'s |

Rejected: also doing `reactive-app-verification`'s live counts here (they verify apps, not reloads).

### D2. The reload point is at the top of the frame loop; `runApp` defers reloads while it runs (autonomous)

`prism_ui_run_app` calls `reload::defer_to_reload_points(prism_ui::host::wake_for_reload)` before it starts its composition, and `reload::apply_at_next_call()` after it has dropped it. Its loop (`ui-toolkit` D15) gains one step at the top of every iteration:

1. **Reload point:** `reload::reload_point()`, which applies the pending reload, if any. The applied-reload hook (D3) invalidates the affected scopes.
2. While a scope is invalid, run a pass. While layout is dirty, lay out.
3. Otherwise block on `host.next_event()`, dispatch the event and its handlers, and go back to 1.

So a reload lands after the handlers of the previous event and before the pass that would show their effect. That pass runs the new code, and no pass, handler, layout or drawing ever sees half a reload. A handler that is running when the reload is requested finishes in its old code. Reloads requested during the first composition are applied at the first iteration.

`wake_for_reload` is called on the session's thread by `reload::request`. It notifies a condition variable of `prism-ui` that `wait reload` waits on (D12), and calls the waker the host registered with `host::set_reload_waker(fn())`: `WindowHost::wake` (`desktop-window` D17), which turns into a `HostEvent::Frame`, so an idle window reaches step 1 at once. The headless host registers none, because its events never block.

`apply_at_next_call` now raises the stack-limit interrupt when a reload is pending (`hot-reload` "Leaving Reload Points Applies a Pending Reload at the Next Call"). Without that, a reload requested in the last frame of an app would wait for the next `runApp`. `reload_point` first reads an atomic "requested" flag, so an iteration without a reload costs one load rather than the pending reload's lock.

Rejected:
- **Applying reloads at the next call while an app runs**, as `hot-reload` does for plain programs. A reload could then land in the middle of a pass, a handler or a layout. That is memory-safe (`compose-runtime` D15), but it would run half a pass in old code and half in new code, which can emit a group tree that neither version emits.
- **A reload point after recomposition, before layout.** The pass that shows a handler's effect would then run old code, and the reload would need a second pass.
- **Deferring only under `--watch`.** Without a session nothing requests reloads, so deferring costs nothing, and one path is simpler.

Compile-time cost: none. Run-time cost: one atomic load per frame-loop iteration.

### D3. Applying a reload invalidates through a hook; a reload during a pass is handled when the pass ends (autonomous)

`prism_runtime::reload` gains `set_applied_hook(Option<fn(&AppliedReload)>)`. `apply_pending` calls the hook on the program thread after the slot writes and the instruction barrier, and before it prints the reload's message. `Program::start` (`prism-codegen`) installs `prism_compose::reload::reload_applied`, as it installs `compose-runtime`'s composition report.

The hook does the work of D4 at once when no pass is active on the thread. When a pass is active, which happens only under the next-call policy (a plain `composition { }` whose body makes a call), it queues the `AppliedReload`'s keys in the thread state. The pass driver handles the queue after the pass's releases, so the walk never sees a group tree in the middle of its update, and the invalidated scopes recompose in the next pass (`compose-runtime` D4 already sends invalidations made during a pass to the next one).

The hook does not call program code, so `compose-runtime` D4's rule that no Rust reference is live while program code runs is kept. It runs under `apply_pending`'s lock, which only the session's `withdraw` and `request` take.

Rejected:
- **The frame loop calling `prism-compose` after `reload_point`.** Plain compositions under `--watch` would then never learn of reloads, and `prism-runtime` cannot depend on `prism-compose`.
- **Walking the tree in the middle of a pass:** the composer's scratch children are not yet committed there.

Compile-time cost: none.

### D4. A reload recomposes the nearest scope above every group of an affected composable body (autonomous; refines `compose-architecture` D23)

§5 D23 recomposes "every restart scope of a reloaded file". With `compose-transform`'s design, that misses three common cases:
- a composable that is not skippable, a `String` parameter's for instance, has no restart scope of its own; the scope above it is usually in its caller's file;
- a content lambda is never a restart scope (`compose-transform` D14), and the composable that receives it, `Column` for instance, is usually in another file;
- an edit of a plain helper function changes what a composable shows without changing the composable's unit, or even its file.

So the reload names *affected composable bodies*, and the runtime invalidates scopes from their groups:

- **Affected bodies.** The composable functions and composable closures whose code is in a unit the reload defines, file or instance. For an instance unit, these are the bodies of its generic. Also every composable body of a file that imports a reloaded file, directly or through other files. That covers every composable that can call reloaded code by name, plain helpers included. The session computes the set from the dependency graph's imports, reversed, which is linear in the program's files and imports, and from the kept summaries (D5).
- **Scopes.** For each live composition of the program thread, one pre-order walk over its groups, with an explicit stack that carries the nearest enclosing restart scope. When a group's key is an affected body's key, that scope, or the group itself when it is a restart group, is invalidated. The root counts as a restart scope, so a group with no restart scope above it invalidates the root. Retired scopes move the invalidation up (D8).
- **The pass.** The invalidated scopes recompose in tree order in the next pass (D6).

An affected scope above a non-skippable composable reruns its body, which calls the non-skippable composable again. A content lambda's author reruns, and with it the lambda. A helper's callers rerun. A file's importers are affected even when only a body changed, because a body edit does not change their check keys (`instance-cache`), and they may call it.

What this misses: a composable that reaches reloaded code only through a function value or a state it was given, from a file that does not import the reloaded one, and whose arguments did not change. It shows the new code at its next recomposition. Following function values would need a whole-program analysis.

Rejected:
- **The restart scopes of reloaded files only** (§5 D23 as written): the three cases above.
- **Recomposing every composition from its root, without skipping:** the simplest correct rule, but it reruns every body. A screen whose top composable is affected already recomposes from the top, while skippable subtrees in unaffected files still skip.
- **A call graph of composables and helpers,** to find exactly the bodies that can reach changed code: a whole-program analysis at every reload, against the compile-speed criterion.
- **Only the bodies whose code changed, through per-body hashes:** a later refinement of the first rule. It would still need the importers rule for helpers.

Compile-time cost: none. Reload-time cost: O(files + imports) for the importers, O(groups + affected bodies) for the walk, and the pass.

### D5. Groups are matched to code by their body keys, carried in summaries (autonomous)

The runtime does not know which unit a group's code comes from, and it should not learn it on every group start. But `compose-transform` D5 keys each composable function's body group `H("body", declaration path)` and each composable closure's by its parent's key and its step. Those keys are computed at compile time and fixed by the source structure. So the transform records them:

- `FileSummary::composable_bodies: Vec<u64>`: the keys of the file's composable bodies that are not part of a generic, functions and closures;
- `GenericSummary::composable_bodies: Vec<u64>`: those of a generic composable and its closures.

Both are in the summary codec, so check entries hold them, and a session that loaded a file from the cache has them without MIR. A body key is 64 bits. A collision with a region key is harmless: it only recomposes one scope more.

Rejected:
- **A unit word passed to every group start:** a word per group start, on every pass of every app, for a lookup only reloads need.
- **`compose-runtime` D15's `invalidate_where`** over restart descriptors' slots: it finds restart scopes only, so it misses the cases of D4.
- **Comparing restart descriptors with the address ranges of reloaded modules:** the same limitation, and modules are not contiguous.

Compile-time cost: one 64-bit push per composable body, already hashed by the transform, and 8 bytes per composable body in each summary.

### D6. No forced pass: an invalidated scope reruns with its force bit, and its children skip normally (autonomous; revises `compose-runtime` D15)

`compose-runtime` D15 planned a forced-pass flag that made `skipping` false for the whole reload pass. It is not needed, and it would cost recomposition:
- an invalidated scope recomposes through its restart function, which passes the force bit (`compose-transform` D10), and the root's adapter passes it too (its D15);
- an invalid group never skips (`compose-runtime` D8);
- every other skippable composable with unchanged arguments skips, which is right: its code did not change, or it would be affected and invalid itself.

So the reload pass is an ordinary pass over the invalidated scopes. The flag and `invalidate_where` are removed from `prism-compose` if they landed. Rejected: the forced pass (it reruns subtrees that D4 has just proven unaffected).

### D7. State is kept when its key and type match, reset when its type changed, and kept when its calculation was edited (autonomous; implements `compose-architecture` D23)

The reload pass reuses the groups whose keys match (`compose-runtime` D5), and every slot read checks the slot's type fingerprint (`compose-runtime` D6). Therefore:

- **Same key, same type:** the value is kept. A remembered `MutableState<Int>` keeps its cell and its count.
- **Same key, other type:** the read misses. The new code computes the value again and writes it, and the old value is forgotten and released at the end of the pass, with any cell its slot owns (`state-handles` D11). Fingerprints cover type arguments, so `mutableStateOf(0)` edited to `mutableStateOf("")` resets that state and no other.
- **No group:** a group no longer emitted is removed at the end of the pass, with its slots and nodes.
- **An edited calculation of the same type** (`mutableStateOf(0)` to `mutableStateOf(10)`) keeps the value. This is Compose's behavior under Live Edit and hot reload, and Flutter's for `State` objects. The reset is one restart away. A remembered value's whole point is to outlive the code that computed it.

Old handles held elsewhere, in a handler not yet replaced or a top-level value, reach a freed cell only through a generation or record mismatch, which is `state-handles`' "stale state handle" runtime error, never a misread. D9 makes sure handlers written by affected bodies are replaced first.

Slots are positional within their group (`compose-transform` D12), so inserting a `remember` of the same type before another shifts the later values by one. That is Compose's positional behavior too, and types keep it memory-safe.

Rejected:
- **Resetting a value when its calculation's code changed**, as `hot-reload` D8 resets top-level values with an edited initializer. It needs a hash per `remember` site in every slot (a word per slot and a hash per site at compile time), and it would lose live state on edits that only touch initial values, which is surprising in a UI being tuned. It is a possible opt-in (Open Questions).
- **Migrating values between types:** `hot-reload` D7.

Compile-time cost: none. Run-time cost: what the pass already does.

### D8. A restart function's contract covers its composable's; a retired scope recomposes from its parent and resets its own slots (autonomous; corrects `compose-transform` D22)

`compose-transform` D22 says that when a kept parameter's type changes, "old restart closures keep calling the old code". But a restart closure `F#restart` captures nothing and has type `(Int) -> Unit` whatever `F`'s parameters are. Under `hot-reload` D5 its contract never changes, so its slot is repointed to the new `F#restart`. The new restart closure would then read the old scope's kept slots by the new parameter list. A changed type would make `prism_compose_kept_slot` raise `composition misuse: no slot to read`. An added `Int` parameter would read a remembered `Int` as an argument and shift every later slot.

The fix has two parts.

- **The contract.** `contract.rs::slot_contract` hashes a restart body's contract together with the contract of the composable it restarts (`BodyKind::Restart`). When `F`'s parameter types change, `F#restart` splits along with `F`. An old scope keeps its old restart function, which would only ever run old `F` against slots that old `F` wrote, so it is consistent. A root content adapter's contract is fixed (it owns one `compose () -> Unit`), so the root never splits. The split of a restart function is not reported on its own: its composable's `signature ... changed` note says it.
- **The scope.** An old scope should still not stay on old code. `AppliedReload` gains `retired_slots`, the old slot addresses of the reload's splits (`PreparedReload::retired_slots`). In D4's walk, a restart scope whose restart function's descriptor names a retired slot is marked *reset*, and its invalidation moves to the nearest enclosing scope whose restart function is current, or the root. That scope reruns in new code, which calls the new `F` (every caller of `F` was compiled again in the reload, since `F`'s signature is in its ABI or its file). The new `F` starts its restart group, whose key is unchanged, so the group is reused. Seeing the reset mark, the runtime forgets all of the group's slots (released at the end of the pass) and clears the mark. Its child groups keep their state. The kept arguments are then stored again as on a first composition, and the body runs.

A composable whose new version is not skippable starts a replaceable group, which does not match the old restart group (`compose-runtime` D5 matches kind and key), so the old one is removed with its subtree. That is rare: it means adding a `String` or content parameter to a skippable composable.

Rejected:
- **Leaving restart contracts as they are:** the misreads above.
- **Recreating the whole group of a retired scope:** it loses its children's state for an edit of the parent's parameters.
- **A version word in each restart group, compared at each restart:** a compare on every restart, for what a contract split already tells.
- **Resetting nothing and letting typed slots catch it:** typed slots catch type changes but not shifted slots of one type (the added `Int`).

Compile-time cost: one more hash per restart body. Reload-time cost: one lookup per restart group in D4's walk, in a set of the reload's splits.

**Review note (2026-10-19).** A reload applied during a pass (a plain composition under the next-call policy) queues its walk until the pass ends (D3), but the rest of that pass may already run new code, which would start a retired scope's group without the reset mark and read the old code's slots by its new layout. So the hook also records the reload's retired slots for the rest of the pass, and a reused restart group whose restart function names one is reset when it starts, as the walk would have marked it. A restart group start checks this only while the list is not empty, so passes without a reload do no extra work (scenario "A scope whose signature changed during a pass is reset").

### D9. Handlers in nodes are replaced by the reload's pass, before the next event (autonomous)

A node owns its handler function values through `prism-ui`'s handler table (`ui-toolkit` D10, D23), which call their code through their descriptors' slots (`closures` D3):

- **Captures unchanged:** the closure's slot is repointed (`hot-reload` D5), so the old value runs the new body at the next click, even before any pass.
- **Captures changed:** the closure's slot splits, and the old value keeps the old code for its old environment. It is replaced when its author composes again: the author registers a new handler entry on every pass, which no longer compares equal to the node's (`ui-toolkit` D23), and `Button` is never skippable, because its content is a composable lambda. A skippable custom composable that takes a handler compares it by descriptor, and code from a new module has new descriptors, so it reruns too. The closure literal is in an affected body (its own), so its author's scope is invalidated (D4). The reload's pass therefore gives every node a new handler from new code, and it runs before the next event is dispatched (D2). No event reaches an old handler of a changed environment.

A handler made by a plain function is covered by D4's importers rule. One received from an unaffected file is not, which is the limitation D4 states. Handlers kept in state cells or top-level values follow `hot-reload`'s rule: old values call old code until they are replaced. The content closure that a running `runApp` was given is kept by its root for the whole app. An edit of its body reaches it through its slot, but a change of its captures takes effect when `main` runs again.

Rejected:
- **Re-pointing handlers' slots even when their captures changed:** new code over an old environment (`hot-reload` D5).
- **Dropping every handler at reload:** a click between the reload and the pass would find none, and the pass replaces them anyway.

Compile-time cost: none.

### D10. Layout and drawing after a reload are those of any pass (autonomous)

After the reload's pass, the frame loop lays out and builds the display list exactly as after a pass caused by an event. Setters mark dirty only properties that changed (`ui-toolkit` D3), and matched node groups keep their nodes (`compose-runtime` D9), with their numbers, focus and measurements (`ui-toolkit` D18). A window presents a frame only if the display list changed (`desktop-window` D11). The toolkit's composables are bundled library code, which a session never reloads, and layout and drawing are Rust code of `prism-ui`, so a reload changes only what user code passes to them. Rejected: relaying out the whole tree after a reload (no benefit, and it would hide which nodes changed).

### D11. Errors keep the old UI; a runtime error in new code ends the program (autonomous)

A reload with errors is never requested (`hot-reload` D12), so the app keeps running its previous code: its composition, nodes and handlers are untouched, events keep being handled, and the session prints the diagnostics and `reload: not applied; the program keeps running its previous code`. A runtime error in reloaded code, in the reload's pass or later, ends the program as every runtime error does. Under `--watch` that stops the program, not the session (`hot-reload` D13), and the next change that compiles runs `main` again in a fresh program.

Rejected:
- **Rolling back to the old code after a runtime error in new code:** there is no unwinding (`codegen-core` D12), and the composition's state after a partial pass is not known to be consistent.
- **Catching runtime errors per composable,** as error boundaries do: a later feature.

### D12. Scripted edits in event scripts: `edit` and `wait reload` (autonomous)

UI fixtures need edits at precise points between events. Counting output lines for an edit script's `after line` steps would tie every fixture to the length of its dumps. So `ui-toolkit`'s reserved `edit` command is defined, and `wait reload` is added:

- **`edit <target> <replacement>`**, under `--watch` only. The CLI reads the replacement when it parses the script (`ui-toolkit` D16), relative to the script's directory. Consecutive `edit` commands form one group. When the frame loop reaches a group, it calls `reload::request_scripted_edit(edits)`, a hook the session installs (`set_edit_hook`). The hook sends `SessionEvent::ScriptedEdit(edits)` to the session and blocks, as the line hook of `hot-reload` D16 does. The session overlays the edits, runs `reload`, which requests the reload if it compiled, and lets the program continue. The loop then goes to its top, where the reload point applies it (D2). A reload with errors leaves nothing pending, and the script continues on the old code. Without `--watch`, the parser reports `Error: <script>:<line>: the edit command needs --watch`, with status 1.
- **`wait reload`**, under `--watch` only. It blocks the headless host until a reload is requested, through D2's condition variable, at most 10 seconds, and then returns to the loop's top. Past the limit, it ends the program with `event script line <n>: no reload arrived within 10 seconds`. It serves tests that save real files (`run_cli_tests.rs`) and the benchmark (D13). The limit is a constant that a Rust test can shorten.
- **`after line` steps under reload points.** The line hook still stops the program and lets the session prepare the reload, but `apply_now` leaves the reload pending when reloads are deferred to reload points, so a `println` during a pass never applies a reload in it.
- **Fixtures.** A case with both `// watch: <edits>` and `// events: <events>` runs `prism run --check-allocations --watch --edits <edits> --headless --events <events> <entry>`. The edit script may hold only comments, which already parses, so the session runs scripted: it watches no files and ends when the app's `main` returns.

Rejected:
- **`after line` triggers only:** fixtures would count dump lines.
- **A separate `--ui-edits` script with event-count triggers:** two scripts to keep in step.
- **Edit commands without `--watch`, implying a session:** an option that changes the run's mode from inside a file.

Compile-time cost: none. Run-time cost: none outside scripts.

### D13. Latency: under 50 ms from save to frame on the counter app, measured (autonomous)

The target is a median under 50 ms on a release build, from the write on disk of a one-line edit of `Counter`'s body to the display list of the frame that shows it, debounce included. `hot-reload` measured 31 ms from the write to `reload: applied` for a 100-file program (its D17): about 12 ms of FSEvents delivery, the 10 ms debounce, 6 ms of front end and 1 ms of code generation. On the counter app, the front end is smaller, and the reload's pass, layout and display list are tens of microseconds (`ui-toolkit` D19). So the budget leaves about 15 ms of margin.

`crates/prism-cli/tests/ui_reload_timing.rs` is an ignored benchmark in the style of `reload_timing.rs`. It writes the counter app to a temporary workspace and runs `prism run --watch --timings --headless --events events` with a script of three clicks, then twenty rounds of `wait reload`, `frame` and `dump`. It rewrites `main.prism` twenty times, each time with a new label constant, and measures from each write to the dump that shows the new label. It prints the median and the 90th percentile, and fails on the target only under `PRISM_ASSERT_TIMING=1`. Measured on 2026-10-19 (release build, Apple silicon, two runs of twenty edits): from the write to the dump of the frame that shows the new label, a median of 30.0 ms and 29.8 ms, with a 90th percentile of 31.6 ms and 30.9 ms, of which about 20 ms run from the first file event to the reload being applied (the 10 ms debounce, the front end and code generation), about 10 ms are FSEvents' delivery, and the reload's pass and layout take 0.02 ms. The target holds with 20 ms to spare. With `--watch`, `--timings` also prints `timing: reload pass <ms> (<n> scopes)` after the pass that follows each reload, so a user sees what a reload cost beyond its compilation. A window adds rasterization, which `desktop-window`'s counters measure and this target does not include.

Rejected:
- **Measuring from the `edit` command:** it leaves out the file events and the debounce, which a user waits for.
- **A target per stage:** a user feels the whole reload.

### D14. Three capability deltas, no new code, and how the change is tested

The rules about slot-table state belong to `compose`, the frame loop's and the scripts' to `ui`, and those about contracts and reload policies to `hot-reload`. No error code is added: the new runtime error is a detail of `ui-toolkit`'s `EventScriptFailed`, and the script error a usage error.

Behavior a program can observe is pinned by executable fixtures under `crates/prism-cli/tests/fixtures/compose/`, `ui/` and `hot-reload/`. They are watch cases (`// watch:`), headless with event scripts where they show a UI, and run under `--check-allocations`, which also checks that reset cells and replaced handlers are freed. What only Rust can set up is pinned by Rust tests:
- the reload point inside a handler, and the shortened `wait reload`, in `prism-ui`;
- restart contracts, in `prism-codegen`;
- the walk's cost, in `prism-compose`;
- a pending reload when reload points end, in `prism-runtime`.

Saving real files under a waiting app is a `run_cli_tests.rs` test.

### D15. Body keys are recorded on composed MIR bodies (autonomous, apply time)

D5 has the transform record body keys for the summaries, but summaries are computed in `prism-mir` from composed MIR, in which a rewritten body no longer holds its regions, so a closure's key, chained from its parent's region, cannot be computed again there. The transform sets `Body::body_key: Option<u64>` on each composable function and closure it composes, the body's codec writes it, and `FileSummary::of` and the generic summaries collect the keys. Records hold it, so instances inherit their generic's keys. Compile-time cost: one word per body in memory and in records, written by the transform that hashed it. Rejected: a side table returned by `compose_file` (every caller of `compose_file` would pass it on to the summary), and scanning composed bodies for their first group start (it ties summaries to the prologue's shape).

### D16. `wait reload` writes out the program's output first (autonomous, apply time)

Output to a pipe is buffered until 8 KiB or the end of the run unless a line hook is set, which only edit scripts set. A test or benchmark that saves a file once an app's dump is printed would never see the dump, so `wait reload` writes out what the program printed before it blocks. This also gives tests a synchronisation point: a dump before `wait reload` arrives exactly when the app starts waiting, so the scenario "Saving a file under a waiting headless app" starts its script with a dump, and the benchmark with one. Run-time cost: one write per `wait reload`. Rejected: line-buffering all output under `--watch` (a write per line of every watched program), and a fixed sleep before saving (flaky on a loaded machine).

### D17. The pass after a reload compares every kept argument (autonomous, review time)

A skippable composable's caller passes two change bits per argument (`compose-transform` D4): `static` for a constant or a top-level value, the caller's own bits for a forwarded parameter, `uncertain` otherwise. `keep` compares only uncertain arguments; `same` and `static` ones read past the kept slot. That assumes the call site's code is the code that kept the value, which a reload breaks: an edit of `Label(5)` to `Label(6)` passes a static `6`, and an edit of `Inner(a, b)` to `Inner(b, a)` forwards `same` bits for swapped values. A composable in an unaffected file then skipped with its old arguments, and a later restart read them back from its kept slots, so the edit never showed.

So applying a reload marks every live composition, and its next pass compares every kept argument as an uncertain one, keeping the caller's bits when the value is equal. A reload applied during a pass also sets this for the rest of that pass (D3). Every call site of an affected body runs in that pass, since its scope is invalidated (D4), so afterwards every kept argument was written by current code and the bits are trusted again.

Rejected:
- **Clearing the static bit at compile time:** a comparison per argument on every pass of every app, for a case that only reloads create.
- **Comparing only at call sites in affected bodies:** the composer does not know which code a call comes from without a word per call (D5's rejected unit word), and forwarded bits reach callees in unaffected files.
- **Invalidating every callee's scope:** reruns skippable subtrees whose arguments did not change, which D6 avoids.

Compile-time cost: none. Run-time cost: one flag read per kept `same` or `static` argument, in the thread-state access `keep` already makes; in the pass after a reload, one equality call per such argument.

## Costs

| Stage | Compile-time cost |
|---|---|
| body keys in summaries (D5) | one push per composable body, 8 bytes per body in each summary |
| restart contracts (D8) | one more hash per restart body |
| everything else | none |

| Run-time operation | Cost |
|---|---|
| frame-loop iteration without a reload (D2) | one atomic load |
| affected bodies (D4) | O(files + imports) in the session, from kept summaries |
| finding scopes (D4, D8) | O(live groups + affected bodies), one walk per composition on the program thread |
| the reload's pass (D6) | the bodies of the invalidated scopes and their non-skipping children |
| kept arguments in the reload's pass (D17) | one equality call per `same` or `static` argument kept |
| reset slots (D7, D8) | released at the end of the pass, as any forgotten slot |

`crates/prism-compose/tests/runtime_timing.rs` gains the walk: compositions of 1,000, 10,000 and 100,000 groups with one affected body, asserting roughly linear growth under `PRISM_ASSERT_TIMING=1`. Measured on 2026-10-19 (release build, Apple silicon, `invalidating_after_a_reload`): a median of 16 ns, 13 ns and 8 ns per live group for 1,000, 10,000 and 100,000 groups, so 0.8 ms for the largest; the time per group does not grow. No compiler stage is added, so `compile_timing.rs` gains no shape. The summaries' growth is covered by its existing shapes.

## Risks / Trade-offs

- **[Importers make most reloads recompose from the top]** A screen file imported by `main.prism` makes `main.prism`'s composables affected. → Their skippable children in unaffected files skip (D6), so a reload costs one pass of the upper levels, far below the latency budget.
- **[Code reached through function values from unaffected files stays stale]** → Rare in UI code, where handlers are written in the composable that passes them. The next recomposition of that composable shows it, and a per-body-hash refinement can narrow D4 later.
- **[A retired scope loses its own remembered state]** Changing a skippable composable's parameters resets its own slots (D8). → It keeps its children, and a parameter change is a structural edit.
- **[Positional slots within a group]** Inserting a `remember` of the same type before another shifts their values (D7). → Compose behaves the same, and types keep it safe.
- **[64-bit body keys]** A collision recomposes one scope more. → Harmless by construction (D5).
- **[The content closure of a running `runApp`]** Its captures cannot change until `main` runs again (D9). → `runApp { App() }` captures nothing, and the split note says so.
- **[Prerequisites unimplemented]** Every name taken from the six packets comes from their designs. → Task 1.1 checks them against the code.

## Migration Plan

No program changes meaning without `--watch`. `runApp` defers reloads while it runs, which is invisible without a session. Summaries and check entries grow by the body keys, which changes the cache's entry format, so the compiler version in every key retires old entries. `compose-runtime`'s `invalidate_where` and forced-pass flag, if they landed, are removed (D6).

## Open Questions

- Should editing a `remember` calculation reset its value, as editing a top-level initializer does? D7 keeps Compose's behavior. An opt-in, or a restart command, can come later (follow-up).
- Per-body hashes would let D4 recompose only bodies whose code changed. `hot-reload`'s open question on reporting changed functions needs the same hashes (follow-up).
