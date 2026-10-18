# Proposal: hot-reload

## Why

Tier 1 was built for hot reload: every call goes through a patchable function table, nothing is inlined, each file is its own code unit, and `monomorphization` and `closures` give instances and closure bodies slots of their own (`codegen-core` D3 and D4). But nothing reloads yet. Each edit means stopping the program and running `prism run` again, which throws away everything it built up. Milestone 2's development loop (`compose-hot-reload`, `docs/architecture.md` §5, D23) needs a program that keeps running while its code changes. This change is item 25 of "Then: the road to Milestone 2" in `ROADMAP.md` and the Phase 3 item "Hot-reload runtime prototype". It also answers the questions earlier changes left for it:

- what happens to the values of top-level `val`s (`codegen-core` D24);
- what happens to live values of a type whose layout changed (`codegen-aggregates` D21);
- how long old code and its static literal data must stay mapped (`runtime-strings-and-drops` D3);
- what a function value made before a reload calls after it (`closures` D3, D4; compose D23).

## What Changes

- **`prism run --watch <file.prism>`** (D2, D13, D15). Runs the program as `prism run` does and keeps the `prism` process alive. It watches the program's files and, on a change, checks them again, compiles only what changed, and patches the running program without restarting it. When `main` returns, the session keeps watching, and the next change runs `main` again in a fresh program. A runtime error stops the program but not the session. A program that does not compile, at start or after an edit, prints its diagnostics and waits for the next change. Status lines on stderr start with `reload:` or `watch:`.
- **Incremental reloads** (D9, D14). The session keeps every file's source, syntax tree, check key and elaborated MIR in memory. A reload parses only the changed files and runs Phase 1 again over the program. It runs Phase 2 only for files whose check key changed, using the key `prism check` uses for its build cache, so a body edit re-checks one file. It compiles only the units whose code key in `instance-cache`'s build plan changed: those files' code units and the instance units whose key changed. It defines them into one new JIT module and links it against the program's single function table. Like `prism run` after `instance-cache`, the session reads and writes the build cache, so the code it compiles is reused by later runs.
- **Reloads are applied at the program's next call** (D3, D4). Compiling happens on the session's thread while the program keeps running. Applying happens on the program's own thread, which repoints every slot of a reload in one step at the next function entry. This costs nothing on the hot path: the session lowers the stack limit that every function's entry already checks, as Go does for preemption, and the check's cold path applies the reload and returns. Frame loops can defer reloads to explicit reload points, which `compose-hot-reload` uses to apply them between frames.
- **Slot contracts make every reload memory-safe** (D5, D6, D7). Each slot records the contract of the code it holds: the type fingerprints of its parameters and result, plus the environment layout for closure code. A fingerprint is a structural hash covering every definition a type reaches. A reload repoints a slot only to code with the same contract. When a function's signature, a type in it, or a closure's captures change, the symbol gets a new slot. New code calls the new slot. Running frames of old code, and function values made before the reload, keep calling the old one. No reload is ever refused for a layout change, and no live value is migrated. Old values stay with the old code that understands them.
- **Top-level values** (D8). A top-level value's storage belongs to the program, keyed by its symbol, its type fingerprint and a hash of its initializer. A reload that changes none of the three keeps the value. One that changes any of them gives the value new storage, initialized lazily at the next read as usual. Old storage, and anything still borrowing it, stays valid.
- **Old code stays mapped** (D10, D11). JIT modules are never freed while the program runs. They hold code that running frames, descriptors and drop entries refer to, and static literal data that strings point at. They are freed only when `main` runs again in a fresh program. A removed declaration's slot keeps its old code, so old callers still work.
- **Errors keep the previous code** (D12). A reload with errors of any stage, `E6001`, `E6002` and `E6003` included, prints the diagnostics as `prism check` would. The program keeps running its previous code, and the session waits for the next change. A newer change replaces a reload that has not been applied yet.
- **File watching** (D15). The session watches the program's files with the `notify` crate, falling back to polling. It registers the directories that hold the program's files and the targets of unresolved imports, coalesces events over a 10 ms quiet period, and ignores events that leave a file's content unchanged.
- **Deterministic tests** (D16). `prism run --watch --edits <script>` replaces file watching with a script. A step such as `after line 2: edit lib/math.prism lib/math.prism.v2` makes the file read as the replacement once the program has written its second line of output. The program waits at that point until the reload is applied. A step `after exit:` applies an edit after `main` returns, which runs `main` again. Edits only change what the session reads; no file on disk changes. Executable fixtures gain a `// watch: <script>` header. Their stdout and stderr cover output before and after each patch, under `--check-allocations`.
- **A latency target** (D17). Saving a one-line body edit to one file of a generated 100-file program should reach patched code within a median of 50 ms on a release build, including the debounce. An ignored benchmark, `reload_timing.rs`, measures this end to end. With `--watch`, `--timings` prints the stages of each reload.
- **Not in scope.** Compose (recomposing reloaded scopes, slot-table state, frame-loop wiring) is `compose-hot-reload`. Also not in scope:
  - migrating values between layouts, or re-running changed initializers eagerly;
  - reclaiming old modules while the program runs;
  - patching a running frame in place (on-stack replacement), so a running `main` loop keeps its own old body;
  - incremental Phase 1;
  - a Prism-level `hotReloadPoint()` built-in, a restart key, and watching from a non-entry directory;
  - Tier 2.

