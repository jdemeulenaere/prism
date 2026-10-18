# Design: hot-reload

## Context

See `proposal.md` for motivation. This is item 25 of "Then: the road to Milestone 2" in `ROADMAP.md`, the Phase 3 item "Hot-reload runtime prototype", and the "Tier 1 hot reload with disabled cross-file inlining" scenario of `docs/architecture.md` §2. Earlier changes shaped Tier 1 for it and left it questions:

- `codegen-core` D3 and D4: one code unit per file, every call through a slot, and a new JIT module per reload. D24 asks what happens to the values of top-level `val`s.
- `codegen-aggregates` D10 and D21: slot signatures survive a layout change. What a live value of a changed type becomes is this change's question.
- `runtime-strings-and-drops` D3: literals are static data of their unit, so old modules must stay mapped.
- `monomorphization` D3 and D4: instances are units with their own slots, and their symbols are stable across edits.
- `closures` D3 and D4: a descriptor holds the address of its closure's slot, never a code address.
- The compose plan, `docs/architecture.md` §5 D23: closures whose captures changed need a new slot, and reloads of composables are applied between frames.

The user was not available, so every decision a reviewer might take differently is marked **(autonomous)** and logged in `docs/decisions-for-review.md`. Fast compilation is a primary criterion (user, 2026-10-19), so each decision states its compile-time cost.

**Baseline.** This change was proposed while `monomorphization` was being implemented, `instance-cache` was being proposed, and `closures`, `escaping-closures` and `collections` were proposed but not implemented. All five land before it. When it is applied, these exist:

- instance units;
- `instance-cache`'s build plan: every file unit and instance unit of a program with its code key, comparable between two edits (its D12);
- relocatable unit artifacts that are defined through one path whether fresh or cached;
- check keys over the direct imports' ABI hashes, the interface hash being merged into the ABI hash;
- function-value descriptors with slot addresses, closure code with its own slot, and owned environments with drop entries in descriptors;
- lists.

This design names those pieces by the terms of `instance-cache`'s proposal. Where its design settles different names, the tasks follow it. The two MODIFIED requirements of `code-generation` and the two of `execution` are not modified by any of the pending packets, so they are written against today's main specs. Compare them again at apply time.

**Baseline update (2026-10-18, `instance-cache` implemented).** The names `instance-cache` landed with, which the tasks of this packet should use:

- The build plan is `crates/prism-cli/src/plan.rs::BuildPlan { units: Vec<PlannedUnit>, instances: Vec<PlannedInstanceKey> }`; `PlannedUnit` holds the path, module, check key and code key, `PlannedInstanceKey` the symbol, display and code key. `CompiledProgram::plan` carries it when a cache keyed the units. `BuildPlan::changed_since(&previous) -> (Vec<&PlannedUnit>, Vec<&PlannedInstanceKey>)` already exists, in plan order (units in check order, instances in symbol order), so task 2.9 changes its return type to `UnitId`s rather than adding it.
- `compile_program` is now `run.rs::compile_program_with(entry, &RunOptions)` (`compile_program(entry)` is it without a cache). It reads and writes the cache by default, so the driver paragraph above ("It never uses the build cache") no longer holds. Its per-file loop already distinguishes `checked`, `compiled` (verdict reused, code generated) and `reused` (artifact loaded), plans instances with `prism_codegen::plan_instances` over `prism_mir::FileSummary`s, and builds missed instances with `build_instance_unit` over `plan::program_types`; task 3.3's split should start from these stages.
- Summaries are `prism_mir::FileSummary` (`entry_point`, `roots`, `generics: Vec<GenericSummary>` with `record_hash`, `calls` and per-default `defaults`); check entries hold them (`BuildCache::read_check`/`write_check`), and records are entries of their own (`read_record`/`write_record`).
- Artifacts are `prism_codegen::artifact::UnitArtifact` (declarations, `CompiledFunction`s, `DataObject`s, symbolic `Relocation`s), produced by `Linker::compile_unit`/`compile_instance` and defined by `Linker::define_artifact`; task 2.3's contracts and value records go into it and its encoding. Code keys are `BuildCache::unit_key(check_key, target)` and `instance_key(symbol, record_hash, definitions_hash, target)`, with `prism_codegen::isa::target_identity()` and `plan::DefinitionsHasher`; they need a `BuildCache` (for the compiler identity), so task 3.2's factoring is still needed for `--no-cache` sessions.
- The Monomorphization Hash covers whole spans, so a body edit of a generic regenerates the instances of the generics declared after it in its file too (D2 note of `instance-cache`); a reload's instance list follows from the code keys either way.

