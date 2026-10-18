# Tasks

Fixtures go under `crates/prism-cli/tests/fixtures/hot-reload/`, one case per scenario, as the header of `fixture_tests.rs` describes. The cases are named after the scenarios' slugs (`a-body-edit-compiles-one-file`, ...) and run as watch cases (task 6.3) under `--check-allocations`. Scenarios whose WHEN describes a Rust test or a real file save are tested where task 7 says, not as fixtures.

Prerequisites (design, "Baseline"): `monomorphization`, `instance-cache`, `closures`, `escaping-closures` and `collections` are archived. The instance units, the build plan and its code keys, relocatable unit artifacts, descriptors, closure symbols and owned environments named below come from them. Where `instance-cache`'s design names them differently than this file, follow it. Before starting, compare the two MODIFIED blocks of each delta (`code-generation`: "Calls Go Through the Function Table" and "Top-Level Values Have Storage and an Accessor"; `execution`: "Program Arguments and Options" and "Runtime Errors End the Program") with the main specs and reconcile any drift.

## 1. Runtime: interrupts, reload hand-off, watch-mode panics (D3, D4, D13, D16)

- [x] 1.1 In `crates/prism-runtime/src/stack.rs`, add `INTERRUPT: usize = usize::MAX`, a `REAL_LIMIT` static holding the limit that `set_stack_limit` computes, and `pub fn interrupt()`, which stores `INTERRUPT` into `STACK_LIMIT`. Add `#[no_mangle] pub extern "C" fn prism_rt_stack_check_failed() -> u8`. When `STACK_LIMIT` holds `INTERRUPT`, it restores `REAL_LIMIT` first, then calls `reload::apply_pending()`, and returns 0 if the stack pointer is above the real limit, 1 otherwise. Without the interrupt it returns 1. Register it in `RUNTIME_FUNCTIONS` (`functions.rs`), whose declaration test keeps the table and the signatures in step. Verify with unit tests: an interrupt with no pending reload returns 0 and restores the limit, and a real overflow returns 1.
- [x] 1.2 Add `crates/prism-runtime/src/reload.rs`. It holds `PendingReload` (a boxed `FnOnce() -> AppliedReload + Send` and the message to print), a `Mutex<Option<PendingReload>>`, and `ReloadPolicy::{AtNextCall, AtReloadPoints}`. Add these functions:
  - `request(pending)`, which replaces any pending reload and returns it, and under `AtNextCall` calls `stack::interrupt()`;
  - `withdraw()`, which takes the pending reload back for D12's superseding;
  - `apply_pending()`, which flushes the output, runs the reload, prints its message to stderr, and executes `isb` on aarch64 (`std::arch::asm!`);
  - `defer_to_reload_points(wake: fn())`;
  - `reload_point() -> Option<AppliedReload>`.

  Verify with unit tests: a request replaces an earlier one, and under `AtReloadPoints` no interrupt is raised and `reload_point` applies the request.
- [x] 1.3 In `panic.rs`, add `set_watch_mode(on_stop: fn())`. Under watch mode, `report` flushes and writes as today, then calls `on_stop` and parks the thread forever (`loop { std::thread::park() }`) instead of `process::exit`. Make sure no runtime lock is held there: `report` already takes the stderr lock locally, and `flush` releases the output lock. `prism_rt_unlinked_function` goes through `report` too. Verify with a unit test in a child thread that the thread parks and `on_stop` runs.
- [x] 1.4 In `output.rs`, add a line hook, `set_line_hook(Option<fn(u64)>)`, called with the newline count of the current run after each newline is appended. The count resets in `start`. Call the hook outside the `STATE` lock, after flushing, so the session can stop the program there (D16). Verify with a unit test counting the lines of `prism_rt_print_newline` calls.

## 2. Backend: stack checks, fingerprints, contracts, value table (D3, D5, D6, D8)