## Capabilities

### New Capabilities
- `hot-reload`: `prism run --watch` and its session. Covers when changes are detected and what a reload compiles, where and when it is applied, slot contracts and what old code keeps calling, top-level values across reloads, old modules, errors, runs after `main` returns or after a runtime error, scripted edits, and the latency target.

### Modified Capabilities
- `code-generation`: "Calls Go Through the Function Table" allows a symbol to get a new slot when its contract changes, and every slot keeps its address. "Top-Level Values Have Storage and an Accessor" gives storage to the program rather than to a code unit. A new requirement makes stack checks double as reload points.
- `execution`: "Program Arguments and Options" lists `--watch` and `--edits`. "Runtime Errors End the Program" says what a runtime error ends under `--watch`.

## Impact

- `crates/prism-codegen`: a new `contract.rs` (type fingerprints, slot contracts) and a new `reload.rs` (`LiveProgram`, `PreparedReload`). In `link.rs`: `Linker::with_table`, slots keyed by contract, and a value table for top-level storage. In `unit.rs`: storage imported, and units list their contracts and values. `lower.rs` and `glue.rs`: the stack check's cold path returns after a reload interrupt. `program.rs`: `Program::start` and a running-program handle. `plan.rs` (from `instance-cache`): comparing two build plans.
- `crates/prism-runtime`: a new `reload.rs` (pending reload, interrupt, reload policy, reload points). In `stack.rs`: the interrupt sentinel and `prism_rt_stack_check_failed`. In `panic.rs`: stopping the thread under watch. In `output.rs`: a line hook for `--edits`. In `functions.rs`: the new entry.
- `crates/prism-resolver`: `DependencyGraph::from_entry_with`, which reuses parsed files from a `SourceStore` and reads through an overlay.
- `crates/prism-abi`: `initializer_hash` for top-level values.
- `crates/prism-cli`: a new `watch.rs` (`WatchSession`, `FileWatcher`, `EditScript`, `SourceOverlay`), `run.rs` split into reusable stages, `main.rs` flags, and the `notify` dependency. Fixtures go under `fixtures/hot-reload/`, with a `// watch:` header in `fixture_tests.rs`. Also a watcher test in `run_cli_tests.rs`, the `reload_timing.rs` benchmark, and a contract-fingerprint shape in `compile_timing.rs`.
- `ROADMAP.md`: item 25 records the chosen shape, and the follow-ups of this change are added when it is archived. `docs/architecture.md` §2's Tier 1 hot reload scenario moves into the specs.
- No language change and no new error code.
- **Prerequisites:** `monomorphization`, `instance-cache`, `closures`, `escaping-closures` and `collections` are archived first (design, "Baseline"). `instance-cache` provides the build plan, code keys and relocatable cached units that a reload compares and reuses.