**Reconciliation at apply time (2026-10-18).** `openspec validate hot-reload --strict` failed on the `execution` delta: its MODIFIED "Program Arguments and Options" dropped the sentence on `--no-cache` and `--explain-cache` and the scenario "Cache options before the file", both added by `instance-cache`. Both are restored, with one sentence saying what the two options mean under `--watch` (the session reads and writes the cache unless `--no-cache`, and `--explain-cache` explains its first load). The other three MODIFIED blocks ("Calls Go Through the Function Table", "Top-Level Values Have Storage and an Accessor", "Runtime Errors End the Program") match the main specs apart from their intended changes, and `closures`, `escaping-closures`, `collections` and `monomorphization` changed none of them. The functions the tasks name were checked against the landed code: `BuildPlan::changed_since` exists already (task 2.9 changes it), `compile_program` is `compile_program_with` (task 3.3 splits it), artifacts are `UnitArtifact` (task 2.3 extends it), and code keys need a compiler identity (task 3.2). Where the tasks' signatures turned out awkward against that code, the change is noted at the task and under "Implementation notes" below.

What exists today:

- **Function table** (`crates/prism-codegen/src/link.rs`). `FunctionTable` has chunks of `AtomicUsize` slots that never move, and maps a symbol to one slot (`slot`, `get`, `set`). The JIT's `symbol_lookup_fn` resolves every `<symbol>#slot` to the table's slot, so a unit's relocations hold slot *addresses*, fixed at finalization. `Linker::new` creates the table and one `JITModule`. `Linker::link` finalizes and writes each defined function's address into its slot. `Program` keeps the table and a `Vec<JITModule>` whose memory is never freed. `codegen-core` D26 made the table owned by the program, not global, "so `hot-reload` will keep the same table across loads".
- **Units** (`unit.rs`). A unit defines its own data: the file name, texts, and each top-level value's `#storage` and `#state` (D18, a state byte 0/1/2). The accessor `#get` initializes lazily through `#init` and registers the value with `prism_rt_register_top_level_value` so that the allocation check releases it at the end.
- **Stack checks** (`lower.rs::check_stack`, `glue.rs::check_stack`). Every compiled body, and every glue function, loads `prism_rt_stack_limit` and compares the stack pointer with it on entry. The cold path calls `prism_rt_panic` (no return) with "stack overflow". The limit is an `AtomicUsize` in `crates/prism-runtime/src/stack.rs`, zero outside a run.
- **Program thread** (`program.rs::Program::run`). The program runs on a spawned thread with an 8 MiB stack, bracketed by `prism_runtime::start` and `finish`. The caller joins it. A runtime error calls `panic.rs::report`, which flushes and calls `process::exit(101)`.
- **Driver** (`crates/prism-cli/src/run.rs::compile_program`). It builds the graph with `DependencyGraph::from_entry` (reading and parsing every file), runs `extract_abis` (Phase 1, whole program, from syntax trees), `elaborate_file` per file in topological order, `find_main`, `check_file`, then lowers and links. It never uses the build cache (`codegen-core` D20). `prism check` computes a check key per file (`BuildCache::check_key`: compiler, module path, source, direct imports' interface hashes), which decides when Phase 2 can be skipped.
- **Output** (`crates/prism-runtime/src/output.rs`). A process-wide buffer behind a `Mutex`, written by `append` and flushed at the end, at runtime errors and when it fills.

## Goals / Non-Goals

**Goals:**
- Editing a function body, a generic body, a closure body or a top-level value of a running program takes effect at the program's next call of it, with no restart and no loss of the program's state.
- Every reload is memory-safe, whatever changed: no code ever runs over a value laid out for different code, and no reload is refused because of what is live.
- A one-file body edit reaches patched code in under 50 ms (median, release build, 100-file program), because a reload does work proportional to what changed and not to the program.
- Deterministic tests: fixtures that show output before and after each patch, under the allocation check.
- The hooks `compose-hot-reload` needs: reload points for frame loops, the list of reloaded units, type fingerprints for slot-table state, and closure contracts.

**Non-Goals:**
- Compose: recomposing scopes, slot-table state, the frame loop's wiring (`compose-hot-reload`).
- On-stack replacement: a frame already running keeps running the code it entered.
- Migrating values between layouts. Re-running initializers eagerly. Reclaiming old modules during a run.
- Incremental Phase 1, and parallel compilation of a reload's units (follow-ups).
- Tier 2, which has no reload.

## Decisions

### D1. Plain programs now, compose next (autonomous)

This change reloads ordinary programs. `compose-hot-reload` (item 32) builds on it to recompose the scopes of reloaded files, reset slots whose type fingerprint changed, and apply reloads between frames. Splitting them follows the roadmap. It also makes the mechanism testable with `println` programs before any compose code runs, and it means compose adds a policy rather than a second mechanism. There is no language change: no new syntax, built-in or error code. Rejected: doing both in one change. Compose needs five unbuilt changes first, so the mechanism would wait for them.

### D2. One session process; the program on its own thread (autonomous)

`prism run --watch` runs a **watch session** in the `prism` process. The session's thread, the CLI's main thread, watches files, runs the front end and code generation for reloads, and prints status. The program runs on the program thread exactly as `Program::run` runs it today: its own 8 MiB stack, `start`, `main`, `finish`. They talk through channels. The program thread sends `LineWritten`, `MainReturned(status)` and `Stopped`. The session hands it pending reloads (D3).

Rejected:
- **Running the program in a child process.** The JIT's code, the function table and the program's memory would then live in another process, so patching would need shared executable memory and an IPC protocol, for no gain over a thread. Isolation from crashes is handled by D13.
- **Restarting the program on every change**, as `cargo watch` or `node --watch` do. That keeps no state, and keeping state is the point. It remains the behaviour after `main` returns (D13).

`desktop-window` will need the process's main thread for `winit`, so the session then moves to a thread of its own. Nothing here depends on which thread it is.

Compile-time cost: none beyond the reload itself (D9).

### D3. Reloads are applied by the program thread at its next call, through the stack-limit check (autonomous)

The session compiles and finalizes a reload's module on its own thread while the program keeps running. The result is a `PreparedReload`: a list of slot writes (slot address, new code address), the value-table updates (D8), and a message. The session publishes it in `prism_runtime::reload` and stores the **interrupt sentinel** `usize::MAX` into `STACK_LIMIT`.

Every compiled body already compares the stack pointer with `STACK_LIMIT` on entry, so the program thread's next such entry takes the cold path. The cold path now calls `prism_rt_stack_check_failed() -> u8` before panicking:

1. The function restores the real limit (kept beside the sentinel).
2. It takes the pending reload, if any, and applies its slot writes with release stores. On aarch64 it then executes an `isb`.
3. It prints the reload's message after flushing the program's output, and returns 0.
4. If the limit was real and the stack really is below it, it returns 1, and the generated code raises "stack overflow" as today.

The limit is restored before the reload is taken. A request that arrives in between is either taken now, leaving a harmless spurious interrupt later, or arrives after the restore and sets the sentinel again, so no request is lost. Glue's stack check uses the same entry point, so glue never reports a false overflow.

Why at a call, on the program thread:
- **Atomic as seen by the program.** All of a reload's slots change between two instructions of the program thread, so it never runs new `a` with old `b` from the same reload.
- **No data race.** Generated code loads slots with plain loads, and only the thread that loads them stores them. Code bytes and slot writes are published to it through the reload's mutex hand-off.
- **Zero cost on the hot path.** The check exists already. The cold path gains one call, so this is Go's preemption trick (it sets `stackguard0` to `stackPreempt`) applied to reloads. Every compiled body checks, leaves included, so a loop that calls only a leaf function reaches the reload at that call. Generated accessors of top-level values do not check, which is harmless: they call the initializer, which does.
- **Prompt.** Any program that makes calls reaches a reload within one call. A loop with no calls is not patched until it calls something. It could not observe the patch before that anyway.

Rejected:
- **Writing the slots from the session thread at once.** The program could run a mix of old and new code from one reload. Plain loads racing atomic stores is a data race in Rust's model, even if harmless on today's CPUs.
- **Suspending the program thread with a signal or `thread_suspend`.** Platform-specific, and it can stop the program inside the allocator or the output lock.
- **A `hotReloadPoint()` built-in that programs call.** Every program would need editing to be reloadable. Frame loops get a runtime-level reload point instead (D4).
- **Polling a reload flag in every function prologue.** An extra load and branch on every call, which the stack check already provides.

While waiting for the reload, the program thread is blocked at most for the slot writes: microseconds, linear in the reloaded slots. Compile-time cost: one more call in each cold block, no new pass.

### D4. A reload policy for frame loops (autonomous, recorded for `compose-hot-reload`)

`prism_runtime::reload` has two policies. **At the next call** (D3) is the default. **At reload points**: a frame loop calls `reload::defer_to_reload_points()`. The session then publishes a reload without setting the sentinel and calls the loop's registered wake-up function. The loop applies the reload when it next calls `reload::reload_point()`, between frames. `reload_point` returns an `AppliedReload` listing the modules of the reloaded file units and instance units, which `compose-hot-reload` uses to recompose their restart scopes (§5 D23). This change implements both policies and tests the second from Rust only, since no Prism program has a frame loop yet. Rejected: leaving the policy entirely to `compose-hot-reload`. It would have to change the runtime interface this change defines.

### D5. A slot is repointed only to code with the same contract (autonomous)

Each slot records the **contract** of the code it holds. For a function, extension, default generator, initializer or accessor, the contract is the type fingerprints (D6) of its parameter and result types, in order, as the MIR body declares them. For closure code it also includes the environment's fingerprint: the closure's captures in order, each with its mode (shared, mutable or owned) and type, and, after `escaping-closures`, whether the environment is inline or on the heap. The machine signature follows from these, so it needs no hash of its own.

When a reload defines a symbol:

- **No slot yet:** the symbol gets a new slot (an added declaration).
- **Same contract as the slot's code:** the slot is repointed. Every later call reaches the new code: from new code, from frames of old code, and from function values made before the reload, since their descriptors hold the slot's address.
- **Different contract:** the symbol gets a **new slot**, and the table maps the symbol to it for every later load. Relocations hold slot addresses fixed at finalization, so code finalized before the reload keeps the old slot, and the old slot keeps the old code. Running frames of old code, and old function values, call the old function with the arguments it was compiled for. New code calls the new one.

A different contract is safe because nothing left unrecompiled can call the symbol with the new contract. A public signature, the types it reaches and a default value's signature are in the ABI hash, so every direct importer is checked and compiled again in the same reload. A private one is called only from its own file. An instance's code key covers its generic's record, whose calls record their callees' signatures, and its type arguments' definitions (D9). A closure slot is named only by its own unit's descriptor. A debug assertion checks this: every unit that refers to a symbol whose slot was split is in the reload.

When the program is running, the reload's message notes each split: `reload: the signature of main::step changed; code already running keeps calling its previous version`.

Rejected:
- **Refusing reloads that change a signature or a layout**, as JVM HotSwap does. Adding a parameter or a field is among the most common edits.
- **Patching every caller, frames included.** A running frame's code cannot be swapped without on-stack replacement.
- **Comparing only the machine signature.** A struct parameter is one pointer whatever its fields, so a field changed from `Int` to `Double` would let new code read old values as the wrong type. This is the case `codegen-aggregates` D21 warned about.
- **Versioned symbol names in code** (`<symbol>#slot@2`). The table's symbol-to-slot map already versions slots without changing any unit or dump.

A closure's slot is keyed by its index within its parent (`closures` D4). Inserting a closure before another therefore shifts which body an old value reaches. When the contracts match, this is memory-safe, but such a value may run the code now at its old position. Compose's group keys have the same positional nature (§5 D5). Keying closures by a hash of their body was rejected, because then a body edit would never reach handlers stored before it, which §5 D22 needs.

Compile-time cost: one contract per compiled body, a hash over its signature's fingerprints. Linear in signatures, since fingerprints are memoized (D6).

### D6. Type fingerprints (autonomous)

A **type fingerprint** is a 256-bit BLAKE3 hash, computed by `contract.rs::TypeFingerprints`, of a type's structure:

- scalars, `String` and `Range` by tag;
- references, nullables, boxes and lists by tag and the fingerprint of their contents;
- function types by their parameter and result fingerprints and the compose flag;
- structs and enums by qualified name, markers, and their fields' or variants' names and fingerprints, in declaration order, with type arguments substituted.

A type that reaches itself through a box or list is hashed with a back-reference to the definition on the stack, so cycles terminate. Fingerprints are memoized per load, by type, over the unit's type table. Two types with the same fingerprint have the same layout and the same drop, clone and equality behaviour. That is the property contracts (D5), value keys (D8) and instance keys (D9) need. It is also what compose's slot type fingerprints need, so `compose-hot-reload` reuses it.

Rejected:
- **Hashing the computed layout (sizes and offsets).** `Int` and `Double` fields share a layout but not a meaning.
- **A program-wide type numbering.** Not stable across reloads.

Compile-time cost: linear in the definitions a load's types reach, each hashed once per load. A shape in `compile_timing.rs` (thousands of functions over deep struct types) shows that it stays linear.

### D7. Live values are never migrated (autonomous; answers `codegen-aggregates` D21)

D5 keeps every value with code compiled for its layout:

- values in frames of old code are handled by old code and passed only to slots of the old contract;
- values reachable from them, on the heap or behind references, are dropped by old glue, which old modules keep;
- top-level values of a changed type get new storage (D8);
- slot-table state of a changed type is reset by `compose-hot-reload` (§5 D23).

So a reload is never refused and never converts a value. The cost is that old code may keep running for a while (a `main` loop calling an old `step`), which the split note (D5) tells the user. Rejected:
- **Migrating values field by field**, as Dart does for instances. It needs a runtime layout description and default values for added fields, and a migrated value would surprise code that held a borrow of it.
- **Refusing the reload with an offer to restart.** It forces a restart for the most common edits, and the old code's behaviour is safe and well defined.

### D8. Top-level values belong to the program, keyed by symbol, type and initializer (autonomous; answers `codegen-core` D24)

A top-level value's `#storage` and `#state` move out of its unit into a **value table** owned by the program, beside the function table. A unit imports them as data symbols, which the linker resolves the way it resolves slots. Each unit lists its values with their size, alignment and **value key**. The key hashes three things: the value's symbol, its type fingerprint (D6), and its initializer hash. The initializer hash is a span-free hash of the declared type and the initializer expression (`prism_abi::initializer_hash`, using the AST hasher behind `generic_body_hash`).

When a reload defines a value:

- **Same key as the storage the symbol has:** the new unit resolves to that storage and its state byte. An initialized value keeps its value, and the new accessor, repointed into the accessor's slot (same contract), returns it.
- **Different key:** new zeroed storage, so the value is initialized at its next read by the new initializer. The accessor's slot is repointed when its contract (the type) is the same, and split otherwise (D5). The old storage stays allocated, readable by old code and by any borrow of it, and registered for release by the allocation check.

So editing a value's initializer resets that value, and editing anything else, including a function its initializer calls, keeps it. Moving the declaration or reformatting it keeps it too, since the hash ignores positions.

Rejected:
- **Always keeping values, as Dart and the JVM do.** An edited initializer then has no visible effect until a restart, which Flutter's documentation has to warn about.
- **Resetting every value of a changed file.** An edit to one function of a configuration file would lose all of the file's state.
- **Re-running a changed initializer eagerly when the reload is applied.** It runs user code, with its output and runtime errors, in the middle of whatever call the program was making (D3).
- **Keeping storage in units and importing old storage selectively.** Two paths for the same data object.

Compile-time cost: one AST hash per top-level value of a re-checked file, linear in its initializer.

### D9. A reload compiles the units whose inputs changed (autonomous)

A reload, `WatchSession::reload`, does the following:

1. **Sources.** It reads the changed paths through the overlay (D16) and compares content hashes. Unchanged content ends the reload with nothing to do.
2. **Graph.** It rebuilds the dependency graph with `DependencyGraph::from_entry_with`, which takes each file's source and syntax tree from a `SourceStore` and parses only files whose content changed. Imports may add or drop files.
3. **Phase 1.** It runs `extract_abis` over the program's syntax trees. This is linear in the program's declarations (headers, not bodies) and needs no reading or parsing of unchanged files.
4. **Phase 2.** It computes each file's check key with the same function `prism check` uses. It runs `elaborate_file` only for files whose key differs from the one their kept MIR was built for. A body edit changes one key. A signature or reachable-type edit also changes its importers' keys, through the ABI hash. A generic body edit changes no importer's key, because `instance-cache` takes body hashes out of the ABI.
5. **Backend checks.** It runs `find_main` and `check_file` on the re-elaborated files.
6. **Plan.** It builds the program's build plan (`instance-cache`) from the files' summaries, the kept ones and the new ones, and compares it with the plan of the code that is running. Units whose code key is new or changed are compiled, and every other unit is left alone. For a file unit, the code key is its check key plus the target. For an instance, it is its symbol, its generic's Monomorphization Hash (the record hash) and its type arguments' definitions, plus the target. An instance is therefore compiled again exactly when its generic's record or a type argument changed, not on an unrelated edit in its file.
7. **Code generation.** It takes each changed unit's artifact from the build cache when it holds the code key, and otherwise lowers and compiles it, with its closures, and writes it to the cache. It defines the artifacts into a new `JITModule` through `Linker::with_table`, and finalizes it. This yields the `PreparedReload` (D3).

Units that did not change are neither lowered nor compiled. Their code stays where it is, and their slots are untouched.

The session keeps in memory, per file, its content hash, its `SourceNode`, its check key, its summary and its elaborated `MirFile`. It also keeps the running plan and the contracts and value keys of the code each slot holds. The build cache is used as `prism run` uses it (D14).

Compile-time cost per reload:
- reading and parsing: linear in the changed files;
- graph and Phase 1: linear in the program's declarations;
- Phase 2 and code generation: linear in the changed files and their re-checked importers;
- planning: linear in the program's instances, from summaries;
- linking: one finalization, linear in the reload's code.

The program-wide steps (graph, Phase 1, planning) work on data in memory, and the benchmark (D17) checks that they fit the budget. Incremental Phase 1 is a follow-up if they stop fitting.

Rejected:
- **Recompiling the whole program on every change and diffing slots.** Simple, but linear in the program at every save, against the latency target.
- **Per-function granularity (compiling only changed bodies of a file).** A unit is the unit of hermetic compilation (the deterministic hermetic builds invariant), and one file compiles in about a millisecond.

### D10. Old modules stay mapped for the life of the program (autonomous)

Each reload's `JITModule` is kept in `LiveProgram::modules` until the program ends, that is, until the process exits or `main` runs again in a fresh program (D13). They must outlive the program's frames, descriptors, drop entries, slots that still hold old code (D5, D11), and the static literal data that strings in kept top-level values point at (`runtime-strings-and-drops` D3). Proving that none of these refer to a module needs a scan of stacks and heaps, which amounts to a garbage collector, against the zero-cost memory safety invariant.

The cost is memory: a module holds only the reloaded units' code and data, tens of kilobytes for a typical file. A thousand reloads in one run stay well under 100 MB. `--timings` reports the number of live modules, so growth is visible. Rejected: reclaiming unreferenced modules (above); reusing old modules' memory for new code (the same proof is needed).

### D11. Removed declarations keep their code (autonomous)

When a declaration disappears from its file, nothing defines its symbol in the reload, and its slot keeps the code it held. New code cannot name it, because the type checker rejects a call of it. Old frames and old function values that still call it keep working. A declaration that comes back with the same contract gets its slot back, repointed. Rejected: pointing removed slots at the unlinked stub, which would turn a harmless edit into a runtime error in old code that is still running.

### D12. A reload with errors keeps the previous code (autonomous)

When any stage of a reload reports errors, the session prints them as `prism run` does, followed by `reload: not applied; the program keeps running its previous code`. The stages are resolution, parsing, Phase 1, type and borrow checking, `E6001` from the backend's checks, and `E6002`/`E6003` for the entry's `main`. The session then waits for the next change. The session's kept state (MIR, keys, contracts) is left as the last applied reload left it, so the next reload is computed against the code that is actually running.

If a prepared reload has not been applied yet (the program is in a loop without calls), a newer change supersedes it. The session takes the pending reload back if the program has not applied it, drops its module (nothing refers to it, since it never ran), and prepares a new one from the latest sources against the last applied state. The same rule holds at start: a program that does not compile prints its diagnostics and `watch: waiting for changes` instead of exiting with status 1.

Rejected: exiting the session on errors (an edit in progress is normally incomplete); applying the files of a reload that compile and skipping the others (a file's importers were checked against its new interface, so a partial reload could link inconsistent units).

### D13. After `main` returns, a change runs it again; a runtime error stops only the program (autonomous)

When `main` returns, the program thread runs `finish` as today (the allocation check under `--check-allocations`). The session prints `watch: main returned with status <status>; waiting for changes`. At the next change that compiles, it builds a **fresh program**: a new function table, value table and modules. Every unit is defined from its cached artifact, and only units whose code key changed are compiled, as for a warm `prism run`. It prints `watch: running main again` and runs it. Old modules are freed with the old program (D10).

A fresh program means a re-run behaves exactly like a new `prism run`: top-level values are initialized again, and the allocation check covers one run. Rejected: re-running `main` in the patched program, which would keep top-level values across runs and make each run's allocation check depend on earlier runs.

A runtime error under `--watch` reports as today, flushing output and printing `panic: ...` and its location. Then, instead of `process::exit`, `panic.rs::report` signals the session and parks the program thread forever. No drop runs and no further code of the program runs, as the `execution` spec requires. The session prints `watch: the program stopped with a runtime error; waiting for changes`, and the next change runs `main` again in a fresh program. A parked thread holds no runtime lock (`report` releases the output lock after flushing) and keeps its stack, 8 MiB of reserved address space per stopped run. Rejected: exiting the session (one bad edit would end it); unwinding the program thread (no unwinding exists, `codegen-core` D12).

Under `--watch`, the session itself ends only on Ctrl-C, or when an edit script ends (D16).

### D14. The session keeps the front end in memory and shares the build cache (autonomous)

The session decides what to re-check with the build cache's check key and what to compile with the build plan's code keys (`instance-cache`). It keeps every file's MIR in memory, so a reload never reads a record or a verdict from disk for a file it already holds. It uses the on-disk cache the way `prism run` does after `instance-cache`: the first load and every fresh program (D13) read cached artifacts, and every unit a reload compiles is written back. A reload's code is then reused by the next `prism run`, and by the session's next fresh program, without Cranelift. `--no-cache` turns this off as for `prism run`. Rejected:
- **Keeping the session out of the cache,** as `prism run` was before `instance-cache` (`codegen-core` D20). The first load and every re-run would compile the whole program.
- **Reading records and MIR from the cache on each reload instead of keeping them in memory.** That costs disk reads and decoding at every save, for memory that is linear in the program.

### D15. File watching with `notify`, a 10 ms debounce and content hashes (autonomous)

`watch.rs::FileWatcher` uses the `notify` crate (version 8: FSEvents on macOS, inotify on Linux, ReadDirectoryChangesW on Windows, MIT or Apache-2.0) in non-recursive mode on each directory that holds a program file or the target of an unresolved import. The directories are registered again after each reload, since the graph may change. Watching directories rather than files catches editors that save by writing a temporary file and renaming it over the original. Events for paths outside the watched set are ignored.

After the first relevant event the watcher waits for 10 ms without events, bounded at 100 ms after the first event, then hands the set of paths to the session. The session compares their content hashes (D9), so a `touch` or a save without changes does nothing. If the native watcher cannot be created, the session falls back to `notify`'s `PollWatcher` at 100 ms and says so on stderr.

Rejected:
- **Polling as the primary mechanism:** 100 ms of latency on average and constant CPU use.
- **`notify-debouncer-full`:** an extra dependency for a 30-line debounce.
- **A recursive watch of the workspace root:** it would see build outputs and unrelated directories.

### D16. Scripted edits make reloads deterministic (autonomous)

Real file events and the next-call rule (D3) make the moment a reload lands depend on timing, so fixtures cannot use them. `prism run --watch --edits <script>` replaces the watcher with a script of steps, one per line, with `#` comments:

```text
after line 2: edit lib/math.prism lib/math.prism.v2
after line 2: edit app/main.prism app/main.prism.v2
after exit: edit app/main.prism app/main.prism.v3
```

`edit <target> <replacement>` makes the session read `<target>` as the content of `<replacement>`. Both paths are relative to the script's directory. This is a **virtual overlay** (`SourceOverlay`): no file on disk changes, so fixtures stay read-only, and a target that does not exist yet becomes a new file. Consecutive steps with the same trigger form one reload.

- `after line <n>`: the output layer counts the newlines the program writes in the current run. When the program writes the `n`-th one, it flushes, signals the session and blocks inside the runtime's output function. The session applies the edits and runs the reload. The program thread applies the result right there, since a runtime call is a safe point like a function entry, and continues. If the reload has errors, the program continues with its previous code.
- `after exit`: applied after `main` returns. The edit and its reload run `main` again in a fresh program (D13).

The session ends when the script has no steps left and no program is running. Its exit status is that of the last run, or 1 when the last reload did not compile. A step whose line is never written ends the session with `watch: the edit script's step "after line <n>" was never reached` and status 1.

Executable fixtures gain a `// watch: <script>` header line. The runner then spawns `prism run --check-allocations --watch --edits <script> <entry>` and compares stdout, stderr and the status as for `// run:`. Replacement files end in `.prism.v<k>`, which `prism check` and directory scans ignore. The real watcher and the next-call rule are tested apart from fixtures: a `run_cli_tests.rs` test edits a file on disk under a program that loops calling a function until the reload changes its result, with a generous timeout, and a `prism-codegen` unit test applies a reload into a running program through the sentinel.

Rejected:
- **Writing files and sleeping in tests:** flaky on loaded CI machines.
- **A Rust-only test API:** it would leave the CLI's session untested by fixtures.
- **A test-only Prism built-in that triggers reloads:** a language change made only for tests.
- **Copying fixtures to a temporary directory and editing them on disk:** slower, and unnecessary with an overlay.

The `edit` step is the one compose's `--events` script will share (`compose-hot-reload`, §5 "Headless Rendering and Scripted Input"), with the same overlay.

Compile-time cost: none in the compiler; a newline counter in the runtime's output path when a script is active.

### D17. Latency target: 50 ms from save to patched code, reported (autonomous)

The target is a median under 50 ms on a release build for the following case: one body line of one file in a generated program of 100 files, with 20 functions each and imports between them, saved on disk while the program runs. It is measured from the write to the session's `reload: applied` line, debounce included.

The ignored test `crates/prism-cli/tests/reload_timing.rs` (`cargo test --release -p prism-cli --test reload_timing -- --ignored --nocapture`) does the following:
- generates the program;
- runs `prism run --watch --timings` on a `main` that loops calling into it;
- rewrites one file 20 times, alternating two bodies, waiting for each `reload: applied`;
- prints the median and 90th percentile of the whole reload and of each stage.

It fails on time only with `PRISM_ASSERT_TIMING=1`, as `run_timing.rs` does (`codegen-core` D22). With `--watch`, `--timings` prints `timing: reload <ms> (detect <ms>, front end <ms>, code generation <ms>, linking <ms>, applied after <ms>)` and the number of live modules after each reload.

The budget is: debounce 10 ms, front end for one file plus Phase 1 over 100 files under 10 ms, one unit's code generation and linking under 5 ms, application under 1 ms. That leaves room for the platform's file-event latency. Rejected: asserting the time in `cargo test` (debug builds and shared runners); a target per stage only (the user feels the whole reload).

### D18. A new `hot-reload` capability, two modified ones, no new code (autonomous)

The session's behaviour, which later tiers and compose extend, gets its own capability, `hot-reload`. `code-generation` changes where slots and value storage live and what a stack check does. `execution` lists the new options and says what a runtime error ends under `--watch`. Reload failures are compile errors with their existing codes, so no code is added to the diagnostics catalog. Rejected: putting the session into `execution`, which describes what one run of a program does.

## Implementation Notes

Where the implementation departs from the tasks' wording, or settles what they left open:

- **The cold path takes the stack pointer** (task 1.1). `prism_rt_stack_check_failed(stack_pointer)` receives the stack pointer the generated check compared, so the runtime decides "really below the real limit" exactly rather than from an address of its own frame. One more argument on a cold path; the fast path is unchanged.
- **Hooks are closures** (tasks 1.3 and 1.4). `set_watch_mode` and `set_line_hook` take `Arc<dyn Fn(..) + Send + Sync>` rather than `fn` pointers, so the session hands them its channel without a global. The stop handler receives the exit status the process would have had, so the session's exit status after a runtime error is still 101. A failed allocation check (an invalid free, status 102) stops only the program too, through the same path.
- **A reload is applied under its lock.** `apply_pending` runs the reload while holding the pending reload's mutex, so `withdraw` returning nothing means that the last request has been applied completely. The session uses that to decide whether the state of its last reload is the running one (D12) without a notification from the program thread.
- **An interrupt raised before the program's limit is set is raised again.** `set_stack_limit` re-interrupts when a reload is pending, so a request that races with the start of a run is not lost.
- **Every compiled body checks the stack** (D3). The design said "every function that calls another"; in fact `lower.rs` checks on entry to every compiled body, leaves included, so a loop calling only a leaf reaches a reload at that call. The specs now say so.
- **A split slot is mapped when the reload is applied** (task 2.6). `FunctionTable::slot_for` allocates the fresh slot of a split without mapping the symbol to it; the load resolves `<symbol>#slot` to it through a map of its own, consulted before the table, and the mapping and the slot's contract are committed by the reload's writes on the program thread (`ReloadWrites::apply`). The value table works the same way. So a reload that is withdrawn (D12) leaves both tables exactly as they were, and the next one is computed against the running code.
- **Preparing goes through the load's linker** (task 2.7). Instead of `LiveProgram::prepare(units, instances, keys)`, a reload takes `LiveProgram::linker()` (`Linker::with_table`), compiles or loads each changed unit's artifact into it as the first load does, and calls `Linker::prepare()`. `LiveProgram::hold` keeps the reload's module and runs the split-safety check; `LiveProgram::settle(applied)` keeps the module or frees it. Fresh and cached units are defined through one path, the one `instance-cache` built.
- **`Program::start` takes a callback** (task 2.8): `on_return(status)` runs on the program thread once `main` returned and the run finished. The stopped case is reported by the watch-mode stop handler, since a parked thread never returns.
- **Unit identities** (task 2.9). `changed_since` returns `UnitId::File(module)` and `UnitId::Instance(symbol)`, which are also the units' names in the linker.
- **The reload tests run in a process of their own.** Running a program and requesting a reload change the runtime's process-wide stack limit, which other `prism-codegen` unit tests calling compiled code directly would see, so they are in `crates/prism-codegen/tests/reload_tests.rs`.
- **Keys without a cache** (task 3.2). The key functions moved from `BuildCache` into `CacheKeys`, which a cache holds for its compiler's identity and a session without a cache builds with `CacheKeys::in_process()`, an identity of the version alone: such keys are only compared within the process. `prism run` now always computes its build plan, with or without a cache.
- **One build path** (task 3.3). `compile_program_with` became `build_program`, which the first load, every reload and every fresh program run (D13) share: it takes the graph, the cache, the keys, what the session kept of each file, and the running plan, and returns the entry's `main`, the new plan, the artifacts of the units the running plan lacks (all of them for a first load), what to keep, and the initializer hashes of the values of the units it built. A file whose check key the session kept is not checked again; a needed unit's artifact comes from the cache, else from kept or new MIR. `link_build` defines and links a build's units for a first load.
- **Diagnostics show what was checked.** Type, borrow, `E6xxx` and ABI errors take their source from the graph rather than from the disk, so a reload of an overlaid file shows the overlay's text (D16).
- **Overlaid new files resolve.** `DirectoryListings::with_virtual_files` makes import resolution treat files that exist only in the overlay as present, so an edit script can add a file.

## Risks / Trade-offs

- **[Old code keeps running after a contract split]** A `main` loop calling `step` keeps the old `step` after its signature changes. → The split note (D5) says so. The program picks up the new version when `main` runs again. On-stack replacement is out of scope.
- **[Instruction-cache coherence on aarch64]** The session writes code that the program thread then executes. → `cranelift-jit`'s finalization invalidates the instruction cache and flushes the pipelines of other threads where the platform provides it. The program thread also executes an `isb` before using a reload (D3), and the new code is at addresses it has never executed.
- **[Platform file-event latency]** FSEvents may batch events for longer than the debounce. → The benchmark measures end to end. If the median misses the target on macOS, the stream's latency parameter is set to zero.
- **[Memory growth over long sessions]** Old modules are never freed (D10). → They are small, and `--timings` shows the count. A fresh program after `main` returns frees them.
- **[A closure index shift with equal contracts]** It sends an old function value to another closure's body (D5). → Memory-safe, and the next creation of the value uses the right code. The same positional matching as compose keys.
- **[Parked threads after runtime errors]** Each stopped run keeps 8 MiB of reserved stack. → Address space, not resident memory. Sessions that stop thousands of times are not a target.

## Migration Plan

No language or output change for `prism run` without `--watch`. Top-level storage moves from units to the program's value table in every run, which no program can observe. `prism dump-clif` shows `#storage` and `#state` as imported data instead of unit data, and no snapshot fixture holds clif text (`codegen-core` D23). Once this change is archived, `compose-hot-reload` uses D4's policy, D6's fingerprints and D5's closure contracts.

## Open Questions

- Should a watch session accept a restart command (a key on stdin) for long-running programs whose `main` loop holds old code? Stdin is the program's once input built-ins exist, so this waits for them (follow-up).
- Should a reload report which functions changed, rather than which files? It needs a per-body hash, which `instance-cache`'s record hashing may provide (follow-up).