- [ ] 2.1 In `crates/prism-codegen/src/lower.rs::check_stack` and `glue.rs::check_stack`, make the overflow branch go to a cold block. That block calls `prism_rt_stack_check_failed` (declared through `UnitContext`'s runtime imports), raises "stack overflow" as today when the result is 1, and jumps back to the body otherwise. Keep the fast path unchanged. Verify with a `lowering_tests.rs` test of the IR shape (the scenario "The cold path of a stack check"), and with the existing stack-overflow fixtures of `execution`, which must still pass.
- [ ] 2.2 Add `crates/prism-codegen/src/contract.rs` with `TypeFingerprints`, a per-load memo from `Type` to a 256-bit `ContentHash` (D6), built over a unit's `TypeTable`. Scalars, `String` and `Range` are hashed by tag. References, nullables, boxes and lists by tag and contents. Function types by parameters, result and the compose flag. Structs and enums by qualified name, markers, and their fields' or variants' names and fingerprints with type arguments substituted, using a back-reference for a definition already on the stack. Use `prism_abi::hasher::DeterministicHasher`. Verify with unit tests:
  - `Int` and `Double` fields differ;
  - adding a field changes a struct's fingerprint;
  - a recursive enum through `Heap` terminates;
  - `Box<Int>` and `Box<Int?>` differ;
  - the same type in two units has the same fingerprint.
- [ ] 2.3 In `contract.rs`, add `SlotContract` and `slot_contract(body, fingerprints)`: the body's kind (function, default, initializer, accessor or closure), its parameter and result fingerprints, and for a closure its captures in order with mode and fingerprint, plus the environment's representation (inline or heap, from `escaping-closures`). Make `CodeUnit` (`unit.rs`) record a contract for each function it lowers (`LoweredFunction::contract`). Carry the contracts, and task 2.4's value records, in the relocatable unit artifact of `instance-cache` (`artifact.rs`) and its codec, so that a unit taken from the cache has them. Verify with unit tests: a renamed parameter keeps the contract, an added parameter changes it, and a changed capture changes a closure's.
- [ ] 2.4 Move top-level value storage out of units (D8). In `unit.rs`, declare `#storage` and `#state` with `Linkage::Import`, and record in `CodeUnit::values` each value's path, size, alignment and `value_key`. The key is a hash of the symbol, the type fingerprint, and the initializer hash the caller supplies through a `ValueKeys` map; without one, for example for `prism dump-clif`, the hash of the symbol alone. In `link.rs`, add a `ValueTable`, which allocates zeroed, aligned storage and a state byte per (symbol, value key) in chunks that never move, and resolve `#storage` and `#state` through the symbol lookup. Verify with the fixtures "Reading an imported top-level value" and "Storage is imported by the unit", and with every existing top-level value fixture of `execution`, which must keep passing.
- [ ] 2.5 In `crates/prism-abi`, add `pub fn initializer_hash(declaration: &ValueDecl) -> ContentHash`, a span-free hash of the declared type and initializer expression built on `hash_expr_span_free` (`signature.rs`). Verify with unit tests: reformatting and moving the declaration keep the hash, and a changed literal changes it.
- [ ] 2.6 In `link.rs`, give `FunctionTable` a contract per slot and a symbol-to-current-slot map. `slot_for(symbol, contract) -> (slot, SlotChange)` returns the current slot when the contracts are equal or the symbol has none (`SlotChange::Repointed` or `New`). Otherwise it allocates a fresh slot, maps the symbol to it and returns `SlotChange::Split`. Keep `slot(symbol)` for the JIT's lookup, resolving to the current mapping. Add `Linker::with_table(table, values)`, which builds a new `JITModule` over an existing table and value table, registering the runtime symbols as `Linker::new` does. Add a split-safety debug assertion: every unit in a load that refers to a split symbol is in that load. Verify with the Rust test of "A repointed slot keeps its address", and a unit test where a different contract yields a new slot while the old slot keeps its code address.
- [ ] 2.7 Add `crates/prism-codegen/src/reload.rs` with `LiveProgram`, which owns the `Arc<FunctionTable>`, the value table and `modules: Vec<JITModule>`, and `PreparedReload`, which holds the slot writes, the splits (symbol, kind), the module, and the names of the reloaded units. `LiveProgram::prepare(units: &[&MirFile], instances: &[InstanceUnit], keys: &ValueKeys) -> Result<PreparedReload, BackendError>` lowers and finalizes into a new module via `Linker::with_table` and does not write any slot. `PreparedReload::into_pending()` gives the runtime closure that writes the slots on the program thread. `LiveProgram::adopt(module)` keeps an applied module, and a withdrawn reload's module is dropped. Make `Program::compile` and `Linker::link` build a `LiveProgram` for the first load, so one path serves both. Verify with the Rust test of "An interrupt does not end the program": compile a looping program, start it, prepare a reload of `value`, request it, and check that `main` returns with status 0.
- [ ] 2.8 In `program.rs`, split `Program::run` into `Program::start(main, arguments, check_allocations, events: Sender<ProgramEvent>) -> RunningProgram`, with `ProgramEvent::{MainReturned(i32), Stopped}`, and a `RunningProgram::join`. `run` becomes `start` plus `join`, unchanged for `prism run`. Verify with the existing executable fixtures.
- [ ] 2.9 In `crates/prism-cli/src/plan.rs` (from `instance-cache`), add `BuildPlan::changed_since(&self, running: &BuildPlan) -> Vec<UnitId>`, which lists the file units and instance units whose code key is new or different, in topological order with instances after files and sorted by symbol (D9). Verify with unit tests: a body edit lists one file unit, a generic body edit lists the file and its instances and no importer, and a field added to a type argument lists that type's file and the instances over it.

## 3. Front end reuse (D9, D14)

- [ ] 3.1 In `crates/prism-resolver/src/graph.rs`, add a `SourceStore`: per absolute path, its content, content hash and parsed `SourceFile`, with an optional overlay map from path to content. Add `DependencyGraph::from_entry_with(entry, store: &mut SourceStore)`, which runs the worklist of `from_entries` but takes content from the store: the overlay first, then a cached entry not invalidated since, then disk. It parses only content whose hash changed. `SourceStore::invalidate(paths)` marks paths to read again. `from_entries` keeps its behaviour by using a fresh store. Verify with unit tests: an unchanged file is not parsed again (count parses), an overlay hides disk content, and a new import adds a node.
- [ ] 3.2 Make the check key and the code keys computable without a cache directory, for `--no-cache` sessions: in `crates/prism-cli/src/cache.rs` and `plan.rs`, factor the key functions out of `BuildCache` (`check_key_hash(compiler, module, source, imports)` and its code-key counterparts) and use them in both places. Verify with the existing `keys_cover_every_input` test.
- [ ] 3.3 In `crates/prism-cli/src/run.rs`, split `compile_program` into stages the session reuses. `front_end(graph) -> Vec<(PathBuf, MirFile)>` elaborates in topological order. `back_end_checks(files, entry) -> main` covers `find_main` and `check_file`. Linking stays as it is. `compile_program` keeps its timings and behaviour. Verify with the existing run tests.

## 4. The watch session (D2, D9, D12, D13)

- [ ] 4.1 Add `crates/prism-cli/src/watch.rs` with `WatchSession`. It owns the `SourceStore`, per file a `LoadedFile` (check key, summary, `MirFile`, its unit's contracts and value keys), the running `BuildPlan`, the build cache (unless `--no-cache`), the `LiveProgram`, the `RunningProgram` and a `Sender`/`Receiver` of `SessionEvent::{FilesChanged(Vec<PathBuf>), Program(ProgramEvent), LineWritten(u64)}`. `WatchSession::start(entry, options)` compiles everything as `compile_program` does, records the keys, and starts `main`. On compile errors it prints them and `watch: waiting for changes`.
- [ ] 4.2 Implement `WatchSession::reload(changed) -> ReloadOutcome`, following D9's steps:
  - invalidate the changed paths in the store and rebuild the graph;
  - run `extract_abis`;
  - compute check keys with `check_key_hash`, and run `elaborate_file`, `find_main` and `check_file` for changed keys only;
  - build the program's plan from the summaries and take `changed_since` the running plan (task 2.9);
  - for each changed unit, take its artifact from the build cache or build and compile it (instance units from their records), writing new artifacts back;
  - compute value keys with `initializer_hash` for the values of re-checked files;
  - call `LiveProgram::prepare`.

  On any error, print the diagnostics with `WorkspaceError::render` and then `reload: not applied; the program keeps running its previous code`, and keep the kept state unchanged. Commit the new keys and MIR only when the reload is applied. Verify with fixtures for "A body edit compiles one file", "A signature change compiles its importers", "A generic body edit compiles its instances" and "A type error keeps the previous code".
- [ ] 4.3 Build the reload's message. It holds `reload: applied <files>` (topological order, relative to the workspace root, with ` (<n> instance[s])`) and one note per split, as "Contract Changes Are Reported" states. Notes are printed only while a program runs. Hand the message to `reload::request` so that it is printed when the reload is applied. Verify with fixtures for "Reporting a changed signature" and "Reporting changed captures".
- [ ] 4.4 Supersede pending reloads (D12). Before preparing, `reload::withdraw()` any pending reload, drop its module, and compute the new reload against the last applied state. Verify with a Rust test of `WatchSession` for "Two saves before the program calls anything", using a program held at a line hook.
- [ ] 4.5 Runs after `main` returns and after runtime errors (D13). On `ProgramEvent::MainReturned(status)`, print `watch: main returned with status <status>; waiting for changes`. On `Stopped`, print `watch: the program stopped with a runtime error; waiting for changes`. At the next change that compiles, drop the `LiveProgram`, build a fresh one from the cached and kept artifacts (compiling only changed units), print `watch: running main again`, and start `main`. Install `panic::set_watch_mode` before the first start. Verify with fixtures for "Editing after main returns", "Fixing a division by zero" and "A runtime error under watch".
- [ ] 4.6 In `crates/prism-cli/src/main.rs::run_program`, accept `--watch` and `--edits <script>` before the file, with `--edits` without `--watch` being a usage error, and update the usage text and `print_usage`. Add `watch: bool` and `edits: Option<PathBuf>` to `RunOptions`, and dispatch to `watch::run_watch`. Under `--timings`, print after each reload `timing: reload <ms> (detect <ms>, front end <ms>, code generation <ms>, linking <ms>, applied after <ms>)` and `timing: live modules <n>`. Verify with fixtures for "Edits without watching" and "Timing a reload" (that case's `stderr.snap` cannot hold times, so the runner checks it with the line-pattern assertion `run_cli_tests.rs` uses for "Reporting timings").

## 5. File watching (D15)

- [ ] 5.1 Add `notify = "8"` to `crates/prism-cli/Cargo.toml` (record the version and licence in the commit message). In `watch.rs`, add `FileWatcher`, which watches each directory holding a program file or an unresolved import's target non-recursively, re-registers them after each reload, filters events to those paths, debounces (10 ms of quiet, at most 100 ms after the first event) and sends `SessionEvent::FilesChanged`. If the native watcher fails, fall back to `notify::PollWatcher` at 100 ms with the stderr line `watch: file events are unavailable; polling every 100 ms`.
- [ ] 5.2 Ignore events whose content hash is unchanged (`SourceStore`). Verify with `run_cli_tests.rs` tests that edit files on disk under `prism run --watch`, each with a 20-second timeout:
  - "Saving an edited body while the program runs";
  - "Saving without changes does nothing": rewrite the same content, then make a real edit, and check that stderr holds exactly one `reload:` line;
  - "An atomic save is seen": write a temporary file and rename it.

## 6. Edit scripts and watch fixtures (D16)

- [ ] 6.1 In `watch.rs`, add `EditScript::parse(path) -> Result<EditScript, String>`. Steps are `after line <n>` or `after exit`, then `: edit <target> <replacement>`, with consecutive steps of the same trigger grouped. Bad lines are a usage error naming the line. Add `SourceOverlay`, which sets the `SourceStore` overlay. Verify with unit tests of parsing, grouping and errors.
- [ ] 6.2 Drive the session from the script. Install the line hook (task 1.4) so that at line `n` the program thread sends `LineWritten(n)` and waits on a reply channel. The session applies the step's overlay, runs `reload`, and replies with the prepared reload, which the program thread applies in the hook; a reload with errors gets a bare reply. Apply `after exit` steps when no program runs. End as "The End of an Edit Script" states, including the "never reached" message. Verify with fixtures for "Edits at an output line", "Two files edited in one reload" and "A step that is never reached".
- [ ] 6.3 In `crates/prism-cli/tests/fixture_tests.rs`, add the header `// watch: <script>`, which implies a run case. The runner spawns `prism run --check-allocations --watch --edits <script> <entry>` in the case directory and compares the status, `stdout.snap` and `stderr.snap` as for `// run:`. Document it in the header comment. Make sure `prism check` of a case and directory scans ignore `*.prism.v<k>` files, which they already do since only `.prism` is read; add a unit test. Every case of the `hot-reload` spec that is not marked as a Rust or real-file test must be a watch case.
- [ ] 6.4 Add the watch fixtures for the remaining scenarios:
  - "A running loop reaches new code at its next call"
  - "A running main keeps its own body"
  - "A function value made before a reload runs the new body"
  - "A changed field type keeps old values with old code"
  - "A changed signature keeps old callers on the old function"
  - "A closure whose captures changed keeps its old code for old values"
  - "A string literal of old code outlives its reload"
  - "An unchanged value keeps its value"
  - "An edited initializer runs again at the next read"
  - "A replaced string value is released"
  - "A removed function still serves the running main"
  - "A program that does not compile at start waits for a change"

  Read each `stdout.snap` and `stderr.snap` against its scenario's THEN before blessing it.

## 7. Rust tests, benchmarks and compile-time checks (D4, D6, D17)

- [ ] 7.1 In `crates/prism-codegen`, add the Rust test for "A deferred reload is applied at the reload point". A test-only runtime function the program calls between iterations calls `reload::reload_point()`. Set `defer_to_reload_points` and check the returned `AppliedReload` and the output.
- [ ] 7.2 Add `crates/prism-cli/tests/reload_timing.rs`, an `#[ignore]` test. It generates a 100-file program (20 functions per file, imports between packages, a `main` looping over calls into every package) in a temporary workspace, and runs `prism run --watch --timings`. It rewrites one file's body 20 times, alternating two versions, and measures from each write to the matching `reload: applied` line. It prints the median and 90th percentile of the total and per stage, and fails on the 50 ms target only under `PRISM_ASSERT_TIMING=1`. Record the measured numbers in the design's D17 when implementing.
- [ ] 7.3 Add to `crates/prism-cli/tests/compile_timing.rs` the shape "contract fingerprints": files with thousands of functions whose parameters are nested structs and enums, compiled through `LiveProgram::prepare`. Run `PRISM_ASSERT_TIMING=1 cargo test --release -p prism-cli --test compile_timing -- --ignored --nocapture` and check that growth stays linear.

## 8. Documentation and checks

- [ ] 8.1 Update `ROADMAP.md`:
  - item 25 records the shape this change chose;
  - the Phase 3 item "Hot-reload runtime prototype" points at this change;
  - the `codegen-aggregates` follow-up on live values of a changed layout is answered by D7.

  At archive time, add the follow-ups of the proposal's non-goals and the design's open questions: a restart command, per-function reload reports, reclaiming modules, incremental Phase 1, and parallel reload compilation.
- [ ] 8.2 In `docs/architecture.md` §2, update the "Implemented today" paragraph, and move the scenario "Tier 1 hot reload with disabled cross-file inlining" out (it is implemented by this change's `code-generation` and `hot-reload` requirements) when archiving.
- [ ] 8.3 Run every check of `openspec/config.yaml`'s apply guidance:
  - `cargo fmt --all -- --check`;
  - `cargo clippy --workspace --all-targets`, with no new warnings;
  - `cargo test --workspace`;
  - `prism check`, `prism fmt --check` and `prism run --check-allocations` on `examples/`;
  - the compile-timing run of task 7.3.

  Also run `prism run --watch examples/run/main.prism` by hand, edit a body in `examples/run/math/numbers.prism`, and check that the change is picked up (then revert the edit).
